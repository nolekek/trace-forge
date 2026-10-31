use crate::policy_types::PolicyLimits;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// Conversion factor between USD dollars and integer micro-dollars ($1.00 = 1,000,000 µUSD).
const USD_TO_MICROS: f64 = 1_000_000.0;

#[inline]
pub fn usd_to_micros(usd: f64) -> u64 {
    if usd <= 0.0 {
        0
    } else {
        (usd * USD_TO_MICROS).round() as u64
    }
}

#[inline]
pub fn micros_to_usd(micros: u64) -> f64 {
    (micros as f64) / USD_TO_MICROS
}

/// Errors raised when a budget or token quota is breached.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum BudgetExceededError {
    #[error("Cost limit exceeded: accumulated ${current_usd:.4} > max limit ${limit_usd:.4}")]
    CostExceeded { current_usd: f64, limit_usd: f64 },

    #[error("Token quota exceeded: accumulated {current_tokens} > max limit {limit_tokens} tokens")]
    TokensExceeded { current_tokens: u64, limit_tokens: u64 },

    #[error("Both cost and token quotas exceeded: cost ${current_usd:.4}/${limit_usd:.4}, tokens {current_tokens}/{limit_tokens}")]
    BothExceeded {
        current_usd: f64,
        limit_usd: f64,
        current_tokens: u64,
        limit_tokens: u64,
    },
}

/// Point-in-time immutable snapshot of session budget metrics.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BudgetSnapshot {
    pub session_id: String,
    pub accumulated_tokens: u64,
    pub accumulated_cost_usd: f64,
    pub max_tokens: u64,
    pub max_budget_usd: f64,
    pub is_exceeded: bool,
    pub remaining_tokens: u64,
    pub remaining_budget_usd: f64,
}

/// Lock-free atomic tracker for an individual session.
pub struct SessionBudgetTracker {
    pub session_id: String,
    accumulated_tokens: AtomicU64,
    accumulated_cost_micros: AtomicU64,
    max_tokens: AtomicU64,
    max_cost_micros: AtomicU64,
}

impl SessionBudgetTracker {
    pub fn new(session_id: impl Into<String>, limits: &PolicyLimits) -> Self {
        Self {
            session_id: session_id.into(),
            accumulated_tokens: AtomicU64::new(0),
            accumulated_cost_micros: AtomicU64::new(0),
            max_tokens: AtomicU64::new(limits.max_tokens_per_session),
            max_cost_micros: AtomicU64::new(usd_to_micros(limits.max_budget_usd)),
        }
    }

    /// Atomically records token and cost consumption.
    pub fn record_usage(&self, tokens: u64, cost_usd: f64) -> Result<BudgetSnapshot, BudgetExceededError> {
        let cost_micros = usd_to_micros(cost_usd);
        let prev_tokens = self.accumulated_tokens.fetch_add(tokens, Ordering::SeqCst);
        let prev_micros = self.accumulated_cost_micros.fetch_add(cost_micros, Ordering::SeqCst);

        let new_tokens = prev_tokens + tokens;
        let new_micros = prev_micros + cost_micros;

        self.evaluate_snapshot(new_tokens, new_micros)
    }

    /// Checks whether the session has already exceeded its configured limits.
    pub fn check_limits(&self) -> Result<BudgetSnapshot, BudgetExceededError> {
        let tokens = self.accumulated_tokens.load(Ordering::SeqCst);
        let micros = self.accumulated_cost_micros.load(Ordering::SeqCst);
        self.evaluate_snapshot(tokens, micros)
    }

    /// Generates a current snapshot without failing if limits are exceeded.
    pub fn snapshot(&self) -> BudgetSnapshot {
        let tokens = self.accumulated_tokens.load(Ordering::SeqCst);
        let micros = self.accumulated_cost_micros.load(Ordering::SeqCst);
        let max_tok = self.max_tokens.load(Ordering::SeqCst);
        let max_mic = self.max_cost_micros.load(Ordering::SeqCst);

        let cost_usd = micros_to_usd(micros);
        let max_usd = micros_to_usd(max_mic);
        let is_exceeded = tokens > max_tok || micros > max_mic;

        let rem_tokens = max_tok.saturating_sub(tokens);
        let rem_usd = if max_usd > cost_usd {
            max_usd - cost_usd
        } else {
            0.0
        };

        BudgetSnapshot {
            session_id: self.session_id.clone(),
            accumulated_tokens: tokens,
            accumulated_cost_usd: cost_usd,
            max_tokens: max_tok,
            max_budget_usd: max_usd,
            is_exceeded,
            remaining_tokens: rem_tokens,
            remaining_budget_usd: rem_usd,
        }
    }

