//! # UI Delivery and MCP Protocol Interceptor Integration Tests
//!
//! Validates:
//! 1. Embedded dashboard UI delivery via rust-embed (200 OK, HTML, Next.js assets).
//! 2. MCP JSON-RPC 2.0 protocol interception:
//!    - Destructive tool execution blocked by Invariant Firewall (code -32000).
//!    - Safe tool execution allowed.
//!    - Breakpoints triggering execution freeze (AwaitingApproval).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use traceforge_core::TraceForgeConfig;
use traceforge_proxy::mcp_interceptor::{
    inspect_mcp_request, McpInspectionResult, MCP_BLOCKED_ERROR_CODE, MCP_BLOCKED_MESSAGE,
};
use traceforge_proxy::ProxyServer;

#[tokio::test]
async fn test_dashboard_embedded_html_served() {
    let config = TraceForgeConfig::default();
    let server = ProxyServer::new(config, None);
    let app = server.router();

    // 1. GET /dashboard
    let req = Request::builder()
        .uri("/dashboard")
        .method("GET")
        .body(Body::empty())
        .expect("Valid HTTP request");

    let response = app.oneshot(req).await.expect("App should respond");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Dashboard route must return 200 OK"
    );

    let content_type = response
        .headers()
        .get("content-type")
        .expect("Content-Type header present")
        .to_str()
        .expect("Valid UTF-8 header");
    assert!(
        content_type.contains("text/html"),
        "Content-Type must be text/html, got: {content_type}"
    );

    let body_bytes = response
        .into_body()
        .collect()
        .await
        .expect("Read response body")
        .to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);

    assert!(
        body_str.contains("<!DOCTYPE html>"),
        "Dashboard body must contain <!DOCTYPE html>"
    );
    assert!(
        body_str.contains("<title>TraceForge"),
        "Dashboard body must contain '<title>TraceForge'"
    );
}

#[tokio::test]
async fn test_dashboard_subpath_and_assets_served() {
    let config = TraceForgeConfig::default();
    let server = ProxyServer::new(config, None);
    let app = server.router();

    // Test GET /dashboard/ (with trailing slash)
    let req = Request::builder()
        .uri("/dashboard/")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Test GET /_next/static/css/adb090c985d1da0e.css
    let req = Request::builder()
        .uri("/_next/static/css/adb090c985d1da0e.css")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ct = response.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.contains("text/css"), "Must be text/css, got: {ct}");

    // Test SPA fallback for subpath
    let req = Request::builder()
        .uri("/dashboard/react-flow-canvas")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("<title>TraceForge"));
}

#[tokio::test]
async fn test_mcp_destructive_tool_call_blocked() {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": "req-destructive-01",
        "method": "tools/call",
        "params": {
            "name": "execute_bash",
            "arguments": {
                "command": "rm -rf /"
            }
        }
    });

    let raw_json = payload.to_string();

    // 1. Direct McpInterceptor inspection
    let result = inspect_mcp_request(&raw_json);
    assert!(
        result.is_blocked(),
        "Destructive tool execution must be blocked"
    );

    match result {
        McpInspectionResult::Blocked {
            error_response,
            rule_id,
            message,
        } => {
            assert_eq!(error_response["jsonrpc"], "2.0");
            assert_eq!(error_response["id"], "req-destructive-01");
            assert_eq!(
                error_response["error"]["code"], MCP_BLOCKED_ERROR_CODE,
                "JSON-RPC error code must be -32000"
            );
            assert_eq!(
                error_response["error"]["message"], MCP_BLOCKED_MESSAGE,
                "Error message must match specification"
            );
            assert!(
                rule_id.contains("safety"),
                "Rule ID must reference safety invariants"
            );
            assert_eq!(message, MCP_BLOCKED_MESSAGE);
        }
        other => panic!("Expected McpInspectionResult::Blocked, got {:?}", other),
    }

    // 2. HTTP POST /mcp route interception through Axum ProxyServer
    let config = TraceForgeConfig::default();
    let server = ProxyServer::new(config, None);
    let app = server.router();

    let req = Request::builder()
        .uri("/mcp")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(raw_json))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp_json: Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(resp_json["jsonrpc"], "2.0");
    assert_eq!(resp_json["id"], "req-destructive-01");
    assert_eq!(resp_json["error"]["code"], -32000);
    assert_eq!(
        resp_json["error"]["message"],
        "Action blocked by TraceForge Invariant Firewall"
    );
}

#[tokio::test]
async fn test_mcp_safe_tool_call_allowed() {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": "req-safe-02",
        "method": "tools/call",
        "params": {
            "name": "read_file",
            "arguments": {
                "path": "src/lib.rs"
            }
        }
    });

    let raw_json = payload.to_string();

    // 1. Direct McpInterceptor inspection
    let result = inspect_mcp_request(&raw_json);
    assert!(
        result.is_allowed(),
        "Safe tool execution must be allowed by invariant firewall"
    );

    match result {
        McpInspectionResult::Allowed {
            method,
            tool_name,
            params,
        } => {
            assert_eq!(method, "tools/call");
            assert_eq!(tool_name.as_deref(), Some("read_file"));
            assert!(params.is_some());
        }
        other => panic!("Expected McpInspectionResult::Allowed, got {:?}", other),
    }

    // 2. HTTP POST /mcp route inspection
    let config = TraceForgeConfig::default();
    let server = ProxyServer::new(config, None);
    let app = server.router();

    let req = Request::builder()
        .uri("/mcp")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(raw_json))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let resp_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(resp_json["result"]["status"], "allowed");
    assert_eq!(resp_json["result"]["tool"], "read_file");
}

#[tokio::test]
async fn test_mcp_breakpoint_triggers_awaiting_approval() {
    let server = ProxyServer::new(TraceForgeConfig::default(), None);

    // Set breakpoint on tool "read_file"
    server.breakpoint_manager().add_breakpoint("read_file");

    let interceptor = server.mcp_interceptor();

    let payload = json!({
        "jsonrpc": "2.0",
        "id": "req-bp-01",
        "method": "tools/call",
        "params": {
            "name": "read_file",
            "arguments": {
                "path": "README.md"
            }
        }
    });

    let result = interceptor.inspect_mcp_request(&payload.to_string());
    assert!(
        result.is_awaiting_approval(),
        "Tool call matching breakpoint must yield AwaitingApproval"
    );

    match result {
        McpInspectionResult::AwaitingApproval {
            tool_name,
            reason,
            ..
        } => {
            assert_eq!(tool_name, "read_file");
            assert!(reason.contains("breakpoint active"));
        }
        other => panic!("Expected AwaitingApproval, got {:?}", other),
    }
}

#[tokio::test]
async fn test_mcp_non_tool_methods_pass_through() {
    // Non-tool MCP methods like "initialize" or "tools/list" should be allowed
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });

    let result = inspect_mcp_request(&payload.to_string());
    assert!(result.is_allowed());
    match result {
        McpInspectionResult::Allowed {
            method, tool_name, ..
        } => {
            assert_eq!(method, "tools/list");
            assert!(tool_name.is_none());
        }
        other => panic!("Expected Allowed, got {:?}", other),
    }
}
