use crate::budget_limiter::BudgetLimiter;
use crate::policy_types::{
    EvaluationResult, PolicyInput, PolicyLimits, PolicyViolation,
    ViolationSeverity,
};
use regex::Regex;
use std::sync::LazyLock;
use std::time::Instant;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvaluatorError {
    #[error("Policy input validation error: {0}")]
    ValidationError(String),

    #[error("Wasm serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("WASM runtime execution failed: {0}")]
    WasmExecutionError(String),
}

// ==============================================================================
// Compiled Regular Expressions for High-Throughput Deterministic Evaluation
// ==============================================================================
static RE_RM_CMD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\brm\b").unwrap()
});

static RE_RM_COMBINED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\brm\s+.*(-[a-zA-Z0-9]*r[a-zA-Z0-9]*f|-[a-zA-Z0-9]*f[a-zA-Z0-9]*r|--recursive\s+--force|--force\s+--recursive)").unwrap()
});

static RE_RM_RECURSIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(-[a-zA-Z0-9]*r|--recursive)\b").unwrap()
});

static RE_RM_FORCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(-[a-zA-Z0-9]*f|--force)\b").unwrap()
});

static RE_MKFS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(mkfs(\.[a-zA-Z0-9_-]+)?|mke2fs)\b").unwrap()
});

static RE_DD_WIPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bdd\s+.*(of=/dev/|if=/dev/(zero|urandom))").unwrap()
});

static RE_SHUTDOWN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(shutdown|reboot|poweroff|halt|init\s+[06])\b").unwrap()
});

static RE_CHMOD_SYSTEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bchmod\s+(-[a-zA-Z0-9-]*\s+)*(777|a\+[rwx]*w[rwx]*)\s+(/|/etc|/usr|/bin|/sbin|/lib|/var|/boot|/root|C:[\\/]|C:[\\/]Windows)\b").unwrap()
});

static RE_SQL_DROP_TABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bDROP\s+TABLE\b").unwrap()
});

static RE_SQL_DROP_DB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bDROP\s+(DATABASE|SCHEMA)\b").unwrap()
});

static RE_SQL_TRUNCATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bTRUNCATE(\s+TABLE)?\b").unwrap()
});

static RE_SQL_ALTER_DROP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bALTER\s+TABLE\s+.*DROP\b").unwrap()
});

static RE_PATH_TRAVERSAL_DOTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\.\.[/\\]|[/\\]\.\.)").unwrap()
});

static RE_PATH_SHADOW_PASSWD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(/etc/(shadow|passwd|sudoers|master\.passwd)|/proc/self/environ)").unwrap()
});

static RE_PATH_SSH_KEYS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\.ssh[/\\](id_rsa|id_ed25519|id_ecdsa|authorized_keys|known_hosts)|~[/\\]\.ssh)").unwrap()
});

static RE_PATH_WIN32_SAM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(system32[\\/]config[\\/](SAM|SYSTEM|SECURITY)|ntds\.dit|reg(\.exe)?\s+save\s+(HKLM|HKEY_LOCAL_MACHINE)[\\/](SAM|SYSTEM|SECURITY)|HKLM[\\/]SAM)").unwrap()
});

static RE_GIT_FORCE_PUSH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bgit\s+.*push\s+.*(--force|-f\b|--force-with-lease)").unwrap()
});

static RE_GIT_RESET_HARD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bgit\s+.*reset\s+.*--hard").unwrap()
});

static RE_PROD_DEPLOY_CMD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(deploy|helm\s+upgrade|kubectl\s+apply)\s+.*(--environment=production|--env=prod|-e\s+prod|-e\s+production|prod|production)\b").unwrap()
});

static RE_PROD_DEPLOY_TOOL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(deploy_prod|deploy_production|release_prod|release_production)$").unwrap()
});

static RE_MODIFY_PERM_TOOL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(modify_permissions|grant_role|revoke_role|update_iam_policy|add_user_to_group)$").unwrap()
});

