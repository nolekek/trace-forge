use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use traceforge_core::{BudgetConfig, TraceForgeConfig};
use traceforge_guard::budget_limiter::BudgetLimiter;
use traceforge_guard::evaluator::PolicyEvaluator;
use traceforge_guard::loop_detector::SemanticLoopDetector;
use traceforge_guard::policy_types::PolicyLimits;
use traceforge_hitl::{BreakpointManager, SessionStateMachine, WsHub};
use traceforge_proxy::ProxyServer;
use traceforge_storage::{AsyncBatchFlusher, SqliteTraceStore};

#[derive(Parser, Debug)]
#[command(
    name = "traceforge",
    author = "TraceForge Systems Team",
    version = env!("CARGO_PKG_VERSION"),
    about = "Zero-overhead streaming reverse proxy & invariant firewall for autonomous LLM agents",
    long_about = "TraceForge intercepts LLM and tool calls with < 5ms latency SLA, enforcing safety invariants, loop detection, and budget limits."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the TraceForge high-performance streaming proxy server
    Run(RunArgs),

    /// Verify upstream connectivity and proxy configuration
    Check(CheckArgs),
}

#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    /// Proxy listening host address
    #[arg(short = 'H', long, default_value = "0.0.0.0", env = "TRACEFORGE_HOST")]
    pub host: String,

    /// Proxy listening port
    #[arg(short, long, default_value_t = 8080, env = "TRACEFORGE_PORT")]
    pub port: u16,

    /// Target upstream LLM provider URL
    #[arg(
        short,
        long,
        default_value = "https://api.openai.com/v1",
        env = "TRACEFORGE_UPSTREAM"
    )]
    pub upstream: String,

    /// Logging level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info", env = "TRACEFORGE_LOG_LEVEL")]
    pub log_level: String,

    /// Output structured JSON logs rather than human-readable text
    #[arg(long, default_value_t = false, env = "TRACEFORGE_JSON_LOGS")]
    pub json_log: bool,

    /// SQLite telemetry database file path
    #[arg(
        long,
        default_value = ".traceforge/traces.db",
        env = "TRACEFORGE_DB_PATH"
    )]
    pub db_path: String,

    /// Maximum spending budget per session in USD
    #[arg(
        long,
        alias = "max-cost",
        default_value_t = 10.0,
        env = "TRACEFORGE_MAX_BUDGET"
    )]
    pub max_budget: f64,

    /// Maximum cumulative token budget per session
    #[arg(long, default_value_t = 100_000, env = "TRACEFORGE_MAX_TOKENS")]
    pub max_tokens: u32,
}

#[derive(Args, Debug, Clone)]
pub struct CheckArgs {
    /// Target upstream URL to probe
    #[arg(short, long, default_value = "https://api.openai.com/v1")]
    pub upstream: String,
}

fn init_telemetry(log_level: &str, json_format: bool) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("traceforge={log_level},tower_http=info")));

    if json_format {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .with_thread_ids(false)
                    .compact(),
            )
            .init();
    }
}

