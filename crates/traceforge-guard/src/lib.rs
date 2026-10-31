//! # TraceForge Guard
//!
//! Real-Time Invariant Firewall, Policy Evaluator and Atomic Budget Limiter
//! for Autonomous LLM Agents.
//!
//! Sub-millisecond policy engine enforcing deterministic invariants:
//! - Destructive shell commands (`rm -rf`, `mkfs`, `dd`, `shutdown`, `reboot`, fork bombs)
//! - Destructive SQL statements (`DROP TABLE`, `DROP DATABASE`, `TRUNCATE`)
//! - Path traversal attacks (`../`, `/etc/shadow`, `~/.ssh/id_rsa`, Win32 SAM/SYSTEM)
//! - Atomic financial and token quotas with lock-free counters
//! - Human-in-the-Loop (HITL) approval triggers (`git push --force`, `git reset --hard`, `deploy_production`, `modify_permissions`)
//! - ABI contract layout preparation for Wasmtime OPA bundles

pub mod budget_limiter;
pub mod evaluator;
pub mod policy_types;

// Flatten primary exports
pub use budget_limiter::{
    micros_to_usd, usd_to_micros, BudgetExceededError, BudgetLimiter, BudgetSnapshot,
    SessionBudgetTracker,
};
pub use evaluator::{
    EvaluatorError, PolicyEvaluator, WasmEvaluationPayload, WasmPolicyBundle,
    WasmPolicyContract, WasmtimeContractAdapter,
};
pub use policy_types::{
    AgentMetadata, EvaluationDecision, EvaluationResult, PolicyInput, PolicyLimits,
    PolicyViolation, ViolationSeverity,
};