static RE_MODIFY_PERM_CMD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(chmod\s+(\+s|u\+s|g\+s)|chown\s+.*root|setfacl|usermod\s+.*-aG|visudo)\b").unwrap()
});

// ==============================================================================
// Wasmtime Contract Preparation Specification
// ==============================================================================

/// Linear memory payload prepared for passing into a Wasmtime OPA instance.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WasmEvaluationPayload {
    pub entrypoint: String,
    pub input_json: Vec<u8>,
    pub memory_size_bytes: usize,
    pub heap_offset: u32,
    pub data_offset: u32,
}

/// Metadata describing a compiled OPA WebAssembly bundle.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WasmPolicyBundle {
    pub bundle_name: String,
    pub wasm_bytes: Vec<u8>,
    pub exported_memory: String,
    pub exported_eval_fn: String,
}

/// ABI Contract specification for Wasmtime OPA execution engine.
pub trait WasmPolicyContract: Send + Sync {
    /// Prepares serialized input context with layout ready for Wasm memory write.
    fn prepare_payload(&self, input: &PolicyInput) -> Result<WasmEvaluationPayload, EvaluatorError>;

    /// Executes the WASM policy evaluation against the guest memory.
    fn evaluate_wasm(&self, payload: &WasmEvaluationPayload) -> Result<EvaluationResult, EvaluatorError>;
}

/// Mock contract implementation verifying Wasm ABI layout without requiring precompiled OPA binaries.
pub struct WasmtimeContractAdapter {
    bundle_name: String,
}

impl WasmtimeContractAdapter {
    pub fn new(bundle_name: impl Into<String>) -> Self {
        Self {
            bundle_name: bundle_name.into(),
        }
    }
}

impl WasmPolicyContract for WasmtimeContractAdapter {
    fn prepare_payload(&self, input: &PolicyInput) -> Result<WasmEvaluationPayload, EvaluatorError> {
        let json_bytes = serde_json::to_vec(input)?;
        let data_len = json_bytes.len();
        // 64KB page alignment for WebAssembly linear memory
        let page_size = 64 * 1024;
        let pages_needed = (data_len + page_size - 1) / page_size + 1;

        Ok(WasmEvaluationPayload {
            entrypoint: format!("traceforge/{}/allow", self.bundle_name),
            input_json: json_bytes,
            memory_size_bytes: pages_needed * page_size,
            heap_offset: 1024,
            data_offset: 2048,
        })
    }

    fn evaluate_wasm(&self, payload: &WasmEvaluationPayload) -> Result<EvaluationResult, EvaluatorError> {
        let input: PolicyInput = serde_json::from_slice(&payload.input_json)?;
        let evaluator = PolicyEvaluator::new();
        evaluator.evaluate(&input)
    }
}

// ==============================================================================
// Policy Evaluator Implementation
// ==============================================================================

/// High-performance deterministic policy evaluator.
/// Executes safety invariants, budget validation, and HITL detection.
#[derive(Clone)]
pub struct PolicyEvaluator {
    budget_limiter: BudgetLimiter,
    default_limits: PolicyLimits,
    wasm_adapter: Option<std::sync::Arc<dyn WasmPolicyContract>>,
}

impl Default for PolicyEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyEvaluator {
    pub fn new() -> Self {
        let default_limits = PolicyLimits::default();
        Self {
            budget_limiter: BudgetLimiter::new(default_limits.clone()),
            default_limits,
            wasm_adapter: Some(std::sync::Arc::new(WasmtimeContractAdapter::new("safety"))),
        }
    }

    pub fn with_limits(mut self, limits: PolicyLimits) -> Self {
        self.default_limits = limits.clone();
        self.budget_limiter = BudgetLimiter::new(limits);
        self
    }

    pub fn with_wasm_adapter(mut self, adapter: std::sync::Arc<dyn WasmPolicyContract>) -> Self {
        self.wasm_adapter = Some(adapter);
        self
    }