async fn handle_run(args: RunArgs) -> Result<(), Box<dyn std::error::Error>> {
    let listen_str = format!("{}:{}", args.host, args.port);
    let listen_addr: SocketAddr = listen_str.parse().map_err(|e| {
        format!("Failed to parse host/port '{listen_str}': {e}")
    })?;

    let mut config = TraceForgeConfig::new(listen_addr, &args.upstream);
    config.budgets = BudgetConfig {
        max_cost_usd_per_session: args.max_budget,
        max_tokens_per_session: args.max_tokens,
        ..Default::default()
    };
    config.validate()?;

    // 1. Initialize SQLite Trace Store
    info!("Initializing SQLite telemetry store at: {}", args.db_path);
    let store = SqliteTraceStore::open(&args.db_path)
        .map_err(|e| format!("Failed to initialize SQLite store at '{}': {}", args.db_path, e))?;

    // 2. Initialize Async Batch Telemetry Flusher
    let flusher = AsyncBatchFlusher::new(Arc::new(store));

    // 3. Initialize Guard Pipeline and Control Plane
    let limits = PolicyLimits::new(args.max_budget, args.max_tokens as u64);
    let budget_limiter = Arc::new(BudgetLimiter::new(limits.clone()));
    let policy_evaluator = Arc::new(PolicyEvaluator::new().with_limits(limits));
    let loop_detector = Arc::new(tokio::sync::Mutex::new(SemanticLoopDetector::default()));
    let breakpoint_manager = Arc::new(BreakpointManager::new());
    let state_machine = Arc::new(SessionStateMachine::new());
    let ws_hub = Arc::new(WsHub::new(state_machine, breakpoint_manager.clone()));

    // 4. Initialize Reverse Proxy Server
    let flusher_clone = flusher.clone();
    let server = ProxyServer::new(config, Some(flusher))
        .with_budget_limiter(budget_limiter)
        .with_policy_evaluator(policy_evaluator)
        .with_loop_detector(loop_detector)
        .with_breakpoint_manager(breakpoint_manager)
        .with_ws_hub(ws_hub);

    info!("╔═══════════════════════════════════════════════════════════╗");
    info!("║             🛡️ TraceForge Streaming Guard 🛡️             ║");
    info!("║   Transparent LLM Interceptor & Autonomous Firewall       ║");
    info!("╠═══════════════════════════════════════════════════════════╣");
    info!("║ Listening on: http://{:<36} ║", listen_str);
    info!("║ Upstream:     {:<44} ║", args.upstream);
    info!("║ Telemetry DB: {:<44} ║", args.db_path);
    info!("║ Latency SLA:  < 5ms Interception Overhead                 ║");
    info!("╚═══════════════════════════════════════════════════════════╝");

    server
        .run_with_shutdown(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to listen for Ctrl+C signal");
            info!("Received shutdown signal. Gracefully draining connections...");
            if let Err(e) = flusher_clone.shutdown_and_flush().await {
                error!("Error flushing telemetry during shutdown: {:?}", e);
            }
            info!("All telemetry flushed successfully. Goodbye!");
        })
        .await?;

    Ok(())
}

async fn handle_check(args: CheckArgs) -> Result<(), Box<dyn std::error::Error>> {
    info!("Probing upstream LLM provider at: {}", args.upstream);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let test_url = format!("{}/models", args.upstream.trim_end_matches('/'));
    match client.get(&test_url).send().await {
        Ok(res) => {
            info!("Upstream responded with HTTP status: {}", res.status());
            if res.status().is_success() || res.status() == reqwest::StatusCode::UNAUTHORIZED {
                info!("✓ Upstream reachable and accepting connections.");
            } else {
                warn!("! Upstream reachable but returned unexpected status: {}", res.status());
            }
        }
        Err(e) => {
            error!("✗ Failed to reach upstream at {}: {}", test_url, e);
            return Err(e.into());
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => {
            init_telemetry(&args.log_level, args.json_log);
            if let Err(err) = handle_run(args).await {
                error!("TraceForge execution error: {}", err);
                return ExitCode::FAILURE;
            }
        }
        Commands::Check(args) => {
            init_telemetry("info", false);
            if let Err(err) = handle_check(args).await {
                error!("TraceForge check error: {}", err);
                return ExitCode::FAILURE;
            }
        }
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_run_args_parsing() {
        let args = vec![
            "traceforge",
            "run",
            "--port",
            "9090",
            "--upstream",
            "https://mock.llm/v1",
            "--json-log",
            "--max-cost",
            "25.5",
            "--db-path",
            "/tmp/test.db",
            "--max-tokens",
            "50000",
        ];

        let cli = Cli::try_parse_from(args).expect("parse cli args");
        match cli.command {
            Commands::Run(run_args) => {
                assert_eq!(run_args.port, 9090);
                assert_eq!(run_args.upstream, "https://mock.llm/v1");
                assert!(run_args.json_log);
                assert!((run_args.max_budget - 25.5).abs() < 1e-6);
                assert_eq!(run_args.db_path, "/tmp/test.db");
                assert_eq!(run_args.max_tokens, 50000);
                assert_eq!(run_args.host, "0.0.0.0");
            }
            _ => panic!("Expected Commands::Run"),
        }
    }

    #[test]
    fn test_cli_max_budget_flag() {
        let args = vec![
            "traceforge",
            "run",
            "--max-budget",
            "42.0",
        ];

        let cli = Cli::try_parse_from(args).expect("parse cli args");
        match cli.command {
            Commands::Run(run_args) => {
                assert!((run_args.max_budget - 42.0).abs() < 1e-6);
            }
            _ => panic!("Expected Commands::Run"),
        }
    }

    #[test]
    fn test_cli_check_command_parsing() {
        let args = vec!["traceforge", "check", "--upstream", "https://api.openai.com/v1"];
        let cli = Cli::try_parse_from(args).expect("parse check args");
        match cli.command {
            Commands::Check(check_args) => {
                assert_eq!(check_args.upstream, "https://api.openai.com/v1");
            }
            _ => panic!("Expected Commands::Check"),
        }
    }
}
