use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::http::header::{HeaderName, AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use reqwest::Client;
use serde_json::Value;
use tracing::{debug, error, info, warn};
use ulid::Ulid;

use traceforge_core::{
    EventKind, TokenUsage, ToolCall, TraceEvent, TraceForgeConfig,
};
use traceforge_guard::budget_limiter::BudgetLimiter;
use traceforge_guard::evaluator::PolicyEvaluator;
use traceforge_guard::loop_detector::{SemanticLoopDetector, RECOVERY_PROMPT};
use traceforge_guard::policy_types::{EvaluationDecision, PolicyInput, PolicyLimits};
use traceforge_hitl::{
    ApprovalDecision, BreakpointManager, SessionStateMachine, WsHub,
};
use traceforge_storage::AsyncBatchFlusher;

use crate::mcp_interceptor::{McpInspectionResult, McpInterceptor};
use crate::stream_tee::StreamTee;

/// Real-time proxy metrics and operational counters.
#[derive(Debug, Default)]
pub struct ProxyMetrics {
    pub total_requests: AtomicU64,
    pub streaming_requests: AtomicU64,
    pub total_tokens: AtomicU64,
    pub total_tool_calls: AtomicU64,
    pub intercepted_violations: AtomicU64,
    pub loops_detected: AtomicU64,
    pub hitl_requests: AtomicU64,
}

/// Shared proxy context passed to Axum route handlers.
#[derive(Clone)]
pub struct ProxyState {
    pub config: Arc<TraceForgeConfig>,
    pub client: Client,
    pub flusher: Option<AsyncBatchFlusher>,
    pub budget_limiter: Arc<BudgetLimiter>,
    pub policy_evaluator: Arc<PolicyEvaluator>,
    pub loop_detector: Arc<tokio::sync::Mutex<SemanticLoopDetector>>,
    pub ws_hub: Arc<WsHub>,
    pub breakpoint_manager: Arc<BreakpointManager>,
    pub mcp_interceptor: Arc<McpInterceptor>,
    pub metrics: Arc<ProxyMetrics>,
    pub start_instant: Instant,
}

impl ProxyState {
    /// Creates a new `ProxyState` with customized or default sub-components.
    pub fn new(
        config: TraceForgeConfig,
        flusher: Option<AsyncBatchFlusher>,
        budget_limiter: Option<Arc<BudgetLimiter>>,
        policy_evaluator: Option<Arc<PolicyEvaluator>>,
        loop_detector: Option<Arc<tokio::sync::Mutex<SemanticLoopDetector>>>,
        ws_hub: Option<Arc<WsHub>>,
        breakpoint_manager: Option<Arc<BreakpointManager>>,
    ) -> Self {
        let client = Client::builder()
            .pool_max_idle_per_host(100)
            .tcp_nodelay(true)
            .build()
            .expect("Failed to build HTTP client");

        let default_limits = PolicyLimits::new(
            config.budgets.max_cost_usd_per_session,
            config.budgets.max_tokens_per_session as u64,
        );

        let budget_limiter = budget_limiter
            .unwrap_or_else(|| Arc::new(BudgetLimiter::new(default_limits.clone())));

        let policy_evaluator = policy_evaluator
            .unwrap_or_else(|| Arc::new(PolicyEvaluator::new().with_limits(default_limits)));

        let loop_detector = loop_detector
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(SemanticLoopDetector::default())));

        let breakpoint_manager =
            breakpoint_manager.unwrap_or_else(|| Arc::new(BreakpointManager::new()));

        let ws_hub = ws_hub.unwrap_or_else(|| {
            Arc::new(WsHub::new(
                Arc::new(SessionStateMachine::new()),
                breakpoint_manager.clone(),
            ))
        });

        let mcp_interceptor = Arc::new(McpInterceptor::new(
            policy_evaluator.clone(),
            breakpoint_manager.clone(),
        ));

        Self {
            config: Arc::new(config),
            client,
            flusher,
            budget_limiter,
            policy_evaluator,
            loop_detector,
            ws_hub,
            breakpoint_manager,
            mcp_interceptor,
            metrics: Arc::new(ProxyMetrics::default()),
            start_instant: Instant::now(),
        }
    }

    /// Emits a trace event with sub-microsecond non-blocking latency:
    /// broadcasts to connected WebSocket clients and queues to `AsyncBatchFlusher`.
    pub fn emit_event(&self, event: TraceEvent) {
        // Broadcast in real-time to WebSockets
        self.ws_hub.broadcast_event(event.clone());

        // Low-overhead async queue write (< 5 µs overhead)
        if let Some(ref flusher) = self.flusher {
            if let Err(e) = flusher.send(event) {
                warn!("TraceForge AsyncBatchFlusher send notice: {:?}", e);
            }
        }
    }
}