    pub fn budget_limiter(&self) -> &BudgetLimiter {
        &self.budget_limiter
    }

    /// Prepares a Wasmtime execution contract payload for external WASM runtime execution.
    pub fn prepare_wasm_contract(&self, input: &PolicyInput) -> Result<WasmEvaluationPayload, EvaluatorError> {
        if let Some(adapter) = &self.wasm_adapter {
            adapter.prepare_payload(input)
        } else {
            let adapter = WasmtimeContractAdapter::new("safety");
            adapter.prepare_payload(input)
        }
    }

    /// Evaluates the input against all security invariants, budget constraints, and HITL triggers.
    pub fn evaluate(&self, input: &PolicyInput) -> Result<EvaluationResult, EvaluatorError> {
        let start_time = Instant::now();

        // 1. Input Validation
        self.validate_input(input)?;

        let mut violations = Vec::new();
        let mut hitl_reasons = Vec::new();

        // 2. Evaluate Safety Invariants
        self.evaluate_safety(input, &mut violations);

        // 3. Evaluate Budget Invariants
        self.evaluate_budget(input, &mut violations);

        // 4. Evaluate HITL Invariants
        self.evaluate_hitl(input, &mut hitl_reasons);

        let latency_us = start_time.elapsed().as_micros() as u64;

        if !violations.is_empty() {
            Ok(EvaluationResult::deny(violations, latency_us))
        } else if !hitl_reasons.is_empty() {
            Ok(EvaluationResult::require_hitl(hitl_reasons, latency_us))
        } else {
            Ok(EvaluationResult::allow(latency_us))
        }
    }

    fn validate_input(&self, input: &PolicyInput) -> Result<(), EvaluatorError> {
        if input.session_id.trim().is_empty() {
            return Err(EvaluatorError::ValidationError("session_id cannot be empty".to_string()));
        }
        if input.accumulated_cost_usd < 0.0 {
            return Err(EvaluatorError::ValidationError("accumulated_cost_usd cannot be negative".to_string()));
        }
        Ok(())
    }

