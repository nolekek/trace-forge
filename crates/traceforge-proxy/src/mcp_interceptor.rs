//! # MCP (Model Context Protocol) JSON-RPC Interceptor
//!
//! Sub-millisecond inspection and interception for MCP JSON-RPC 2.0 messages.
//! Enforces OPA invariant security firewall rules and step-by-step breakpoints on tool calls.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};

use traceforge_guard::evaluator::PolicyEvaluator;
use traceforge_guard::policy_types::{EvaluationDecision, PolicyInput};
use traceforge_hitl::BreakpointManager;

/// Standard JSON-RPC error code for invariant firewall policy violations.
pub const MCP_BLOCKED_ERROR_CODE: i64 = -32000;

/// Standard error message for blocked MCP tool calls.
pub const MCP_BLOCKED_MESSAGE: &str = "Action blocked by TraceForge Invariant Firewall";

/// JSON-RPC 2.0 Error descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Fully formatted JSON-RPC 2.0 Error Response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: String,
    pub id: Value,
    pub error: JsonRpcError,
}

/// The outcome of evaluating an MCP JSON-RPC message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "details")]
pub enum McpInspectionResult {
    /// Tool execution or non-tool method permitted.
    Allowed {
        method: String,
        tool_name: Option<String>,
        params: Option<Value>,
    },
    /// Destructive tool execution blocked by safety invariant.
    Blocked {
        error_response: Value,
        rule_id: String,
        message: String,
    },
    /// Execution suspended awaiting human operator confirmation or breakpoint resume.
    AwaitingApproval {
        session_id: String,
        tool_name: String,
        arguments: Value,
        reason: String,
    },
    /// Unparseable or malformed JSON-RPC 2.0 request.
    InvalidJsonRpc {
        error_response: Value,
        message: String,
    },
}

impl McpInspectionResult {
    /// Returns true if the message was allowed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    /// Returns true if the message was blocked by invariant firewall.
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }

    /// Returns true if the message requires approval before continuing.
    pub fn is_awaiting_approval(&self) -> bool {
        matches!(self, Self::AwaitingApproval { .. })
    }

    /// Extracts the JSON-RPC error response Value if blocked or invalid.
    pub fn error_response(&self) -> Option<&Value> {
        match self {
            Self::Blocked { error_response, .. } | Self::InvalidJsonRpc { error_response, .. } => {
                Some(error_response)
            }
            _ => None,
        }
    }
}

/// High-throughput interceptor for Model Context Protocol (MCP) JSON-RPC streams.
#[derive(Clone)]
pub struct McpInterceptor {
    policy_evaluator: Arc<PolicyEvaluator>,
    breakpoint_manager: Arc<BreakpointManager>,
    default_session_id: String,
}

impl Default for McpInterceptor {
    fn default() -> Self {
        Self::new(
            Arc::new(PolicyEvaluator::new()),
            Arc::new(BreakpointManager::new()),
        )
    }
}

impl McpInterceptor {
    /// Constructs a new `McpInterceptor` with shared policy and breakpoint components.
    pub fn new(
        policy_evaluator: Arc<PolicyEvaluator>,
        breakpoint_manager: Arc<BreakpointManager>,
    ) -> Self {
        Self {
            policy_evaluator,
            breakpoint_manager,
            default_session_id: "default_mcp_session".to_string(),
        }
    }