/// Outcome of inspecting an individual tool call during request/response interception.
enum ToolEvaluationAction {
    Allow,
    Modified(Value),
    Denied {
        rule_id: String,
        message: String,
    },
    LoopDetected {
        recovery_prompt: String,
    },
}

/// Evaluates a single tool call through SemanticLoopDetector, PolicyEvaluator, and BreakpointManager/HITL.
async fn evaluate_tool_call(
    state: &ProxyState,
    session_id: &str,
    tool_name: &str,
    tool_args_val: &Value,
    thought: Option<&str>,
) -> ToolEvaluationAction {
    // 1. Loop & Thrashing Detector Evaluation
    {
        let mut loop_detector = state.loop_detector.lock().await;
        let loop_result =
            loop_detector.check_action(session_id, tool_name, tool_args_val, thought);

        if loop_result.is_loop {
            state.metrics.loops_detected.fetch_add(1, Ordering::Relaxed);
            warn!(
                session_id = %session_id,
                tool = %tool_name,
                similarity = loop_result.similarity_score,
                "Action loop detected by SemanticLoopDetector"
            );

            state.emit_event(TraceEvent::loop_detected(
                session_id,
                tool_name,
                loop_result.similarity_score,
            ));

            let recovery = loop_result
                .recovery_prompt
                .unwrap_or_else(|| RECOVERY_PROMPT.to_string());

            return ToolEvaluationAction::LoopDetected {
                recovery_prompt: recovery,
            };
        }
    }

    // 2. Policy Evaluator Invariant Verification
    let (accum_cost, accum_tokens) = match state.budget_limiter.get_session_stats(session_id) {
        Some(stats) => (stats.accumulated_cost_usd, stats.accumulated_tokens),
        None => (0.0, 0),
    };

    let policy_input = PolicyInput::new(session_id, tool_name)
        .with_tool_args(tool_args_val.clone())
        .with_accumulated_cost(accum_cost)
        .with_token_count(accum_tokens);

    let eval_res = match state.policy_evaluator.evaluate(&policy_input) {
        Ok(r) => r,
        Err(err) => {
            error!("Policy evaluator failure: {}", err);
            return ToolEvaluationAction::Denied {
                rule_id: "evaluator.internal_error".to_string(),
                message: format!("Policy evaluator failure: {}", err),
            };
        }
    };

    if eval_res.decision == EvaluationDecision::Deny {
        state.metrics.intercepted_violations.fetch_add(1, Ordering::Relaxed);
        let first_violation = eval_res.violations.first();
        let rule_id = first_violation
            .map(|v| v.rule_id.clone())
            .unwrap_or_else(|| "safety.invariant".to_string());
        let message = first_violation
            .map(|v| v.message.clone())
            .unwrap_or_else(|| "Operation prohibited by safety policy".to_string());

        warn!(
            session_id = %session_id,
            tool = %tool_name,
            rule = %rule_id,
            "Security invariant violation intercepted: {message}"
        );

        let policy_result = traceforge_core::PolicyResult::deny(&rule_id, &message);
        state.emit_event(TraceEvent::policy_evaluation(
            session_id,
            policy_result,
            serde_json::to_value(&policy_input).unwrap_or_default(),
        ));

        let _ = state.ws_hub.state_machine().trip_circuit(session_id, &message);

        return ToolEvaluationAction::Denied { rule_id, message };
    }

    // 3. HITL & Breakpoint Inspection
    let is_breakpoint = state.breakpoint_manager.should_break(session_id, tool_name);
    let requires_hitl = eval_res.decision == EvaluationDecision::RequireHitl || is_breakpoint;

    if requires_hitl {
        state.metrics.hitl_requests.fetch_add(1, Ordering::Relaxed);
        let reason = if is_breakpoint {
            format!("Execution breakpoint active on tool '{tool_name}'")
        } else {
            eval_res.hitl_reasons.join("; ")
        };

        info!(
            session_id = %session_id,
            tool = %tool_name,
            reason = %reason,
            "Freezing execution for Human-In-The-Loop approval"
        );

        // Await operator resolution via WebSocket
        let approval_result = state
            .ws_hub
            .request_approval(
                session_id,
                tool_name,
                tool_args_val.clone(),
                &reason,
                Some(std::time::Duration::from_secs(60)),
            )
            .await;

        match approval_result {
            Ok(ApprovalDecision::Allow) => {
                info!(session_id = %session_id, tool = %tool_name, "Operator allowed tool call");
                state.emit_event(TraceEvent::hitl_action(session_id, "approval", "allowed"));
                return ToolEvaluationAction::Allow;
            }
            Ok(ApprovalDecision::Modify { modified_args }) => {
                info!(session_id = %session_id, tool = %tool_name, "Operator modified tool call arguments");
                state.emit_event(TraceEvent::hitl_action(
                    session_id,
                    "modify",
                    "arguments_modified",
                ));
                return ToolEvaluationAction::Modified(modified_args);
            }
            Ok(ApprovalDecision::Deny { reason }) => {
                let deny_reason = reason.unwrap_or_else(|| "Rejected by human operator".to_string());
                warn!(session_id = %session_id, tool = %tool_name, "Operator denied tool call: {deny_reason}");
                state.emit_event(TraceEvent::hitl_action(session_id, "deny", &deny_reason));
                return ToolEvaluationAction::Denied {
                    rule_id: "hitl.operator_denied".to_string(),
                    message: deny_reason,
                };
            }
            Err(e) => {
                error!(session_id = %session_id, tool = %tool_name, "HITL approval error or timeout: {e}");
                state.emit_event(TraceEvent::hitl_action(session_id, "error", &e.to_string()));
                return ToolEvaluationAction::Denied {
                    rule_id: "hitl.timeout_or_error".to_string(),
                    message: format!("HITL approval cancelled or timed out: {e}"),
                };
            }
        }
    }

    ToolEvaluationAction::Allow
}

