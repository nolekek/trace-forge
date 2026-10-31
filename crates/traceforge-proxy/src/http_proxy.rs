use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::header::{HeaderName, AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, info, warn};
use ulid::Ulid;

use traceforge_core::{
    EventKind, TokenUsage, ToolCall, TraceEvent, TraceForgeConfig,
};

use crate::stream_tee::StreamTee;

/// Real-time proxy metrics and operational counters.
#[derive(Debug, Default)]
pub struct ProxyMetrics {
    pub total_requests: AtomicU64,
    pub streaming_requests: AtomicU64,
    pub total_tokens: AtomicU64,
    pub total_tool_calls: AtomicU64,
}

/// Shared proxy context passed to Axum route handlers.
#[derive(Clone)]
pub struct ProxyState {
    pub config: Arc<TraceForgeConfig>,
    pub client: Client,
    pub event_tx: Option<Sender<TraceEvent>>,
    pub metrics: Arc<ProxyMetrics>,
    pub start_instant: Instant,
}

impl ProxyState {
    pub fn new(config: TraceForgeConfig, event_tx: Option<Sender<TraceEvent>>) -> Self {
        let client = Client::builder()
            .pool_max_idle_per_host(100)
            .tcp_nodelay(true)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            config: Arc::new(config),
            client,
            event_tx,
            metrics: Arc::new(ProxyMetrics::default()),
            start_instant: Instant::now(),
        }
    }

    /// Emit an event asynchronously to the observability event channel.
    pub fn emit_event(&self, event: TraceEvent) {
        if let Some(ref tx) = self.event_tx {
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = tx.send(event).await {
                    error!("Failed to emit trace event: {:?}", e);
                }
            });
        }
    }
}

/// Health check handler returning status, version, and uptime.
pub async fn health_handler(State(state): State<ProxyState>) -> impl IntoResponse {
    let uptime = state.start_instant.elapsed().as_secs();
    Json(serde_json::json!({
        "status": "ok",
        "service": "traceforge-proxy",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime,
        "upstream_url": state.config.upstream_url,
        "metrics": {
            "total_requests": state.metrics.total_requests.load(Ordering::Relaxed),
            "streaming_requests": state.metrics.streaming_requests.load(Ordering::Relaxed),
            "total_tokens": state.metrics.total_tokens.load(Ordering::Relaxed),
            "total_tool_calls": state.metrics.total_tool_calls.load(Ordering::Relaxed),
        }
    }))
}

