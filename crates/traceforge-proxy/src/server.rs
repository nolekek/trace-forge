use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

use traceforge_core::TraceForgeConfig;
use traceforge_guard::budget_limiter::BudgetLimiter;
use traceforge_guard::evaluator::PolicyEvaluator;
use traceforge_guard::loop_detector::SemanticLoopDetector;
use traceforge_guard::policy_types::PolicyLimits;
use traceforge_hitl::{BreakpointManager, SessionStateMachine, WsHub};
use traceforge_storage::AsyncBatchFlusher;

use crate::http_proxy::{
    chat_completions_handler, fallback_proxy_handler, health_handler, mcp_route_handler,
    ws_route_handler, ProxyState,
};
use crate::mcp_interceptor::McpInterceptor;

/// High-performance reverse proxy HTTP server with safety invariant firewall and HITL control plane.
pub struct ProxyServer {
    config: TraceForgeConfig,
    flusher: Option<AsyncBatchFlusher>,
    budget_limiter: Arc<BudgetLimiter>,
    policy_evaluator: Arc<PolicyEvaluator>,
    loop_detector: Arc<tokio::sync::Mutex<SemanticLoopDetector>>,
    ws_hub: Arc<WsHub>,
    breakpoint_manager: Arc<BreakpointManager>,
}

impl ProxyServer {
    /// Creates a new `ProxyServer` with default invariant engines and control plane.
    pub fn new(config: TraceForgeConfig, flusher: Option<AsyncBatchFlusher>) -> Self {
        let limits = PolicyLimits::new(
            config.budgets.max_cost_usd_per_session,
            config.budgets.max_tokens_per_session as u64,
        );

        let budget_limiter = Arc::new(BudgetLimiter::new(limits.clone()));
        let policy_evaluator = Arc::new(PolicyEvaluator::new().with_limits(limits));
        let loop_detector = Arc::new(tokio::sync::Mutex::new(SemanticLoopDetector::default()));
        let breakpoint_manager = Arc::new(BreakpointManager::new());
        let ws_hub = Arc::new(WsHub::new(
            Arc::new(SessionStateMachine::new()),
            breakpoint_manager.clone(),
        ));

        Self {
            config,
            flusher,
            budget_limiter,
            policy_evaluator,
            loop_detector,
            ws_hub,
            breakpoint_manager,
        }
    }

    pub fn with_flusher(mut self, flusher: AsyncBatchFlusher) -> Self {
        self.flusher = Some(flusher);
        self
    }

    pub fn with_budget_limiter(mut self, limiter: Arc<BudgetLimiter>) -> Self {
        self.budget_limiter = limiter;
        self
    }

    pub fn with_policy_evaluator(mut self, evaluator: Arc<PolicyEvaluator>) -> Self {
        self.policy_evaluator = evaluator;
        self
    }

    pub fn with_loop_detector(
        mut self,
        detector: Arc<tokio::sync::Mutex<SemanticLoopDetector>>,
    ) -> Self {
        self.loop_detector = detector;
        self
    }

    pub fn with_ws_hub(mut self, ws_hub: Arc<WsHub>) -> Self {
        self.ws_hub = ws_hub;
        self
    }

    pub fn with_breakpoint_manager(mut self, bpm: Arc<BreakpointManager>) -> Self {
        self.breakpoint_manager = bpm;
        self
    }

    pub fn ws_hub(&self) -> &Arc<WsHub> {
        &self.ws_hub
    }

    pub fn breakpoint_manager(&self) -> &Arc<BreakpointManager> {
        &self.breakpoint_manager
    }

    pub fn budget_limiter(&self) -> &Arc<BudgetLimiter> {
        &self.budget_limiter
    }

    pub fn policy_evaluator(&self) -> &Arc<PolicyEvaluator> {
        &self.policy_evaluator
    }

    pub fn loop_detector(&self) -> &Arc<tokio::sync::Mutex<SemanticLoopDetector>> {
        &self.loop_detector
    }

    pub fn flusher(&self) -> Option<&AsyncBatchFlusher> {
        self.flusher.as_ref()
    }

    pub fn mcp_interceptor(&self) -> McpInterceptor {
        McpInterceptor::new(
            self.policy_evaluator.clone(),
            self.breakpoint_manager.clone(),
        )
    }

    /// Construct Axum router configured for reverse proxy, WebSocket control plane, and health endpoints.
    pub fn router(&self) -> Router {
        let state = ProxyState::new(
            self.config.clone(),
            self.flusher.clone(),
            Some(self.budget_limiter.clone()),
            Some(self.policy_evaluator.clone()),
            Some(self.loop_detector.clone()),
            Some(self.ws_hub.clone()),
            Some(self.breakpoint_manager.clone()),
        );

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);

        Router::new()
            .route("/v1/chat/completions", post(chat_completions_handler))
            .route("/health", get(health_handler))
            .route("/ws/control", get(ws_route_handler))
            .route("/mcp", post(mcp_route_handler))
            .route("/dashboard", get(traceforge_ui::serve_static_asset))
            .route("/dashboard/", get(traceforge_ui::serve_static_asset))
            .route("/dashboard/{*path}", get(traceforge_ui::serve_static_asset))
            .route("/_next/{*path}", get(traceforge_ui::serve_static_asset))
            .route("/", get(traceforge_ui::serve_static_asset))
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
