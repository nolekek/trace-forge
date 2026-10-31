use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::types::{PolicyResult, TokenUsage};

/// Discrete event classifications within TraceForge's observability plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    LlmRequest,
    LlmChunk,
    ToolCall,
    PolicyEvaluation,
    LoopDetected,
    HitlAction,
}

/// The core telemetry envelope capturing LLM and agent runtime execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub event_id: Ulid,
    pub session_id: String,
    pub timestamp_ns: u64,
    pub latency_us: u32,
    pub kind: EventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsage>,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_result: Option<PolicyResult>,
    pub payload: serde_json::Value,
}

impl TraceEvent {
    /// Creates a base trace event with the current timestamp in nanoseconds.
    pub fn new(
        session_id: impl Into<String>,
        kind: EventKind,
        payload: serde_json::Value,
    ) -> Self {
        let timestamp_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);

        Self {
            event_id: Ulid::new(),
            session_id: session_id.into(),
            timestamp_ns,
            latency_us: 0,
            kind,
            token_usage: None,
            cost_usd: 0.0,
            policy_result: None,
            payload,
        }
    }

    pub fn with_latency_us(mut self, latency_us: u32) -> Self {
        self.latency_us = latency_us;
        self
    }

    pub fn with_token_usage(mut self, usage: TokenUsage) -> Self {
        self.token_usage = Some(usage);
        self
    }

    pub fn with_cost_usd(mut self, cost: f64) -> Self {
        self.cost_usd = cost;
        self
    }

    pub fn with_policy_result(mut self, result: PolicyResult) -> Self {
        self.policy_result = Some(result);
        self
    }

    pub fn llm_request(session_id: impl Into<String>, payload: serde_json::Value) -> Self {
        Self::new(session_id, EventKind::LlmRequest, payload)
    }

    pub fn llm_chunk(
        session_id: impl Into<String>,
        chunk_index: usize,
        delta_payload: serde_json::Value,
    ) -> Self {
        Self::new(
            session_id,
            EventKind::LlmChunk,
            serde_json::json!({
                "chunk_index": chunk_index,
                "delta": delta_payload
            }),
        )
    }

    pub fn tool_call(
        session_id: impl Into<String>,
        tool_id: &str,
        tool_name: &str,
        arguments: &str,
    ) -> Self {
        Self::new(
            session_id,
            EventKind::ToolCall,
            serde_json::json!({
                "tool_id": tool_id,
                "tool_name": tool_name,
                "arguments": arguments,
            }),
        )
    }

    pub fn policy_evaluation(
        session_id: impl Into<String>,
        policy_result: PolicyResult,
        input_context: serde_json::Value,
    ) -> Self {
        let mut evt = Self::new(session_id, EventKind::PolicyEvaluation, input_context);
        evt.policy_result = Some(policy_result);
        evt
    }

    pub fn loop_detected(
        session_id: impl Into<String>,
        repeated_action: &str,
        similarity_score: f32,
    ) -> Self {
        Self::new(
            session_id,
            EventKind::LoopDetected,
            serde_json::json!({
                "action": repeated_action,
                "similarity_score": similarity_score,
            }),
        )
    }

    pub fn hitl_action(
        session_id: impl Into<String>,
        action_type: &str,
        resolution: &str,
    ) -> Self {
        Self::new(
            session_id,
            EventKind::HitlAction,
            serde_json::json!({
                "action_type": action_type,
                "resolution": resolution,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_event_creation_and_serde() {
        let evt = TraceEvent::llm_request("sess_123", serde_json::json!({"model": "gpt-4o"}))
            .with_latency_us(1250)
            .with_token_usage(TokenUsage::new(100, 50))
            .with_cost_usd(0.0035);

        assert_eq!(evt.session_id, "sess_123");
        assert_eq!(evt.kind, EventKind::LlmRequest);
        assert_eq!(evt.latency_us, 1250);
        assert_eq!(evt.token_usage.as_ref().unwrap().total_tokens, 150);

        let serialized = serde_json::to_string(&evt).expect("serialize event");
        let deserialized: TraceEvent = serde_json::from_str(&serialized).expect("deserialize event");

        assert_eq!(deserialized.session_id, "sess_123");
        assert_eq!(deserialized.kind, EventKind::LlmRequest);
        assert_eq!(deserialized.latency_us, 1250);
    }

    #[test]
    fn test_event_kinds() {
        let tool_evt = TraceEvent::tool_call("s1", "call_1", "run_bash", "ls -la");
        assert_eq!(tool_evt.kind, EventKind::ToolCall);

        let loop_evt = TraceEvent::loop_detected("s1", "fetch_url", 0.98);
        assert_eq!(loop_evt.kind, EventKind::LoopDetected);

        let hitl_evt = TraceEvent::hitl_action("s1", "approve", "operator confirmed");
        assert_eq!(hitl_evt.kind, EventKind::HitlAction);
    }
}
