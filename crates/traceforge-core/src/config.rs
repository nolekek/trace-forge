use std::net::SocketAddr;
use serde::{Deserialize, Serialize};

use crate::errors::{CoreError, Result};

/// Global configuration for the TraceForge proxy runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceForgeConfig {
    /// Proxy listening address and port (e.g., 0.0.0.0:8080)
    pub listen_addr: SocketAddr,
    /// Upstream LLM provider base URL (e.g., https://api.openai.com/v1)
    pub upstream_url: String,
    /// Strict latency thresholds and SLA bounds
    pub latency_thresholds: LatencyThresholds,
    /// Token and cost budget constraints per session
    pub budgets: BudgetConfig,
    /// Logging and observability output configuration
    pub logging: LoggingConfig,
}

impl Default for TraceForgeConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:8080".parse().expect("valid socket addr"),
            upstream_url: "https://api.openai.com/v1".to_string(),
            latency_thresholds: LatencyThresholds::default(),
            budgets: BudgetConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

impl TraceForgeConfig {
    pub fn new(listen_addr: SocketAddr, upstream_url: impl Into<String>) -> Self {
        Self {
            listen_addr,
            upstream_url: upstream_url.into(),
            ..Default::default()
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.upstream_url.is_empty() {
            return Err(CoreError::Validation("upstream_url cannot be empty".into()));
        }
        if !self.upstream_url.starts_with("http://") && !self.upstream_url.starts_with("https://") {
            return Err(CoreError::Validation(
                "upstream_url must begin with http:// or https://".into(),
            ));
        }
        if self.latency_thresholds.max_overhead_ms == 0 {
            return Err(CoreError::Validation(
                "max_overhead_ms must be greater than 0".into(),
            ));
        }
        Ok(())
    }

    /// Read configuration overrides from standard environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(addr_str) = std::env::var("TRACEFORGE_LISTEN_ADDR") {
            if let Ok(addr) = addr_str.parse() {
                config.listen_addr = addr;
            }
        }

        if let Ok(upstream) = std::env::var("TRACEFORGE_UPSTREAM_URL") {
            config.upstream_url = upstream;
        }

        if let Ok(log_level) = std::env::var("TRACEFORGE_LOG_LEVEL") {
            config.logging.level = log_level;
        }

        config
    }
}

/// Latency monitoring thresholds in milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyThresholds {
    /// Latency warning threshold for full requests (default: 5000ms)
    pub warn_threshold_ms: u64,
    /// Time To First Token target maximum (default: 1500ms)
    pub max_ttft_ms: u64,
    /// Strict proxy interception overhead bound (< 5ms SLA)
    pub max_overhead_ms: u64,
}

impl Default for LatencyThresholds {
    fn default() -> Self {
        Self {
            warn_threshold_ms: 5000,
            max_ttft_ms: 1500,
            max_overhead_ms: 5,
        }
    }
}

/// Budget limitations applied across agent execution steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Max cumulative dollar spend permitted per agent session
    pub max_cost_usd_per_session: f64,
    /// Max cumulative tokens permitted per agent session
    pub max_tokens_per_session: u32,
    /// Dollar cost per 1,000 prompt tokens
    pub prompt_cost_per_1k: f64,
    /// Dollar cost per 1,000 completion tokens
    pub completion_cost_per_1k: f64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_cost_usd_per_session: 10.0,
            max_tokens_per_session: 100_000,
            prompt_cost_per_1k: 0.0015,
            completion_cost_per_1k: 0.002,
        }
    }
}

impl BudgetConfig {
    /// Calculate estimated USD cost from token usage counts.
    pub fn calculate_cost(&self, prompt_tokens: u32, completion_tokens: u32) -> f64 {
        let prompt_cost = (prompt_tokens as f64 / 1000.0) * self.prompt_cost_per_1k;
        let completion_cost = (completion_tokens as f64 / 1000.0) * self.completion_cost_per_1k;
        prompt_cost + completion_cost
    }
}

/// Logging and structured diagnostics setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub json_format: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            json_format: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_valid() {
        let config = TraceForgeConfig::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.latency_thresholds.max_overhead_ms, 5);
        assert_eq!(config.budgets.max_tokens_per_session, 100_000);
    }

    #[test]
    fn test_invalid_upstream_fails_validation() {
        let mut config = TraceForgeConfig::default();
        config.upstream_url = "invalid-url".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_cost_calculation() {
        let budget = BudgetConfig {
            prompt_cost_per_1k: 0.001,
            completion_cost_per_1k: 0.002,
            ..Default::default()
        };
        let cost = budget.calculate_cost(2000, 1000);
        // 2 * 0.001 + 1 * 0.002 = 0.004
        assert!((cost - 0.004).abs() < 1e-6);
    }
}