/// Builds a synthetic LLM chat completion response containing an error or recovery prompt.
fn build_synthetic_completion_response(
    session_id: &str,
    model: &str,
    content: &str,
    error_type: &str,
) -> Response {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let completion_id = format!("chatcmpl-tf-{}", Ulid::new());

    let payload = serde_json::json!({
        "id": completion_id,
        "object": "chat.completion",
        "created": now_secs,
        "model": model,
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0
        },
        "policy_violation": {
            "type": error_type,
            "message": content
        }
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .header(
            HeaderName::from_static("x-traceforge-session-id"),
            HeaderValue::from_str(session_id).unwrap_or(HeaderValue::from_static("")),
        )
        .header(
            HeaderName::from_static("x-traceforge-interception"),
            HeaderValue::from_static("synthetic-correction"),
        )
        .body(Body::from(serde_json::to_vec(&payload).unwrap_or_default()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Health check handler returning status, version, and operational telemetry.
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
            "intercepted_violations": state.metrics.intercepted_violations.load(Ordering::Relaxed),
            "loops_detected": state.metrics.loops_detected.load(Ordering::Relaxed),
            "hitl_requests": state.metrics.hitl_requests.load(Ordering::Relaxed),
        }
    }))
}

/// WebSocket route handler registering at `/ws/control`.
pub async fn ws_route_handler(
    State(state): State<ProxyState>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| state.ws_hub.clone().handle_socket(socket))
}