    /// Updates the configured limits for this session.
    pub fn update_limits(&self, limits: &PolicyLimits) {
        self.max_tokens.store(limits.max_tokens_per_session, Ordering::SeqCst);
        self.max_cost_micros.store(usd_to_micros(limits.max_budget_usd), Ordering::SeqCst);
    }

    /// Resets all accumulated usage metrics to zero.
    pub fn reset(&self) {
        self.accumulated_tokens.store(0, Ordering::SeqCst);
        self.accumulated_cost_micros.store(0, Ordering::SeqCst);
    }

    fn evaluate_snapshot(&self, tokens: u64, micros: u64) -> Result<BudgetSnapshot, BudgetExceededError> {
        let max_tok = self.max_tokens.load(Ordering::SeqCst);
        let max_mic = self.max_cost_micros.load(Ordering::SeqCst);

        let cost_usd = micros_to_usd(micros);
        let max_usd = micros_to_usd(max_mic);

        let cost_exceeded = micros > max_mic;
        let tokens_exceeded = tokens > max_tok;

        let rem_tokens = max_tok.saturating_sub(tokens);
        let rem_usd = if max_usd > cost_usd {
            max_usd - cost_usd
        } else {
            0.0
        };

        let snapshot = BudgetSnapshot {
            session_id: self.session_id.clone(),
            accumulated_tokens: tokens,
            accumulated_cost_usd: cost_usd,
            max_tokens: max_tok,
            max_budget_usd: max_usd,
            is_exceeded: cost_exceeded || tokens_exceeded,
            remaining_tokens: rem_tokens,
            remaining_budget_usd: rem_usd,
        };

        if cost_exceeded && tokens_exceeded {
            Err(BudgetExceededError::BothExceeded {
                current_usd: cost_usd,
                limit_usd: max_usd,
                current_tokens: tokens,
                limit_tokens: max_tok,
            })
        } else if cost_exceeded {
            Err(BudgetExceededError::CostExceeded {
                current_usd: cost_usd,
                limit_usd: max_usd,
            })
        } else if tokens_exceeded {
            Err(BudgetExceededError::TokensExceeded {
                current_tokens: tokens,
                limit_tokens: max_tok,
            })
        } else {
            Ok(snapshot)
        }
    }
}

/// Thread-safe, in-memory multi-session budget limiter.
/// Coordinates session trackers with sub-microsecond atomic operations.
#[derive(Clone)]
pub struct BudgetLimiter {
    default_limits: PolicyLimits,
    sessions: Arc<RwLock<HashMap<String, Arc<SessionBudgetTracker>>>>,
}

impl Default for BudgetLimiter {
    fn default() -> Self {
        Self::new(PolicyLimits::default())
    }
}

impl BudgetLimiter {
    pub fn new(default_limits: PolicyLimits) -> Self {
        Self {
            default_limits,
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Retrieves an existing session tracker or creates a new one atomically.
    pub fn get_or_create_session(&self, session_id: &str) -> Arc<SessionBudgetTracker> {
        {
            let read_guard = self.sessions.read().unwrap();
            if let Some(tracker) = read_guard.get(session_id) {
                return Arc::clone(tracker);
            }
        }

        let mut write_guard = self.sessions.write().unwrap();
        write_guard
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SessionBudgetTracker::new(session_id, &self.default_limits)))
            .clone()
    }

    /// Records usage for a session and checks whether quotas were exceeded.
    pub fn record_usage(
        &self,
        session_id: &str,
        tokens: u64,
        cost_usd: f64,
    ) -> Result<BudgetSnapshot, BudgetExceededError> {
        let tracker = self.get_or_create_session(session_id);
        tracker.record_usage(tokens, cost_usd)
    }

    /// Verifies current limits for a session.
    pub fn check_limits(&self, session_id: &str) -> Result<BudgetSnapshot, BudgetExceededError> {
        let tracker = self.get_or_create_session(session_id);
        tracker.check_limits()
    }

    /// Returns the current statistics for a session if it exists.
    pub fn get_session_stats(&self, session_id: &str) -> Option<BudgetSnapshot> {
        let read_guard = self.sessions.read().unwrap();
        read_guard.get(session_id).map(|tracker| tracker.snapshot())
    }

    /// Sets custom limits for a specific session.
    pub fn set_session_limits(&self, session_id: &str, limits: PolicyLimits) {
        let tracker = self.get_or_create_session(session_id);
        tracker.update_limits(&limits);
    }

    /// Resets usage metrics for a specific session.
    pub fn reset_session(&self, session_id: &str) {
        let read_guard = self.sessions.read().unwrap();
        if let Some(tracker) = read_guard.get(session_id) {
            tracker.reset();
        }
    }

    /// Clears all tracked sessions from memory.
    pub fn clear_all(&self) {
        let mut write_guard = self.sessions.write().unwrap();
        write_guard.clear();
    }
}
