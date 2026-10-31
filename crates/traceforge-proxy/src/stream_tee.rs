use std::collections::BTreeMap;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::Stream;
use pin_project_lite::pin_project;
use tracing::{debug, trace};

use traceforge_core::{StreamChunk, TokenUsage, ToolCall};

/// Intermediate representation of a tool call accumulated across streaming chunks.
#[derive(Debug, Clone, Default)]
struct PartialToolCall {
    id: String,
    tool_type: String,
    name: String,
    arguments: String,
}

/// Final summary of an intercepted and accumulated SSE stream.
#[derive(Debug, Clone)]
pub struct StreamSummary {
    pub full_content: String,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub token_usage: TokenUsage,
    pub ttft: Option<Duration>,
    pub total_latency: Duration,
    pub proxy_overhead: Duration,
    pub chunk_count: usize,
    pub finish_reasons: Vec<String>,
}

/// In-memory zero-copy accumulator for SSE chunks.
#[derive(Debug)]
pub struct StreamAccumulator {
    start_time: Instant,
    first_token_time: Option<Instant>,
    accumulated_content: String,
    accumulated_reasoning: String,
    accumulated_tools: BTreeMap<usize, PartialToolCall>,
    residual_buffer: Vec<u8>,
    usage: Option<TokenUsage>,
    finish_reasons: Vec<String>,
    chunk_count: usize,
    overhead_accumulator: Duration,
    is_done: bool,
}

impl StreamAccumulator {
    pub fn new(start_time: Instant) -> Self {
        Self {
            start_time,
            first_token_time: None,
            accumulated_content: String::with_capacity(1024),
            accumulated_reasoning: String::with_capacity(512),
            accumulated_tools: BTreeMap::new(),
            residual_buffer: Vec::with_capacity(2048),
            usage: None,
            finish_reasons: Vec::new(),
            chunk_count: 0,
            overhead_accumulator: Duration::ZERO,
            is_done: false,
        }
    }