    fn extract_command_str<'a>(&self, input: &'a PolicyInput) -> &'a str {
        if let Some(cmd) = input.tool_args.get("command").and_then(|v| v.as_str()) {
            return cmd;
        }
        if let Some(cmd) = input.tool_args.get("cmd").and_then(|v| v.as_str()) {
            return cmd;
        }
        if let Some(cmd) = input.tool_args.get("script").and_then(|v| v.as_str()) {
            return cmd;
        }
        ""
    }

    fn extract_sql_str<'a>(&self, input: &'a PolicyInput) -> &'a str {
        if let Some(sql) = input.tool_args.get("query").and_then(|v| v.as_str()) {
            return sql;
        }
        if let Some(sql) = input.tool_args.get("sql").and_then(|v| v.as_str()) {
            return sql;
        }
        if let Some(sql) = input.tool_args.get("statement").and_then(|v| v.as_str()) {
            return sql;
        }
        self.extract_command_str(input)
    }

    fn evaluate_safety(&self, input: &PolicyInput, violations: &mut Vec<PolicyViolation>) {
        let cmd = self.extract_command_str(input);
        let sql = self.extract_sql_str(input);

        // A. Destructive Bash Commands
        if !cmd.is_empty() {
            let is_destructive_rm = RE_RM_COMBINED.is_match(cmd)
                || (RE_RM_CMD.is_match(cmd)
                    && RE_RM_RECURSIVE.is_match(cmd)
                    && RE_RM_FORCE.is_match(cmd));

            if is_destructive_rm {
                violations.push(
                    PolicyViolation::new(
                        "safety.destructive_bash.rm_rf",
                        ViolationSeverity::Critical,
                        format!("Prohibited destructive recursive deletion command: {cmd}"),
                    )
                    .with_target(cmd),
                );
            }

            if RE_MKFS.is_match(cmd) {
                violations.push(
                    PolicyViolation::new(
                        "safety.destructive_bash.mkfs",
                        ViolationSeverity::Critical,
                        format!("Prohibited filesystem creation/format command: {cmd}"),
                    )
                    .with_target(cmd),
                );
            }

            if RE_DD_WIPE.is_match(cmd) {
                violations.push(
                    PolicyViolation::new(
                        "safety.destructive_bash.dd_wipe",
                        ViolationSeverity::Critical,
                        format!("Prohibited raw device write or disk wipe command: {cmd}"),
                    )
                    .with_target(cmd),
                );
            }

            if RE_SHUTDOWN.is_match(cmd) {
                violations.push(
                    PolicyViolation::new(
                        "safety.destructive_bash.system_shutdown",
                        ViolationSeverity::Critical,
                        format!("Prohibited host shutdown, reboot or halt command: {cmd}"),
                    )
                    .with_target(cmd),
                );
            }

            let compact_cmd = cmd.replace(' ', "");
            if compact_cmd.contains(":(){:|:&};:") {
                violations.push(
                    PolicyViolation::new(
                        "safety.destructive_bash.fork_bomb",
                        ViolationSeverity::Critical,
                        "Prohibited shell fork bomb detected",
                    )
                    .with_target(cmd),
                );
            }

            if RE_CHMOD_SYSTEM.is_match(cmd) {
                violations.push(
                    PolicyViolation::new(
                        "safety.system_modification.chmod_system_path",
                        ViolationSeverity::Critical,
                        format!("Prohibited unsafe permission modification on critical system path: {cmd}"),
                    )
                    .with_target(cmd),
                );
            }
        }

        // B. Destructive SQL
        if !sql.is_empty() {
            if RE_SQL_DROP_TABLE.is_match(sql) {
                violations.push(
                    PolicyViolation::new(
                        "safety.destructive_sql.drop_table",
                        ViolationSeverity::Critical,
                        format!("Prohibited DROP TABLE execution: {sql}"),
                    )
                    .with_target(sql),
                );
            }

            if RE_SQL_DROP_DB.is_match(sql) {
                violations.push(
                    PolicyViolation::new(
                        "safety.destructive_sql.drop_database",
                        ViolationSeverity::Critical,
                        format!("Prohibited DROP DATABASE/SCHEMA execution: {sql}"),
                    )
                    .with_target(sql),
                );
            }

            if RE_SQL_TRUNCATE.is_match(sql) {
                violations.push(
                    PolicyViolation::new(
                        "safety.destructive_sql.truncate",
                        ViolationSeverity::Critical,
                        format!("Prohibited TRUNCATE execution: {sql}"),
                    )
                    .with_target(sql),
                );
            }

            if RE_SQL_ALTER_DROP.is_match(sql) {
                violations.push(
                    PolicyViolation::new(
                        "safety.destructive_sql.alter_drop",
                        ViolationSeverity::High,
                        format!("Prohibited ALTER TABLE DROP execution: {sql}"),
                    )
                    .with_target(sql),
                );
            }
        }

        // C. Path Traversal & Sensitive Files
        self.check_path_string(cmd, "command", violations);

        if let serde_json::Value::Object(map) = &input.tool_args {
            for (key, val) in map {
                if let Some(s) = val.as_str() {
                    self.check_path_string(s, key, violations);
                }
            }
        }
    }

    fn check_path_string(&self, s: &str, field_name: &str, violations: &mut Vec<PolicyViolation>) {
        if s.is_empty() {
            return;
        }

        if RE_PATH_TRAVERSAL_DOTS.is_match(s) {
            violations.push(
                PolicyViolation::new(
                    "safety.path_traversal.directory_escape",
                    ViolationSeverity::Critical,
                    format!("Path traversal attempt detected in '{field_name}': {s}"),
                )
                .with_target(s),
            );
        }

        if RE_PATH_SHADOW_PASSWD.is_match(s) {
            violations.push(
                PolicyViolation::new(
                    "safety.path_traversal.sensitive_unix_credential",
                    ViolationSeverity::Critical,
                    format!("Access to sensitive Unix credential file detected in '{field_name}': {s}"),
                )
                .with_target(s),
            );
        }

        if RE_PATH_SSH_KEYS.is_match(s) {
            violations.push(
                PolicyViolation::new(
                    "safety.path_traversal.ssh_key",
                    ViolationSeverity::Critical,
                    format!("Access to SSH private keys/configuration detected in '{field_name}': {s}"),
                )
                .with_target(s),
            );
        }

        if RE_PATH_WIN32_SAM.is_match(s) {
            violations.push(
                PolicyViolation::new(
                    "safety.path_traversal.win32_sam",
                    ViolationSeverity::Critical,
                    format!("Access to Win32 SAM/SYSTEM registry hive detected in '{field_name}': {s}"),
                )
                .with_target(s),
            );
        }
    }

    fn evaluate_budget(&self, input: &PolicyInput, violations: &mut Vec<PolicyViolation>) {
        let (max_budget_usd, max_tokens) = if let Some(limits) = &input.limits {
            (limits.max_budget_usd, limits.max_tokens_per_session)
        } else {
            (self.default_limits.max_budget_usd, self.default_limits.max_tokens_per_session)
        };

        if input.accumulated_cost_usd > max_budget_usd {
            violations.push(PolicyViolation::new(
                "budget.cost_exceeded",
                ViolationSeverity::Critical,
                format!(
                    "Accumulated session cost ${:.4} exceeds maximum budget limit of ${:.4}",
                    input.accumulated_cost_usd, max_budget_usd
                ),
            ));
        }

        if input.token_count > max_tokens {
            violations.push(PolicyViolation::new(
                "budget.tokens_exceeded",
                ViolationSeverity::High,
                format!(
                    "Accumulated session tokens {} exceeds maximum limit of {} tokens",
                    input.token_count, max_tokens
                ),
            ));
        }
    }

    fn evaluate_hitl(&self, input: &PolicyInput, hitl_reasons: &mut Vec<String>) {
        let cmd = self.extract_command_str(input);
        let tool = &input.tool_name;

        // 1. Git Force Push
        if !cmd.is_empty() && RE_GIT_FORCE_PUSH.is_match(cmd) {
            hitl_reasons.push(
                "Force push can rewrite remote repository history and discard unmerged commits".to_string(),
            );
        }

        // 2. Git Hard Reset
        if !cmd.is_empty() && RE_GIT_RESET_HARD.is_match(cmd) {
            hitl_reasons.push(
                "Hard reset permanently discards all uncommitted working tree files and staged changes".to_string(),
            );
        }

        // 3. Production Deployment
        let is_prod_tool = RE_PROD_DEPLOY_TOOL.is_match(tool);
        let is_prod_arg = input
            .tool_args
            .get("environment")
            .or_else(|| input.tool_args.get("env"))
            .and_then(|v| v.as_str())
            .map(|env| env.eq_ignore_ascii_case("production") || env.eq_ignore_ascii_case("prod"))
            .unwrap_or(false);
        let is_prod_cmd = !cmd.is_empty() && RE_PROD_DEPLOY_CMD.is_match(cmd);

        if is_prod_tool || is_prod_arg || is_prod_cmd {
            hitl_reasons.push(
                "Production deployment alters live user-facing workloads and requires operator verification".to_string(),
            );
        }

        // 4. Modify Permissions
        let is_perm_tool = RE_MODIFY_PERM_TOOL.is_match(tool);
        let is_perm_cmd = !cmd.is_empty() && RE_MODIFY_PERM_CMD.is_match(cmd);

        if is_perm_tool || is_perm_cmd {
            hitl_reasons.push(
                "Modifying permissions, IAM roles, or ACLs changes security boundaries".to_string(),
            );
        }
    }
}
