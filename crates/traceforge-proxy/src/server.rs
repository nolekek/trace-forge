use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::mpsc::Sender;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

use traceforge_core::{TraceEvent, TraceForgeConfig};

use crate::http_proxy::{
    chat_completions_handler, fallback_proxy_handler, health_handler, ProxyState,
};

/// High-performance reverse proxy HTTP server.
pub struct ProxyServer {
    config: TraceForgeConfig,
    event_tx: Option<Sender<TraceEvent>>,
}

impl ProxyServer {
    pub fn new(config: TraceForgeConfig, event_tx: Option<Sender<TraceEvent>>) -> Self {
        Self { config, event_tx }
    }

    /// Construct Axum router configured for reverse proxy and health endpoints.
    pub fn router(&self) -> Router {
        let state = ProxyState::new(self.config.clone(), self.event_tx.clone());

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);

        Router::new()
            .route("/v1/chat/completions", post(chat_completions_handler))
            .route("/health", get(health_handler))
            .fallback(fallback_proxy_handler)
            .layer(cors)
            .layer(TraceLayer::new_for_http())
            .with_state(state)
    }

    /// Run the server binding to `config.listen_addr`.
    pub async fn run(self) -> Result<(), std::io::Error> {
        let addr = self.config.listen_addr;
        let app = self.router();

        info!("Starting TraceForge Streaming Reverse Proxy on http://{}", addr);
        info!("Targeting upstream LLM provider at: {}", self.config.upstream_url);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, app).await
    }

    /// Run the server with graceful shutdown signal.
    pub async fn run_with_shutdown<F>(self, shutdown_signal: F) -> Result<(), std::io::Error>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let addr = self.config.listen_addr;
        let app = self.router();

        info!("Starting TraceForge Streaming Reverse Proxy on http://{}", addr);
        info!("Targeting upstream LLM provider at: {}", self.config.upstream_url);

        let listener = TcpListener::bind(addr).await?;
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal)
            .await
    }
}