/// Reverse proxy handler for POST /v1/chat/completions.
/// Intercepts both streaming (SSE zero-copy tee) and non-streaming requests.
pub async fn chat_completions_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let req_start = Instant::now();
    state.metrics.total_requests.fetch_add(1, Ordering::Relaxed);

    // 1. Session ID resolution
    let session_id = headers
        .get("x-session-id")
        .or_else(|| headers.get("x-traceforge-session-id"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Ulid::new().to_string());

    // 2. Parse request JSON body
    let body_json: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            warn!("Invalid JSON received in chat completions request: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": {
                        "message": format!("Invalid JSON request body: {}", e),
                        "type": "invalid_request_error"
                    }
                })),
            )
                .into_response();
        }
    };

    let is_streaming = body_json
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let model = body_json
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    debug!(
        session_id = %session_id,
        model = %model,
        streaming = is_streaming,
        "Intercepted chat completions request"
    );

    // Emit initial LLM Request telemetry event
    state.emit_event(TraceEvent::llm_request(
        &session_id,
        serde_json::json!({
            "model": model,
            "streaming": is_streaming,
            "messages_count": body_json.get("messages").and_then(|m| m.as_array()).map(|a| a.len()).unwrap_or(0),
        }),
    ));

    // 3. Prepare Upstream Request
    let upstream_target = format!(
        "{}/chat/completions",
        state.config.upstream_url.trim_end_matches('/')
    );

    let mut forward_req = state.client.post(&upstream_target);

    // Propagate Authorization & custom headers
    if let Some(auth) = headers.get(AUTHORIZATION) {
        forward_req = forward_req.header(AUTHORIZATION, auth);
    }
    forward_req = forward_req.header(CONTENT_TYPE, "application/json");

    let upstream_send_start = Instant::now();
    let upstream_res = match forward_req.body(body).send().await {
        Ok(res) => res,
        Err(err) => {
            error!("Upstream LLM connection failure: {}", err);
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": {
                        "message": format!("TraceForge upstream connection failed: {}", err),
                        "type": "bad_gateway"
                    }
                })),
            )
                .into_response();
        }
    };

    let upstream_status = upstream_res.status();
    let upstream_time = upstream_send_start.elapsed();

    // 4. Handle non-streaming response
    if !is_streaming {
        let res_bytes = match upstream_res.bytes().await {
            Ok(b) => b,
            Err(err) => {
                error!("Error reading upstream response body: {}", err);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "message": "Failed to read upstream response" }
                    })),
                )
                    .into_response();
            }
        };

        let total_latency = req_start.elapsed();
        let proxy_overhead = total_latency.saturating_sub(upstream_time);

        // Inspect JSON for token usage and tool calls
        if let Ok(res_json) = serde_json::from_slice::<Value>(&res_bytes) {
            let usage = res_json.get("usage").and_then(|u| {
                serde_json::from_value::<TokenUsage>(u.clone()).ok()
            });

            if let Some(ref u) = usage {
                state.metrics.total_tokens.fetch_add(u.total_tokens as u64, Ordering::Relaxed);
            }

            // Extract tool calls if present
            if let Some(choices) = res_json.get("choices").and_then(|c| c.as_array()) {
                for choice in choices {
                    if let Some(tool_calls) = choice.get("message").and_then(|m| m.get("tool_calls")).and_then(|t| t.as_array()) {
                        for tc in tool_calls {
                            if let Ok(tool) = serde_json::from_value::<ToolCall>(tc.clone()) {
                                state.metrics.total_tool_calls.fetch_add(1, Ordering::Relaxed);
                                state.emit_event(TraceEvent::tool_call(
                                    &session_id,
                                    &tool.id,
                                    &tool.function.name,
                                    &tool.function.arguments,
                                ));
                            }
                        }
                    }
                }
            }

            let cost = usage.as_ref().map(|u| {
                state.config.budgets.calculate_cost(u.prompt_tokens, u.completion_tokens)
            }).unwrap_or(0.0);

            let mut final_evt = TraceEvent::new(
                &session_id,
                EventKind::LlmRequest,
                serde_json::json!({
                    "model": model,
                    "status": upstream_status.as_u16(),
                    "total_latency_us": total_latency.as_micros(),
                    "proxy_overhead_us": proxy_overhead.as_micros(),
                }),
            )
            .with_latency_us(total_latency.as_micros() as u32)
            .with_cost_usd(cost);

            if let Some(u) = usage {
                final_evt = final_evt.with_token_usage(u);
            }
            state.emit_event(final_evt);
        }

        let response = Response::builder()
            .status(upstream_status)
            .header(CONTENT_TYPE, "application/json")
            .header(
                HeaderName::from_static("x-traceforge-session-id"),
                HeaderValue::from_str(&session_id).unwrap_or(HeaderValue::from_static("")),
            )
            .header(
                HeaderName::from_static("x-traceforge-latency-us"),
                HeaderValue::from_str(&total_latency.as_micros().to_string()).unwrap(),
            )
            .header(
                HeaderName::from_static("x-traceforge-overhead-us"),
                HeaderValue::from_str(&proxy_overhead.as_micros().to_string()).unwrap(),
            )
            .body(Body::from(res_bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());

        return response;
    }

    // 5. Handle streaming SSE response with Zero-Copy Stream Teeing
    state.metrics.streaming_requests.fetch_add(1, Ordering::Relaxed);

    // If upstream returned an error status, pass it back immediately without streaming
    if !upstream_status.is_success() {
        let err_bytes = upstream_res.bytes().await.unwrap_or_default();
        return Response::builder()
            .status(upstream_status)
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(err_bytes))
            .unwrap();
    }

    let byte_stream = upstream_res.bytes_stream();
    let state_clone = state.clone();
    let session_id_clone = session_id.clone();
    let model_clone = model.clone();

    let stream_tee = StreamTee::new(byte_stream, req_start, move |summary| {
        let ttft_us = summary.ttft.map(|t| t.as_micros() as u64);
        let total_us = summary.total_latency.as_micros() as u32;
        let overhead_us = summary.proxy_overhead.as_micros() as u64;

        info!(
            session_id = %session_id_clone,
            model = %model_clone,
            chunks = summary.chunk_count,
            ttft_us = ?ttft_us,
            total_us = total_us,
            overhead_us = overhead_us,
            "SSE Stream complete"
        );

        state_clone
            .metrics
            .total_tokens
            .fetch_add(summary.token_usage.total_tokens as u64, Ordering::Relaxed);

        // Emit individual tool call events
        for tool in &summary.tool_calls {
            state_clone.metrics.total_tool_calls.fetch_add(1, Ordering::Relaxed);
            state_clone.emit_event(TraceEvent::tool_call(
                &session_id_clone,
                &tool.id,
                &tool.function.name,
                &tool.function.arguments,
            ));
        }

        // Calculate cost
        let cost = state_clone.config.budgets.calculate_cost(
            summary.token_usage.prompt_tokens,
            summary.token_usage.completion_tokens,
        );

        // Emit final aggregate chunk event
        let final_evt = TraceEvent::new(
            &session_id_clone,
            EventKind::LlmChunk,
            serde_json::json!({
                "model": model_clone,
                "chunks_count": summary.chunk_count,
                "finish_reasons": summary.finish_reasons,
                "ttft_us": ttft_us,
                "total_latency_us": total_us,
                "proxy_overhead_us": overhead_us,
                "tool_calls_count": summary.tool_calls.len(),
            }),
        )
        .with_latency_us(total_us)
        .with_token_usage(summary.token_usage)
        .with_cost_usd(cost);

        state_clone.emit_event(final_evt);
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header(
            HeaderName::from_static("x-traceforge-session-id"),
            HeaderValue::from_str(&session_id).unwrap_or(HeaderValue::from_static("")),
        )
        .body(Body::from_stream(stream_tee))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Catch-all fallback proxy handler forwarding any other paths to upstream.
pub async fn fallback_proxy_handler(
    State(state): State<ProxyState>,
    req: axum::extract::Request,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| format!("?{}", q)).unwrap_or_default();
    let target = format!("{}{}{}", state.config.upstream_url.trim_end_matches('/'), path, query);

    let mut upstream_req = state.client.request(method, &target);
    for (name, val) in req.headers() {
        if name != "host" {
            upstream_req = upstream_req.header(name, val);
        }
    }

    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    match upstream_req.body(body_bytes).send().await {
        Ok(res) => {
            let status = res.status();
            let mut response_builder = Response::builder().status(status);
            for (key, value) in res.headers() {
                response_builder = response_builder.header(key, value);
            }
            let bytes = res.bytes().await.unwrap_or_default();
            response_builder.body(Body::from(bytes)).unwrap()
        }
        Err(e) => {
            error!("Fallback proxy request failed: {}", e);
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}