    /// Ingests a new incoming raw byte chunk from the upstream HTTP stream.
    /// Operates sub-millisecond to maintain strict < 5ms SLA bounds.
    pub fn process_bytes(&mut self, chunk: &[u8]) {
        let op_start = Instant::now();
        self.chunk_count += 1;

        // Append new bytes to any leftover bytes from previous frame
        self.residual_buffer.extend_from_slice(chunk);

        // Find and process all complete SSE messages separated by "\n\n" or "\r\n\r\n"
        while let Some((pos, delim_len)) = find_next_frame_delimiter(&self.residual_buffer) {
            let frame_str = match std::str::from_utf8(&self.residual_buffer[..pos]) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    self.residual_buffer.drain(..(pos + delim_len));
                    continue;
                }
            };
            self.residual_buffer.drain(..(pos + delim_len));
            self.process_sse_frame_str(&frame_str);
        }

        self.overhead_accumulator += op_start.elapsed();
    }

    fn process_sse_frame_str(&mut self, frame_str: &str) {
        for line in frame_str.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with(':') {
                continue; // SSE comment or ping
            }

            if let Some(data_str) = trimmed.strip_prefix("data:") {
                let payload = data_str.trim();
                if payload == "[DONE]" {
                    self.is_done = true;
                    continue;
                }

                if let Ok(chunk) = serde_json::from_str::<StreamChunk>(payload) {
                    self.ingest_chunk_model(chunk);
                } else {
                    trace!("Non-JSON SSE payload received: {}", payload);
                }
            }
        }
    }

    fn ingest_chunk_model(&mut self, chunk: StreamChunk) {
        if let Some(usage) = chunk.usage {
            self.usage = Some(usage);
        }

        for choice in chunk.choices {
            if let Some(ref fr) = choice.finish_reason {
                if !self.finish_reasons.contains(fr) {
                    self.finish_reasons.push(fr.clone());
                }
            }

            let delta = choice.delta;
            let mut has_token = false;

            if let Some(content) = delta.content {
                if !content.is_empty() {
                    has_token = true;
                    self.accumulated_content.push_str(&content);
                }
            }

            if let Some(reasoning) = delta.reasoning_content {
                if !reasoning.is_empty() {
                    has_token = true;
                    self.accumulated_reasoning.push_str(&reasoning);
                }
            }

            if let Some(tool_deltas) = delta.tool_calls {
                for td in tool_deltas {
                    has_token = true;
                    let partial = self.accumulated_tools.entry(td.index).or_default();

                    if let Some(id) = td.id {
                        partial.id = id;
                    }
                    if let Some(tt) = td.tool_type {
                        partial.tool_type = tt;
                    }
                    if let Some(func) = td.function {
                        if let Some(fn_name) = func.name {
                            partial.name.push_str(&fn_name);
                        }
                        if let Some(fn_args) = func.arguments {
                            partial.arguments.push_str(&fn_args);
                        }
                    }
                }
            }

            // Record Time To First Token on the very first content/tool delta
            if has_token && self.first_token_time.is_none() {
                self.first_token_time = Some(Instant::now());
                debug!(
                    "Stream TTFT recorded: {:?}",
                    self.first_token_time.map(|t| t.duration_since(self.start_time))
                );
            }
        }
    }

    /// Finalizes accumulated state into an immutable `StreamSummary`.
    pub fn finalize(mut self) -> StreamSummary {
        let op_start = Instant::now();
        let total_latency = self.start_time.elapsed();
        let ttft = self.first_token_time.map(|t| t.duration_since(self.start_time));

        // Process any leftover frame in residual_buffer
        if !self.residual_buffer.is_empty() {
            let leftover = std::mem::take(&mut self.residual_buffer);
            if let Ok(s) = std::str::from_utf8(&leftover) {
                self.process_sse_frame_str(s);
            }
        }

        // Construct ToolCalls
        let tool_calls: Vec<ToolCall> = self
            .accumulated_tools
            .into_values()
            .map(|pt| {
                let tool_type = if pt.tool_type.is_empty() {
                    "function".to_string()
                } else {
                    pt.tool_type
                };
                ToolCall {
                    id: pt.id,
                    tool_type,
                    function: traceforge_core::FunctionCall {
                        name: pt.name,
                        arguments: pt.arguments,
                    },
                }
            })
            .collect();

        // Calculate token usage: use upstream usage if available, else estimate
        let token_usage = if let Some(u) = self.usage {
            u
        } else {
            let estimated_chars = self.accumulated_content.len()
                + self.accumulated_reasoning.len()
                + tool_calls
                    .iter()
                    .map(|t| t.function.name.len() + t.function.arguments.len())
                    .sum::<usize>();

            // Standard estimation: ~4 chars per token
            let completion_tokens = ((estimated_chars as f64 / 4.0).ceil() as u32).max(1);
            TokenUsage {
                prompt_tokens: 0,
                completion_tokens,
                total_tokens: completion_tokens,
                reasoning_tokens: if self.accumulated_reasoning.is_empty() {
                    None
                } else {
                    Some(((self.accumulated_reasoning.len() as f64 / 4.0).ceil() as u32).max(1))
                },
            }
        };

        let reasoning_content = if self.accumulated_reasoning.is_empty() {
            None
        } else {
            Some(self.accumulated_reasoning)
        };

        self.overhead_accumulator += op_start.elapsed();

        StreamSummary {
            full_content: self.accumulated_content,
            reasoning_content,
            tool_calls,
            token_usage,
            ttft,
            total_latency,
            proxy_overhead: self.overhead_accumulator,
            chunk_count: self.chunk_count,
            finish_reasons: self.finish_reasons,
        }
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_next_frame_delimiter(buffer: &[u8]) -> Option<(usize, usize)> {
    let p_lf = find_subsequence(buffer, b"\n\n");
    let p_crlf = find_subsequence(buffer, b"\r\n\r\n");

    match (p_lf, p_crlf) {
        (Some(pos_lf), Some(pos_crlf)) => {
            if pos_lf <= pos_crlf {
                Some((pos_lf, 2))
            } else {
                Some((pos_crlf, 4))
            }
        }
        (Some(pos), None) => Some((pos, 2)),
        (None, Some(pos)) => Some((pos, 4)),
        (None, None) => None,
    }
}

pin_project! {
    /// Non-blocking, zero-copy Stream Tee wrapper.
    ///
    /// Passes every incoming `Bytes` chunk directly to the downstream HTTP client
    /// while feeding the internal `StreamAccumulator` with zero allocations.
    pub struct StreamTee<S, F> {
        #[pin]
        inner: S,
        accumulator: Option<StreamAccumulator>,
        on_complete: Option<F>,
    }
}

impl<S, F> StreamTee<S, F>
where
    F: FnOnce(StreamSummary) + Send + 'static,
{
    pub fn new(inner: S, start_time: Instant, on_complete: F) -> Self {
        Self {
            inner,
            accumulator: Some(StreamAccumulator::new(start_time)),
            on_complete: Some(on_complete),
        }
    }
}

impl<S, E, F> Stream for StreamTee<S, F>
where
    S: Stream<Item = Result<Bytes, E>>,
    F: FnOnce(StreamSummary) + Send + 'static,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        match this.inner.poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                if let Some(acc) = this.accumulator.as_mut() {
                    acc.process_bytes(&bytes);
                }
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(err))),
            Poll::Ready(None) => {
                if let Some(acc) = this.accumulator.take() {
                    if let Some(cb) = this.on_complete.take() {
                        let summary = acc.finalize();
                        cb(summary);
                    }
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_chunk_content_accumulation() {
        let start = Instant::now();
        let mut acc = StreamAccumulator::new(start);

        let chunk1 = b"data: {\"id\":\"1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello \"},\"finish_reason\":null}]}\n\n";
        let chunk2 = b"data: {\"id\":\"1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"world!\"},\"finish_reason\":null}]}\n\n";
        let chunk3 = b"data: [DONE]\n\n";

        acc.process_bytes(chunk1);
        acc.process_bytes(chunk2);
        acc.process_bytes(chunk3);

        let summary = acc.finalize();
        assert_eq!(summary.full_content, "Hello world!");
        assert!(summary.ttft.is_some());
        assert_eq!(summary.chunk_count, 3);
        assert!(summary.proxy_overhead < Duration::from_millis(5));
    }

    #[test]
    fn test_sse_fragmented_across_tcp_boundary() {
        let start = Instant::now();
        let mut acc = StreamAccumulator::new(start);

        // Frame split right in the middle of JSON
        let part1 = b"data: {\"id\":\"1\",\"choices\":[{\"index\":0,\"delta\":{\"con";
        let part2 = b"tent\":\"Fragmented message\"},\"finish_reason\":null}]}\n\n";

        acc.process_bytes(part1);
        assert_eq!(acc.accumulated_content, ""); // Not finished yet

        acc.process_bytes(part2);
        let summary = acc.finalize();
        assert_eq!(summary.full_content, "Fragmented message");
    }

    #[test]
    fn test_sse_tool_call_stream_assembly() {
        let start = Instant::now();
        let mut acc = StreamAccumulator::new(start);

        let chunk1 = br#"data: {"id":"1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_99","type":"function","function":{"name":"run_bash","arguments":""}}]},"finish_reason":null}]}

"#;
        let chunk2 = br#"data: {"id":"1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"cmd\":"}}]},"finish_reason":null}]}

"#;
        let chunk3 = br#"data: {"id":"1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"echo hello\"}"}}]},"finish_reason":"tool_calls"}]}

"#;
        let chunk4 = b"data: [DONE]\n\n";

        acc.process_bytes(chunk1);
        acc.process_bytes(chunk2);
        acc.process_bytes(chunk3);
        acc.process_bytes(chunk4);

        let summary = acc.finalize();
        assert_eq!(summary.tool_calls.len(), 1);
        let tool = &summary.tool_calls[0];
        assert_eq!(tool.id, "call_99");
        assert_eq!(tool.function.name, "run_bash");
        assert_eq!(tool.function.arguments, "{\"cmd\":\"echo hello\"}");
        assert_eq!(summary.finish_reasons, vec!["tool_calls"]);
    }

    #[test]
    fn test_sse_reasoning_and_usage_chunks() {
        let start = Instant::now();
        let mut acc = StreamAccumulator::new(start);

        let chunk1 = br#"data: {"id":"1","choices":[{"index":0,"delta":{"reasoning_content":"Let's calculate 2+2."},"finish_reason":null}]}

"#;
        let chunk2 = br#"data: {"id":"1","choices":[{"index":0,"delta":{"content":"The answer is 4."},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}

"#;
        acc.process_bytes(chunk1);
        acc.process_bytes(chunk2);

        let summary = acc.finalize();
        assert_eq!(summary.full_content, "The answer is 4.");
        assert_eq!(summary.reasoning_content.as_deref(), Some("Let's calculate 2+2."));
        assert_eq!(summary.token_usage.total_tokens, 30);
    }

    #[test]
    fn test_multiple_concurrent_tool_calls() {
        let start = Instant::now();
        let mut acc = StreamAccumulator::new(start);

        let chunk1 = br#"data: {"id":"1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.txt\"}"}},{"index":1,"id":"call_b","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"b.txt\"}"}}]},"finish_reason":null}]}

"#;
        let chunk2 = b"data: [DONE]\n\n";

        acc.process_bytes(chunk1);
        acc.process_bytes(chunk2);

        let summary = acc.finalize();
        assert_eq!(summary.tool_calls.len(), 2);
        assert_eq!(summary.tool_calls[0].id, "call_a");
        assert_eq!(summary.tool_calls[0].function.name, "read_file");
        assert_eq!(summary.tool_calls[1].id, "call_b");
        assert_eq!(summary.tool_calls[1].function.name, "read_file");
    }

    #[tokio::test]
    async fn test_stream_tee_async_passthrough() {
        use futures_util::stream;
        use futures_util::StreamExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let start = Instant::now();
        let completed = Arc::new(AtomicBool::new(false));
        let completed_clone = completed.clone();

        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from("data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Chunk 1 \"}}]}\n\n")),
            Ok(Bytes::from("data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Chunk 2\"}}]}\n\n")),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ];

        let base_stream = stream::iter(chunks);
        let mut tee = StreamTee::new(base_stream, start, move |summary| {
            assert_eq!(summary.full_content, "Chunk 1 Chunk 2");
            assert_eq!(summary.chunk_count, 3);
            completed_clone.store(true, Ordering::SeqCst);
        });

        let mut output_bytes = Vec::new();
        while let Some(item) = tee.next().await {
            let b = item.expect("valid chunk");
            output_bytes.extend_from_slice(&b);
        }

        assert!(completed.load(Ordering::SeqCst));
        let output_str = String::from_utf8(output_bytes).unwrap();
        assert!(output_str.contains("Chunk 1 "));
        assert!(output_str.contains("Chunk 2"));
        assert!(output_str.contains("[DONE]"));
    }

    #[test]
    fn test_sub_5ms_overhead_sla() {
        let start = Instant::now();
        let mut acc = StreamAccumulator::new(start);

        let chunk = b"data: {\"id\":\"bench\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"word \"},\"finish_reason\":null}]}\n\n";

        // Simulate 100 consecutive stream chunks
        for _ in 0..100 {
            acc.process_bytes(chunk);
        }

        let summary = acc.finalize();
        assert_eq!(summary.chunk_count, 100);
        // Strict SLA: proxy inspection overhead must be well under 5ms
        assert!(
            summary.proxy_overhead < Duration::from_millis(5),
            "Proxy overhead exceeded 5ms SLA: {:?}",
            summary.proxy_overhead
        );
    }
}
