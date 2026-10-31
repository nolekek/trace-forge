use serde_json::json;
use std::sync::Arc;
use std::thread;
use traceforge_guard::{
    BudgetExceededError, BudgetLimiter, EvaluationDecision, PolicyEvaluator, PolicyInput,
    PolicyLimits, ViolationSeverity, WasmPolicyContract, WasmtimeContractAdapter,
};

#[test]
fn test_safe_bash_command_allowed() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_001", "execute_bash")
        .with_tool_args(json!({ "command": "ls -la /var/log/app" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Allow);
    assert!(result.violations.is_empty());
    assert!(result.hitl_reasons.is_empty());
}

#[test]
fn test_safe_sql_query_allowed() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_002", "execute_sql")
        .with_tool_args(json!({ "query": "SELECT id, name, created_at FROM users WHERE status = 'active'" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Allow);
    assert!(result.violations.is_empty());
}

#[test]
fn test_destructive_bash_rm_rf_root_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_003", "execute_bash")
        .with_tool_args(json!({ "command": "rm -rf /" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.destructive_bash.rm_rf"));
    assert_eq!(result.violations[0].severity, ViolationSeverity::Critical);
}

#[test]
fn test_destructive_bash_rm_rf_variants_denied() {
    let evaluator = PolicyEvaluator::new();
    let variations = [
        "rm -fr /var/lib/data",
        "rm -r -f /var/cache",
        "rm --recursive --force /tmp/test",
        "rm -rf *",
    ];

    for cmd in variations {
        let input = PolicyInput::new("sess_test", "execute_bash")
            .with_tool_args(json!({ "command": cmd }));
        let result = evaluator.evaluate(&input).expect("evaluation succeeded");
        assert_eq!(
            result.decision,
            EvaluationDecision::Deny,
            "Command should be denied: {cmd}"
        );
        assert!(
            result.violations.iter().any(|v| v.rule_id == "safety.destructive_bash.rm_rf"),
            "Command failed to match rule: {cmd}"
        );
    }
}

#[test]
fn test_destructive_bash_mkfs_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_004", "execute_bash")
        .with_tool_args(json!({ "command": "mkfs.ext4 /dev/nvme0n1p1" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.destructive_bash.mkfs"));
}

#[test]
fn test_destructive_bash_dd_wipe_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_005", "execute_bash")
        .with_tool_args(json!({ "command": "dd if=/dev/zero of=/dev/sda bs=1M count=1000" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.destructive_bash.dd_wipe"));
}

#[test]
fn test_destructive_bash_system_halt_denied() {
    let evaluator = PolicyEvaluator::new();
    for cmd in ["shutdown -h now", "reboot", "init 0", "poweroff"] {
        let input = PolicyInput::new("sess_006", "execute_bash")
            .with_tool_args(json!({ "command": cmd }));

        let result = evaluator.evaluate(&input).expect("evaluation succeeded");
        assert_eq!(result.decision, EvaluationDecision::Deny);
        assert!(result.violations.iter().any(|v| v.rule_id == "safety.destructive_bash.system_shutdown"));
    }
}

#[test]
fn test_destructive_bash_fork_bomb_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_007", "execute_bash")
        .with_tool_args(json!({ "command": ":(){ :|:& };:" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.destructive_bash.fork_bomb"));
}

#[test]
fn test_destructive_bash_chmod_777_system_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_008", "execute_bash")
        .with_tool_args(json!({ "command": "chmod -R 777 /etc" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.system_modification.chmod_system_path"));
}

#[test]
fn test_destructive_sql_drop_table_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_009", "execute_sql")
        .with_tool_args(json!({ "query": "DROP TABLE accounts;" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.destructive_sql.drop_table"));
}

#[test]
fn test_destructive_sql_drop_database_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_010", "execute_sql")
        .with_tool_args(json!({ "query": "DROP DATABASE production;" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.destructive_sql.drop_database"));
}

#[test]
fn test_destructive_sql_truncate_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_011", "execute_sql")
        .with_tool_args(json!({ "query": "TRUNCATE TABLE audit_trail;" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.destructive_sql.truncate"));
}

#[test]
fn test_path_traversal_etc_shadow_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_012", "read_file")
        .with_tool_args(json!({ "path": "/etc/shadow" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.path_traversal.sensitive_unix_credential"));
}

#[test]
fn test_path_traversal_ssh_key_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_013", "execute_bash")
        .with_tool_args(json!({ "command": "cat ~/.ssh/id_rsa" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.path_traversal.ssh_key"));
}

#[test]
fn test_path_traversal_win32_sam_registry_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_014", "execute_bash")
        .with_tool_args(json!({ "command": "reg save HKLM\\SAM C:\\temp\\sam.save" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.path_traversal.win32_sam"));
}

#[test]
fn test_path_traversal_dot_dot_escape_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_015", "view_file")
        .with_tool_args(json!({ "file_path": "../../../../etc/passwd" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "safety.path_traversal.directory_escape"));
}

#[test]
fn test_budget_exceeded_cost_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_016", "execute_bash")
        .with_tool_args(json!({ "command": "echo safe" }))
        .with_accumulated_cost(10.50) // Default max budget is $5.00
        .with_token_count(1000);

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "budget.cost_exceeded"));
}

#[test]
fn test_budget_exceeded_tokens_denied() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_017", "execute_bash")
        .with_tool_args(json!({ "command": "echo safe" }))
        .with_accumulated_cost(1.00)
        .with_token_count(150_000); // Default max is 100,000

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Deny);
    assert!(result.violations.iter().any(|v| v.rule_id == "budget.tokens_exceeded"));
}

#[test]
fn test_budget_custom_limits_respected() {
    let evaluator = PolicyEvaluator::new();
    let limits = PolicyLimits::new(50.0, 500_000);
    let input = PolicyInput::new("sess_018", "execute_bash")
        .with_limits(limits)
        .with_accumulated_cost(25.0)
        .with_token_count(200_000)
        .with_tool_args(json!({ "command": "echo 'within custom limits'" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Allow);
}

#[test]
fn test_budget_limiter_atomic_concurrency() {
    let limiter = Arc::new(BudgetLimiter::new(PolicyLimits::new(100.0, 1_000_000)));
    let session_id = "concurrent_sess_01";
    let num_threads = 10;
    let iterations_per_thread = 100;

    let mut handles = Vec::new();
    for _ in 0..num_threads {
        let lim = Arc::clone(&limiter);
        handles.push(thread::spawn(move || {
            for _ in 0..iterations_per_thread {
                lim.record_usage(session_id, 10, 0.001).unwrap();
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let stats = limiter.get_session_stats(session_id).expect("stats exist");
    let expected_tokens = num_threads as u64 * iterations_per_thread as u64 * 10;
    let expected_cost = num_threads as f64 * iterations_per_thread as f64 * 0.001;

    assert_eq!(stats.accumulated_tokens, expected_tokens);
    assert!((stats.accumulated_cost_usd - expected_cost).abs() < 1e-6);
    assert!(!stats.is_exceeded);
}

#[test]
fn test_budget_limiter_breach_detection() {
    let limiter = BudgetLimiter::new(PolicyLimits::new(1.0, 1000));
    let sess = "breach_test";

    // 1st step: within limits
    let snap1 = limiter.record_usage(sess, 500, 0.50).unwrap();
    assert_eq!(snap1.accumulated_tokens, 500);

    // 2nd step: breaches cost ($1.20 > $1.00)
    let res = limiter.record_usage(sess, 100, 0.70);
    match res {
        Err(BudgetExceededError::CostExceeded { current_usd, limit_usd }) => {
            assert!((current_usd - 1.20).abs() < 1e-6);
            assert_eq!(limit_usd, 1.0);
        }
        other => panic!("Expected CostExceeded, got {other:?}"),
    }
}

#[test]
fn test_hitl_git_push_force() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_019", "execute_bash")
        .with_tool_args(json!({ "command": "git push --force origin main" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::RequireHitl);
    assert!(result.hitl_reasons.iter().any(|r| r.contains("Force push")));
}

#[test]
fn test_hitl_git_reset_hard() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_020", "execute_bash")
        .with_tool_args(json!({ "command": "git reset --hard HEAD~1" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::RequireHitl);
    assert!(result.hitl_reasons.iter().any(|r| r.contains("Hard reset")));
}

#[test]
fn test_hitl_deploy_production() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_021", "deploy_production")
        .with_tool_args(json!({ "version": "v2.4.0" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::RequireHitl);
    assert!(result.hitl_reasons.iter().any(|r| r.contains("Production deployment")));
}

#[test]
fn test_hitl_production_env_argument() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_022", "trigger_pipeline")
        .with_tool_args(json!({ "pipeline": "deploy", "environment": "production" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::RequireHitl);
}

#[test]
fn test_hitl_modify_permissions() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_023", "modify_permissions")
        .with_tool_args(json!({ "role": "admin", "target_user": "external_agent" }));

    let result = evaluator.evaluate(&input).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::RequireHitl);
    assert!(result.hitl_reasons.iter().any(|r| r.contains("Modifying permissions")));
}

#[test]
fn test_wasm_contract_preparation_and_payload_layout() {
    let adapter = WasmtimeContractAdapter::new("safety");
    let input = PolicyInput::new("sess_wasm", "execute_bash")
        .with_tool_args(json!({ "command": "echo 'payload check'" }));

    let payload = adapter.prepare_payload(&input).expect("payload prepared");
    assert_eq!(payload.entrypoint, "traceforge/safety/allow");
    assert!(!payload.input_json.is_empty());
    assert_eq!(payload.memory_size_bytes % (64 * 1024), 0, "must be page aligned");
    assert_eq!(payload.heap_offset, 1024);
    assert_eq!(payload.data_offset, 2048);

    // Round-trip evaluation through contract
    let result = adapter.evaluate_wasm(&payload).expect("evaluation succeeded");
    assert_eq!(result.decision, EvaluationDecision::Allow);
}

#[test]
fn test_validation_empty_session_id() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("  ", "execute_bash");
    let res = evaluator.evaluate(&input);
    assert!(res.is_err());
}

#[test]
fn test_validation_negative_cost() {
    let evaluator = PolicyEvaluator::new();
    let input = PolicyInput::new("sess_val", "execute_bash").with_accumulated_cost(-5.0);
    let res = evaluator.evaluate(&input);
    assert!(res.is_err());
}