    /// Overrides the default session ID used for telemetry and state tracking.
    pub fn with_default_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.default_session_id = session_id.into();
        self
    }

    pub fn policy_evaluator(&self) -> &Arc<PolicyEvaluator> {
        &self.policy_evaluator
    }

    pub fn breakpoint_manager(&self) -> &Arc<BreakpointManager> {
        &self.breakpoint_manager
    }

    /// Inspects an incoming MCP JSON-RPC 2.0 message using the default session ID.
    pub fn inspect_mcp_request(&self, raw_json: &str) -> McpInspectionResult {
        self.inspect_mcp_request_with_session(raw_json, &self.default_session_id)
    }

    /// Inspects an incoming MCP JSON-RPC 2.0 message with a designated session ID.
    pub fn inspect_mcp_request_with_session(
        &self,
        raw_json: &str,
        session_id: &str,
    ) -> McpInspectionResult {
        let trimmed = raw_json.trim();
        if trimmed.is_empty() {
            let error_response = json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": {
                    "code": -32700,
                    "message": "Parse error: empty request"
                }
            });
            return McpInspectionResult::InvalidJsonRpc {
                error_response,
                message: "Empty request string".to_string(),
            };
        }

        // 1. Parse JSON payload
        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(err) => {
                let error_response = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {
                        "code": -32700,
                        "message": format!("Parse error: {}", err)
                    }
                });
                return McpInspectionResult::InvalidJsonRpc {
                    error_response,
                    message: format!("Invalid JSON: {}", err),
                };
            }
        };

        let obj = match parsed.as_object() {
            Some(o) => o,
            None => {
                let error_response = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {
                        "code": -32600,
                        "message": "Invalid Request: Expected a JSON object"
                    }
                });
                return McpInspectionResult::InvalidJsonRpc {
                    error_response,
                    message: "JSON-RPC 2.0 request must be a JSON object".to_string(),
                };
            }
        };

        // 2. Extract JSON-RPC ID
        let id = obj.get("id").cloned().unwrap_or(Value::Null);

        // 3. Validate JSON-RPC version
        let jsonrpc_val = obj.get("jsonrpc").and_then(|v| v.as_str());
        if jsonrpc_val != Some("2.0") {
            let error_response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32600,
                    "message": "Invalid Request: 'jsonrpc' version must be '2.0'"
                }
            });
            return McpInspectionResult::InvalidJsonRpc {
                error_response,
                message: "Missing or invalid 'jsonrpc' field; expected '2.0'".to_string(),
            };
        }

        // 4. Extract Method
        let method = match obj.get("method").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => {
                let error_response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32600,
                        "message": "Invalid Request: Missing 'method'"
                    }
                });
                return McpInspectionResult::InvalidJsonRpc {
                    error_response,
                    message: "Missing 'method' in JSON-RPC request".to_string(),
                };
            }
        };

        let params_val = obj.get("params");

        // 5. Check if this is an MCP tool execution (e.g. "tools/call" or "call_tool")
        let is_tool_call = method == "tools/call" || method == "call_tool";

        if !is_tool_call {
            // General administrative / discovery calls (initialize, ping, tools/list, etc.) are allowed
            return McpInspectionResult::Allowed {
                method: method.to_string(),
                tool_name: None,
                params: params_val.cloned(),
            };
        }

        // 6. Extract tool name and arguments from params
        let (tool_name, arguments_val) = match params_val {
            Some(Value::Object(p)) => {
                let name = p
                    .get("name")
                    .or_else(|| p.get("tool"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let args = p
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Default::default()));

                (name, args)
            }
            _ => ("".to_string(), Value::Object(Default::default())),
        };

        if tool_name.trim().is_empty() {
            let error_response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32602,
                    "message": "Invalid params: Missing tool 'name'"
                }
            });
            return McpInspectionResult::InvalidJsonRpc {
                error_response,
                message: "Missing tool 'name' in MCP tools/call params".to_string(),
            };
        }

        // Normalize arguments: parse JSON string if passed as string, otherwise preserve object
        let normalized_args = match &arguments_val {
            Value::Object(_) => arguments_val.clone(),
            Value::String(s) => {
                if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                    if parsed.is_object() {
                        parsed
                    } else {
                        json!({ "command": s })
                    }
                } else {
                    json!({ "command": s })
                }
            }
            _ => json!({}),
        };

        // 7. Check BreakpointManager for execution freeze
        let should_break = self.breakpoint_manager.should_break(session_id, &tool_name);
        if should_break {
            info!(
                session_id = %session_id,
                tool = %tool_name,
                "Execution breakpoint triggered on MCP tool call"
            );
            return McpInspectionResult::AwaitingApproval {
                session_id: session_id.to_string(),
                tool_name: tool_name.clone(),
                arguments: normalized_args,
                reason: format!("Execution breakpoint active on MCP tool '{tool_name}'"),
            };
        }

        // 8. Evaluate through PolicyEvaluator
        let effective_session = if session_id.trim().is_empty() {
            &self.default_session_id
        } else {
            session_id
        };

        let policy_input = PolicyInput::new(effective_session, &tool_name)
            .with_tool_args(normalized_args.clone());

        let eval_res = match self.policy_evaluator.evaluate(&policy_input) {
            Ok(res) => res,
            Err(err) => {
                warn!("Policy evaluation error during MCP inspection: {}", err);
                let error_response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": MCP_BLOCKED_ERROR_CODE,
                        "message": MCP_BLOCKED_MESSAGE,
                        "data": {
                            "rule_id": "evaluator.internal_error",
                            "error": err.to_string(),
                        }
                    }
                });
                return McpInspectionResult::Blocked {
                    error_response,
                    rule_id: "evaluator.internal_error".to_string(),
                    message: format!("Policy evaluation error: {}", err),
                };
            }
        };

        match eval_res.decision {
            EvaluationDecision::Deny => {
                let first_violation = eval_res.violations.first();
                let rule_id = first_violation
                    .map(|v| v.rule_id.clone())
                    .unwrap_or_else(|| "safety.invariant".to_string());
                let violation_msg = first_violation
                    .map(|v| v.message.clone())
                    .unwrap_or_else(|| MCP_BLOCKED_MESSAGE.to_string());

                warn!(
                    session_id = %session_id,
                    tool = %tool_name,
                    rule = %rule_id,
                    "MCP tool call blocked by TraceForge Invariant Firewall: {violation_msg}"
                );

                let error_response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": MCP_BLOCKED_ERROR_CODE,
                        "message": MCP_BLOCKED_MESSAGE,
                        "data": {
                            "rule_id": rule_id,
                            "violations": eval_res.violations,
                            "details": violation_msg,
                        }
                    }
                });

                McpInspectionResult::Blocked {
                    error_response,
                    rule_id,
                    message: MCP_BLOCKED_MESSAGE.to_string(),
                }
            }
            EvaluationDecision::RequireHitl => {
                let reason = eval_res.hitl_reasons.join("; ");
                let reason_str = if reason.is_empty() {
                    "Human-In-The-Loop review required by policy".to_string()
                } else {
                    reason
                };

                info!(
                    session_id = %session_id,
                    tool = %tool_name,
                    reason = %reason_str,
                    "MCP tool call requires operator approval"
                );

                McpInspectionResult::AwaitingApproval {
                    session_id: session_id.to_string(),
                    tool_name,
                    arguments: normalized_args,
                    reason: reason_str,
                }
            }
            EvaluationDecision::Allow => McpInspectionResult::Allowed {
                method: method.to_string(),
                tool_name: Some(tool_name),
                params: params_val.cloned(),
            },
        }
    }
}

/// Standalone convenience helper inspecting an MCP JSON-RPC 2.0 message with default policy engines.
pub fn inspect_mcp_request(raw_json: &str) -> McpInspectionResult {
    McpInterceptor::default().inspect_mcp_request(raw_json)
}
