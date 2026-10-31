use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use http_body_util::BodyExt;
use tower::ServiceExt;

use traceforge_core::TraceForgeConfig;
use traceforge_guard::policy_types::PolicyLimits;
use traceforge_hitl::{ApprovalDecision, HitlSessionState, WsOutboundMessage};
use traceforge_proxy::ProxyServer;
use traceforge_storage::{AsyncBatchFlusher, SqliteTraceStore, TraceStore};

/// Spawns an ephemeral mock LLM server returning the given JSON responses.
async fn spawn_mock_upstream<F>(handler: F) -> (String, tokio::task::JoinHandle<()>)
where
    F: Fn(serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
{
    let handler_arc = Arc::new(handler);
    let app = Router::new().route(
        "/chat/completions",
        post({
            let h = handler_arc.clone();
            move |Json(body): Json<serde_json::Value>| {
                let h = h.clone();
                async move {
                    let resp = h(body);
                    Json(resp).into_response()
                }
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    let url = format!("http://127.0.0.1:{}", addr.port());

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });

    (url, handle)
}

#[tokio::test]
async fn test_pipeline_budget_limit_blocking() {
    let dummy_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let config = TraceForgeConfig::new(dummy_addr, "http://127.0.0.1:9999");
    let server = ProxyServer::new(config, None);

    let session_id = "sess_budget_breached";

    // Pre-seed session budget with usage exceeding the default $10.0 limit
    server
        .budget_limiter()
        .set_session_limits(session_id, PolicyLimits::new(5.0, 1000));
    let _ = server
        .budget_limiter()
        .record_usage(session_id, 2000, 10.0);

    let app = server.router();

    let request_body = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Hello world"}]
    });

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("Content-Type", "application/json")
        .header("x-session-id", session_id)
        .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    // Must return HTTP 429 Too Many Requests
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(body_json["error"]["type"], "budget_exceeded");
    assert_eq!(body_json["error"]["code"], 429);
    assert!(body_json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("exceeded"));
}

#[tokio::test]
async fn test_pipeline_destructive_command_blocking() {
    // Upstream returns a destructive command
    let (upstream_url, _handle) = spawn_mock_upstream(|_req| {
        serde_json::json!({
            "id": "chatcmpl-mock-rm",
            "object": "chat.completion",
            "created": 1720000000,
            "model": "gpt-4o",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call_rm_1",
                                "type": "function",
                                "function": {
                                    "name": "execute_bash",
                                    "arguments": "{\"command\":\"rm -rf /\"}"
                                }
                            }
                        ]
                    },
                    "finish_reason": "tool_calls"
                }
            ],
            "usage": {
                "prompt_tokens": 15,
                "completion_tokens": 15,
                "total_tokens": 30
            }
        })
    })
    .await;

    let dummy_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let config = TraceForgeConfig::new(dummy_addr, upstream_url);
    let server = ProxyServer::new(config, None);

    let session_id = "sess_destructive_bash";
    let app = server.router();

    let req_body = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Clean up files"}]
    });

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("Content-Type", "application/json")
        .header("x-session-id", session_id)
        .body(Body::from(serde_json::to_vec(&req_body).unwrap()))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    // Verify synthetic invariant error was returned
    let content = body_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap();
    assert!(content.contains("TraceForge Guard: Action blocked due to policy invariant"));
    assert!(content.contains("safety.destructive_bash.rm_rf"));

    // Verify session circuit state was tripped
    assert_eq!(
        server.ws_hub().state_machine().get_state(session_id),
        HitlSessionState::CircuitTripped
    );
}

