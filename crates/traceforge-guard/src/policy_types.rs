use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Decision outcome of evaluating policies for an agent step or tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationDecision {
    /// Action conforms to all security and operational invariants.
    Allow,
    /// Action is strictly prohibited by security or budget invariants.
    Deny,
    /// Action is paused and queued for Human-in-the-Loop review and manual approval.
    RequireHitl,
}

impl EvaluationDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, Self::Deny)
    }

    pub fn is_hitl_required(&self) -> bool {
        matches!(self, Self::RequireHitl)
    }
}

/// Severity classification for policy violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ViolationSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Detailed violation record produced during policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyViolation {
    pub rule_id: String,
    pub severity: ViolationSeverity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl PolicyViolation {
    pub fn new(
        rule_id: impl Into<String>,
        severity: ViolationSeverity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            severity,
            message: message.into(),
            target: None,
            details: None,
        }
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Limits configured or requested for evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyLimits {
    pub max_budget_usd: f64,
    pub max_tokens_per_session: u64,
}

impl Default for PolicyLimits {
    fn default() -> Self {
        Self {
            max_budget_usd: 5.0,
            max_tokens_per_session: 100_000,
        }
    }
}

impl PolicyLimits {
    pub fn new(max_budget_usd: f64, max_tokens_per_session: u64) -> Self {
        Self {
            max_budget_usd,
            max_tokens_per_session,
        }
    }
}

/// Agent role and authorization metadata included in policy input.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMetadata {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub permission_level: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl AgentMetadata {
    pub fn new(role: impl Into<String>, permission_level: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            permission_level: permission_level.into(),
            extra: HashMap::new(),
        }
    }
}

/// Canonical input context evaluated against policies.
/// Matches Section 5.1 JSON Input format from architecture_design.md.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyInput {
    #[serde(default)]
    pub timestamp: u64,
    pub session_id: String,
    #[serde(default)]
    pub step_number: u64,
    #[serde(default)]
    pub accumulated_cost_usd: f64,
    #[serde(default)]
    pub token_count: u64,
    pub tool_name: String,
    #[serde(default)]
    pub tool_args: serde_json::Value,
    #[serde(default)]
    pub semantic_loop_score: f64,
    #[serde(default)]
    pub agent_metadata: AgentMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<PolicyLimits>,
}

impl PolicyInput {
    pub fn new(session_id: impl Into<String>, tool_name: impl Into<String>) -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            session_id: session_id.into(),
            step_number: 1,
            accumulated_cost_usd: 0.0,
            token_count: 0,
            tool_name: tool_name.into(),
            tool_args: serde_json::Value::Object(serde_json::Map::new()),
            semantic_loop_score: 0.0,
            agent_metadata: AgentMetadata::default(),
            limits: None,
        }
    }

    pub fn with_step_number(mut self, step_number: u64) -> Self {
        self.step_number = step_number;
        self
    }

    pub fn with_tool_args(mut self, args: serde_json::Value) -> Self {
        self.tool_args = args;
        self
    }

    pub fn with_accumulated_cost(mut self, cost_usd: f64) -> Self {
        self.accumulated_cost_usd = cost_usd;
        self
    }

    pub fn with_token_count(mut self, token_count: u64) -> Self {
        self.token_count = token_count;
        self
    }

    pub fn with_metadata(mut self, role: impl Into<String>, permission_level: impl Into<String>) -> Self {
        self.agent_metadata = AgentMetadata::new(role, permission_level);
        self
    }

    pub fn with_limits(mut self, limits: PolicyLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    pub fn with_loop_score(mut self, score: f64) -> Self {
        self.semantic_loop_score = score;
        self
    }
}

/// Comprehensive outcome returned by policy evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationResult {
    pub decision: EvaluationDecision,
    pub violations: Vec<PolicyViolation>,
    pub hitl_reasons: Vec<String>,
    pub evaluation_latency_us: u64,
}

impl EvaluationResult {
    pub fn allow(latency_us: u64) -> Self {
        Self {
            decision: EvaluationDecision::Allow,
            violations: Vec::new(),
            hitl_reasons: Vec::new(),
            evaluation_latency_us: latency_us,
        }
    }

    pub fn deny(violations: Vec<PolicyViolation>, latency_us: u64) -> Self {
        Self {
            decision: EvaluationDecision::Deny,
            violations,
            hitl_reasons: Vec::new(),
            evaluation_latency_us: latency_us,
        }
    }

    pub fn require_hitl(reasons: Vec<String>, latency_us: u64) -> Self {
        Self {
            decision: EvaluationDecision::RequireHitl,
            violations: Vec::new(),
            hitl_reasons: reasons,
            evaluation_latency_us: latency_us,
        }
    }

    #[cfg(feature = "traceforge-core")]
    pub fn into_core_policy_result(self) -> traceforge_core::types::PolicyResult {
        match self.decision {
            EvaluationDecision::Allow => traceforge_core::types::PolicyResult::allow(),
            EvaluationDecision::Deny => {
                let first_rule = self.violations.first().map(|v| v.rule_id.clone());
                let first_msg = self.violations.first().map(|v| v.message.clone());
                let violation_msgs = self.violations.into_iter().map(|v| v.message).collect();
                traceforge_core::types::PolicyResult {
                    allow: false,
                    rule_name: first_rule,
                    reason: first_msg,
                    require_hitl: false,
                    violations: violation_msgs,
                }
            }
            EvaluationDecision::RequireHitl => {
                let first_reason = self.hitl_reasons.first().cloned();
                traceforge_core::types::PolicyResult {
                    allow: false,
                    rule_name: Some("hitl.required".to_string()),
                    reason: first_reason,
                    require_hitl: true,
                    violations: self.hitl_reasons,
                }
            }
        }
    }
}
