use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use traceforge_core::{BudgetConfig, EventKind, TraceEvent, TraceForgeConfig};
use traceforge_proxy::ProxyServer;

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

    /// Maximum spending budget per session in USD
    #[arg(long, default_value_t = 10.0, env = "TRACEFORGE_MAX_COST")]
    pub max_cost: f64,

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
        max_cost_usd_per_session: args.max_cost,
        max_tokens_per_session: args.max_tokens,
        ..Default::default()
    };
    config.validate()?;

    // Create async telemetry event channel
    let (event_tx, mut event_rx) = mpsc::channel::<TraceEvent>(2048);

    // Spawn telemetry processor worker
    tokio::spawn(async move {
        while let Some(evt) = event_rx.recv().await {
            match evt.kind {
                EventKind::ToolCall => {
                    info!(
                        session = %evt.session_id,
                        tool = %evt.payload["tool_name"],
                        args = %evt.payload["arguments"],
                        "🚨 Intercepted Tool Call"
                    );
                }
                EventKind::LlmRequest => {
                    info!(
                        session = %evt.session_id,
                        latency_us = evt.latency_us,
                        cost_usd = evt.cost_usd,
                        tokens = ?evt.token_usage.map(|u| u.total_tokens),
                        "📡 LLM Request Completed"
                    );
                }
                EventKind::LlmChunk => {
                    info!(
                        session = %evt.session_id,
                        latency_us = evt.latency_us,
                        tokens = ?evt.token_usage.map(|u| u.total_tokens),
                        cost_usd = evt.cost_usd,
                        chunks = ?evt.payload["chunks_count"],
                        "⚡ SSE Stream Finished"
                    );
                }
                _ => {
                    info!(
                        session = %evt.session_id,
                        kind = ?evt.kind,
                        "Telemetry event received"
                    );
                }
            }
        }
    });

    let server = ProxyServer::new(config, Some(event_tx));

    info!("╔═══════════════════════════════════════════════════════════╗");
    info!("║             🛡️ TraceForge Streaming Guard 🛡️             ║");
    info!("║   Transparent LLM Interceptor & Autonomous Firewall       ║");
    info!("╠═══════════════════════════════════════════════════════════╣");
    info!("║ Listening on: http://{:<36} ║", listen_str);
    info!("║ Upstream:     {:<44} ║", args.upstream);
    info!("║ Latency SLA:  < 5ms Interception Overhead                 ║");
    info!("╚═══════════════════════════════════════════════════════════╝");

    server
        .run_with_shutdown(async {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to listen for Ctrl+C signal");
            info!("Received shutdown signal. Gracefully draining connections...");
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
        ];

        let cli = Cli::try_parse_from(args).expect("parse cli args");
        match cli.command {
            Commands::Run(run_args) => {
                assert_eq!(run_args.port, 9090);
                assert_eq!(run_args.upstream, "https://mock.llm/v1");
                assert!(run_args.json_log);
                assert!((run_args.max_cost - 25.5).abs() < 1e-6);
                assert_eq!(run_args.host, "0.0.0.0");
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
