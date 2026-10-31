//! # TraceForge Proxy
//!
//! Sub-millisecond latency reverse proxy for LLM endpoints with zero-copy SSE stream teeing.

pub mod http_proxy;
pub mod server;
pub mod stream_tee;

pub use http_proxy::{ProxyMetrics, ProxyState};
pub use server::ProxyServer;
pub use stream_tee::{StreamAccumulator, StreamSummary, StreamTee};

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use traceforge_core::TraceForgeConfig;

    #[tokio::test]
    async fn test_health_endpoint() {
        let config = TraceForgeConfig::default();
        let server = ProxyServer::new(config, None);
        let app = server.router();

        let req = Request::builder()
            .uri("/health")
            .method("GET")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["service"], "traceforge-proxy");
    }

    #[tokio::test]
    async fn test_invalid_json_chat_completions() {
        let config = TraceForgeConfig::default();
        let server = ProxyServer::new(config, None);
        let app = server.router();

        let req = Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(Body::from("invalid json {"))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"]["message"].as_str().unwrap().contains("Invalid JSON"));
    }
}
