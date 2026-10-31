use serde::{Deserialize, Serialize};

/// Detailed token usage accounting for an LLM exchange.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}

impl TokenUsage {
    pub fn new(prompt_tokens: u32, completion_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            reasoning_tokens: None,
        }
    }

    pub fn with_reasoning(mut self, reasoning_tokens: u32) -> Self {
        self.reasoning_tokens = Some(reasoning_tokens);
        self
    }

    pub fn add(&mut self, other: &TokenUsage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
        if let Some(r) = other.reasoning_tokens {
            *self.reasoning_tokens.get_or_insert(0) += r;
        }
    }
}

/// A structured tool call extracted from LLM completions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "default_tool_type")]
    pub tool_type: String,
    pub function: FunctionCall,
}

fn default_tool_type() -> String {
    "function".to_string()
}

impl ToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            tool_type: "function".to_string(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }
}

/// The specific function details inside a tool call.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Result of policy invariant evaluation (e.g., OPA or loop detectors).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyResult {
    pub allow: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub require_hitl: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violations: Vec<String>,
}

impl PolicyResult {
    pub fn allow() -> Self {
        Self {
            allow: true,
            rule_name: None,
            reason: None,
            require_hitl: false,
            violations: Vec::new(),
        }
    }

    pub fn deny(rule_name: impl Into<String>, reason: impl Into<String>) -> Self {
        let reason_str = reason.into();
        Self {
            allow: false,
            rule_name: Some(rule_name.into()),
            reason: Some(reason_str.clone()),
            require_hitl: false,
            violations: vec![reason_str],
        }
    }

    pub fn hitl_required(rule_name: impl Into<String>, reason: impl Into<String>) -> Self {
        let reason_str = reason.into();
        Self {
            allow: false,
            rule_name: Some(rule_name.into()),
            reason: Some(reason_str.clone()),
            require_hitl: true,
            violations: vec![reason_str],
        }
    }
}

/// Execution state of an agent session or stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionState {
    #[default]
    Running,
    Streaming,
    AwaitingPolicy,
    AwaitingApproval,
    Paused,
    Tripped,
    Completed,
}

/// Latency telemetry container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LatencyMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttft_us: Option<u64>,
    pub total_latency_us: u64,
    pub proxy_overhead_us: u64,
}

/// Representation of an SSE streaming chunk from OpenAI-compatible models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<StreamChoice>,
    #[serde(default)]
    pub usage: Option<TokenUsage>,
}

/// Single choice entry in a streaming response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChoice {
    pub index: usize,
    pub delta: StreamDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// Delta payload representing incremental token or tool generation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamDelta {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<StreamToolCallDelta>>,
}

/// Streaming delta for a tool call slice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamToolCallDelta {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub tool_type: Option<String>,
    #[serde(default)]
    pub function: Option<StreamFunctionDelta>,
}

/// Streaming delta for function arguments or name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_usage_addition() {
        let mut usage1 = TokenUsage::new(10, 20);
        let usage2 = TokenUsage::new(5, 15).with_reasoning(8);
        usage1.add(&usage2);

        assert_eq!(usage1.prompt_tokens, 15);
        assert_eq!(usage1.completion_tokens, 35);
        assert_eq!(usage1.total_tokens, 50);
        assert_eq!(usage1.reasoning_tokens, Some(8));
    }

    #[test]
    fn test_tool_call_serialization() {
        let tool = ToolCall::new("call_1", "execute_bash", "{\"command\":\"ls\"}");
        let json = serde_json::to_string(&tool).expect("serialize tool");
        let parsed: ToolCall = serde_json::from_str(&json).expect("deserialize tool");
        assert_eq!(parsed.id, "call_1");
        assert_eq!(parsed.function.name, "execute_bash");
        assert_eq!(parsed.function.arguments, "{\"command\":\"ls\"}");
    }

    #[test]
    fn test_policy_result_variants() {
        let ok = PolicyResult::allow();
        assert!(ok.allow);
        assert!(!ok.require_hitl);

        let hitl = PolicyResult::hitl_required("bash_guard", "destructive command");
        assert!(!hitl.allow);
        assert!(hitl.require_hitl);
        assert_eq!(hitl.rule_name.as_deref(), Some("bash_guard"));
    }

    #[test]
    fn test_stream_chunk_deserialization() {
        let json_data = r#"{
            "id": "chatcmpl-test",
            "object": "chat.completion.chunk",
            "created": 1720000000,
            "model": "gpt-4o",
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "content": "Hello",
                        "reasoning_content": "Thinking step"
                    },
                    "finish_reason": null
                }
            ],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 8,
                "total_tokens": 20
            }
        }"#;

        let chunk: StreamChunk = serde_json::from_str(json_data).expect("deserialization ok");
        assert_eq!(chunk.id.as_deref(), Some("chatcmpl-test"));
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("Hello"));
        assert_eq!(
            chunk.choices[0].delta.reasoning_content.as_deref(),
            Some("Thinking step")
        );
        let usage = chunk.usage.expect("usage present");
        assert_eq!(usage.total_tokens, 20);
    }

    #[test]
    fn test_session_state_default() {
        let state = SessionState::default();
        assert_eq!(state, SessionState::Running);
    }
}