/// Reverse proxy handler for POST /v1/chat/completions.
/// Intercepts requests, evaluates safety invariants, and performs zero-copy SSE teeing.
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

    // 2. Pre-Flight Budget Check (< 0.2ms)
    if let Err(budget_err) = state.budget_limiter.check_limits(&session_id) {
        warn!(
            session_id = %session_id,
            "Pre-flight budget check exceeded: {budget_err}"
        );

        let policy_res =
            traceforge_core::PolicyResult::deny("budget.limit_exceeded", budget_err.to_string());
        let evt = TraceEvent::new(
            &session_id,
            EventKind::PolicyEvaluation,
            serde_json::json!({
                "error": "budget_exceeded",
                "details": budget_err.to_string(),
            }),
        )
        .with_policy_result(policy_res);

        state.emit_event(evt);

        return Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header(CONTENT_TYPE, "application/json")
            .header(
                HeaderName::from_static("x-traceforge-session-id"),
                HeaderValue::from_str(&session_id).unwrap_or(HeaderValue::from_static("")),
            )
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "error": {
                        "message": format!("Session budget exceeded: {budget_err}"),
                        "type": "budget_exceeded",
                        "code": 429
                    }
                }))
                .unwrap_or_default(),
            ))
            .unwrap_or_else(|_| StatusCode::TOO_MANY_REQUESTS.into_response());
    }

    // 3. Parse request JSON body
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

    // 4. Prepare Upstream Request
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

    // 5. Handle non-streaming response
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

        if let Ok(mut res_json) = serde_json::from_slice::<Value>(&res_bytes) {
            let usage = res_json.get("usage").and_then(|u| {
                serde_json::from_value::<TokenUsage>(u.clone()).ok()
            });

            let cost = usage
                .as_ref()
                .map(|u| {
                    state
                        .config
                        .budgets
                        .calculate_cost(u.prompt_tokens, u.completion_tokens)
                })
                .unwrap_or(0.0);

            // Record token and cost consumption into the budget limiter
            if let Some(ref u) = usage {
                state
                    .metrics
                    .total_tokens
                    .fetch_add(u.total_tokens as u64, Ordering::Relaxed);
                let _ = state
                    .budget_limiter
                    .record_usage(&session_id, u.total_tokens as u64, cost);
            }

            // Inspect and evaluate tool calls if present
            if let Some(choices) = res_json.get_mut("choices").and_then(|c| c.as_array_mut()) {
                for choice in choices {
                    if let Some(tool_calls) = choice
                        .get_mut("message")
                        .and_then(|m| m.get_mut("tool_calls"))
                        .and_then(|t| t.as_array_mut())
                    {
                        for tc in tool_calls {
                            if let Ok(tool) = serde_json::from_value::<ToolCall>(tc.clone()) {
                                state.metrics.total_tool_calls.fetch_add(1, Ordering::Relaxed);

                                state.emit_event(TraceEvent::tool_call(
                                    &session_id,
                                    &tool.id,
                                    &tool.function.name,
                                    &tool.function.arguments,
                                ));

                                let args_val: Value =
                                    serde_json::from_str(&tool.function.arguments)
                                        .unwrap_or_else(|_| Value::String(tool.function.arguments.clone()));

                                let action = evaluate_tool_call(
                                    &state,
                                    &session_id,
                                    &tool.function.name,
                                    &args_val,
                                    None,
                                )
                                .await;

                                match action {
                                    ToolEvaluationAction::Allow => {
                                        // Proceed unmodified
                                    }
                                    ToolEvaluationAction::Modified(modified_args) => {
                                        // Update arguments in tool call response
                                        if let Some(f) = tc.get_mut("function") {
                                            f["arguments"] = Value::String(
                                                serde_json::to_string(&modified_args)
                                                    .unwrap_or_default(),
                                            );
                                        }
                                    }
                                    ToolEvaluationAction::Denied { rule_id, message } => {
                                        let synthetic_content = format!(
                                            "TraceForge Guard: Action blocked due to policy invariant [{}]. {}",
                                            rule_id, message
                                        );
                                        return build_synthetic_completion_response(
                                            &session_id,
                                            &model,
                                            &synthetic_content,
                                            "policy_violation",
                                        );
                                    }
                                    ToolEvaluationAction::LoopDetected { recovery_prompt } => {
                                        return build_synthetic_completion_response(
                                            &session_id,
                                            &model,
                                            &recovery_prompt,
                                            "loop_detected",
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

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

            let res_final_bytes = serde_json::to_vec(&res_json).unwrap_or_else(|_| res_bytes.to_vec());

            return Response::builder()
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
                .body(Body::from(res_final_bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }

        // Raw passthrough if not valid JSON
        return Response::builder()
            .status(upstream_status)
            .header(CONTENT_TYPE, "application/json")
            .header(
                HeaderName::from_static("x-traceforge-session-id"),
                HeaderValue::from_str(&session_id).unwrap_or(HeaderValue::from_static("")),
            )
            .body(Body::from(res_bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    // 6. Handle streaming SSE response with Zero-Copy Stream Teeing
    state.metrics.streaming_requests.fetch_add(1, Ordering::Relaxed);

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

        let cost = state_clone.config.budgets.calculate_cost(
            summary.token_usage.prompt_tokens,
            summary.token_usage.completion_tokens,
        );

        // Record usage into budget limiter
        let _ = state_clone.budget_limiter.record_usage(
            &session_id_clone,
            summary.token_usage.total_tokens as u64,
            cost,
        );

        // Emit individual tool call events and evaluate in background task
        for tool in &summary.tool_calls {
            state_clone.metrics.total_tool_calls.fetch_add(1, Ordering::Relaxed);
            state_clone.emit_event(TraceEvent::tool_call(
                &session_id_clone,
                &tool.id,
                &tool.function.name,
                &tool.function.arguments,
            ));

            let sc = state_clone.clone();
            let sess = session_id_clone.clone();
            let t_name = tool.function.name.clone();
            let t_args_str = tool.function.arguments.clone();
            let thought = summary.reasoning_content.clone();

            tokio::spawn(async move {
                let args_val: Value = serde_json::from_str(&t_args_str)
                    .unwrap_or_else(|_| Value::String(t_args_str));
                let _ = evaluate_tool_call(
                    &sc,
                    &sess,
                    &t_name,
                    &args_val,
                    thought.as_deref(),
                )
                .await;
            });
        }

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

/// Route handler for incoming MCP JSON-RPC 2.0 requests over HTTP.
pub async fn mcp_route_handler(
    State(state): State<ProxyState>,
    body: String,
) -> Response {
    let result = state.mcp_interceptor.inspect_mcp_request(&body);
    match result {
        McpInspectionResult::Blocked { error_response, .. } => {
            (StatusCode::OK, Json(error_response)).into_response()
        }
        McpInspectionResult::InvalidJsonRpc { error_response, .. } => {
            (StatusCode::BAD_REQUEST, Json(error_response)).into_response()
        }
        McpInspectionResult::AwaitingApproval {
            session_id,
            tool_name,
            arguments,
            reason,
        } => {
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "result": {
                    "status": "AWAITING_APPROVAL",
                    "session_id": session_id,
                    "tool": tool_name,
                    "arguments": arguments,
                    "reason": reason,
                }
            });
            (StatusCode::ACCEPTED, Json(resp)).into_response()
        }
        McpInspectionResult::Allowed { method, tool_name, params } => {
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "result": {
                    "status": "allowed",
                    "method": method,
                    "tool": tool_name,
                    "params": params,
                }
            });
            (StatusCode::OK, Json(resp)).into_response()
        }
    }
}