#[tokio::test]
async fn test_pipeline_loop_detection_recovery_prompt() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    // Upstream returns repeated identical actions
    let (upstream_url, _handle) = spawn_mock_upstream(move |_req| {
        let count = call_count_clone.fetch_add(1, Ordering::SeqCst);
        serde_json::json!({
            "id": format!("chatcmpl-repeat-{}", count),
            "object": "chat.completion",
            "created": 1720000000,
            "model": "gpt-4o",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": format!("call_read_{}", count),
                                "type": "function",
                                "function": {
                                    "name": "read_file",
                                    "arguments": "{\"path\":\"/app/config.json\"}"
                                }
                            }
                        ]
                    },
                    "finish_reason": "tool_calls"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 10,
                "total_tokens": 20
            }
        })
    })
    .await;

    let dummy_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let config = TraceForgeConfig::new(dummy_addr, upstream_url);
    let server = ProxyServer::new(config, None);

    let session_id = "sess_loop_test";

    // 1st request -> normal
    let app1 = server.router();
    let req1 = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("Content-Type", "application/json")
        .header("x-session-id", session_id)
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"model": "gpt-4o", "messages": []})).unwrap(),
        ))
        .unwrap();
    let res1 = app1.oneshot(req1).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);

    // 2nd request -> normal
    let app2 = server.router();
    let req2 = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("Content-Type", "application/json")
        .header("x-session-id", session_id)
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"model": "gpt-4o", "messages": []})).unwrap(),
        ))
        .unwrap();
    let res2 = app2.oneshot(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::OK);

    // 3rd request -> SemanticLoopDetector trips!
    let app3 = server.router();
    let req3 = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("Content-Type", "application/json")
        .header("x-session-id", session_id)
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({"model": "gpt-4o", "messages": []})).unwrap(),
        ))
        .unwrap();
    let res3 = app3.oneshot(req3).await.unwrap();
    assert_eq!(res3.status(), StatusCode::OK);

    let body_bytes = res3.into_body().collect().await.unwrap().to_bytes();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    let content = body_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap();
    assert!(content.contains("CircuitBreaker: Action loop detected with high semantic similarity. You are repeating ineffective tool calls. Stop and change strategy."));
}

#[tokio::test]
async fn test_pipeline_async_trace_flush_to_sqlite() {
    let (upstream_url, _handle) = spawn_mock_upstream(|_req| {
        serde_json::json!({
            "id": "chatcmpl-sqlite-test",
            "object": "chat.completion",
            "created": 1720000000,
            "model": "gpt-4o",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello SQLite persistence!"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 8,
                "completion_tokens": 12,
                "total_tokens": 20
            }
        })
    })
    .await;

    // Initialize in-memory SQLite store and async flusher
    let store = SqliteTraceStore::in_memory().expect("in-memory sqlite store");
    let flusher = AsyncBatchFlusher::new(Arc::new(store.clone()));

    let dummy_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let config = TraceForgeConfig::new(dummy_addr, upstream_url);
    let server = ProxyServer::new(config, Some(flusher.clone()));

    let session_id = "sess_sqlite_flush_verify";
    let app = server.router();

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("Content-Type", "application/json")
        .header("x-session-id", session_id)
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "model": "gpt-4o",
                "messages": [{"role": "user", "content": "Persist me!"}]
            }))
            .unwrap(),
        ))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Flush all queued events to SQLite
    flusher.flush().await.expect("flush events to storage");

    // Retrieve events from SQLite store
    let events = store
        .get_session_events(session_id)
        .await
        .expect("get session events");

    assert!(!events.is_empty(), "Trace events must be persisted in SQLite");
    let has_llm_request = events
        .iter()
        .any(|e| e.kind == traceforge_core::EventKind::LlmRequest);
    assert!(has_llm_request, "Must have LlmRequest event in SQLite");
}

#[tokio::test]
async fn test_pipeline_hitl_approval_breakpoint() {
    let (upstream_url, _handle) = spawn_mock_upstream(|_req| {
        serde_json::json!({
            "id": "chatcmpl-breakpoint-test",
            "object": "chat.completion",
            "created": 1720000000,
            "model": "gpt-4o",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call_deploy_1",
                                "type": "function",
                                "function": {
                                    "name": "deploy_production",
                                    "arguments": "{\"service\":\"payments\"}"
                                }
                            }
                        ]
                    },
                    "finish_reason": "tool_calls"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 10,
                "total_tokens": 20
            }
        })
    })
    .await;

    let dummy_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let config = TraceForgeConfig::new(dummy_addr, upstream_url);
    let server = ProxyServer::new(config, None);

    // Set breakpoint on deploy_production
    server
        .breakpoint_manager()
        .add_breakpoint("deploy_production");

    let session_id = "sess_breakpoint_verify";
    let ws_hub = server.ws_hub().clone();
    let mut rx = ws_hub.subscribe();

    // Background operator that approves the action
    tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let WsOutboundMessage::ApprovalRequired { request } = msg {
                tokio::time::sleep(Duration::from_millis(50)).await;
                ws_hub.resolve_approval(&request.id, ApprovalDecision::Allow);
                break;
            }
        }
    });

    let app = server.router();
    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("Content-Type", "application/json")
        .header("x-session-id", session_id)
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "model": "gpt-4o",
                "messages": [{"role": "user", "content": "Deploy payments"}]
            }))
            .unwrap(),
        ))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    // The tool call was allowed, so tool_calls are present
    let tool_name = &body_json["choices"][0]["message"]["tool_calls"][0]["function"]["name"];
    assert_eq!(tool_name, "deploy_production");
}
