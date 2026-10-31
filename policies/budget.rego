package traceforge.budget

import future.keywords.contains
import future.keywords.if
import future.keywords.in

default allow := true

# ==============================================================================
# Metadata & Default Thresholds
# ==============================================================================
metadata := {
    "name": "traceforge.budget",
    "version": "1.0.0",
    "description": "Financial and token quota invariant firewall preventing budget exhaustion"
}

default_max_budget_usd := 5.0
default_max_tokens_per_session := 100000

# Effective limits: use limits specified in input context or fallback to defaults
effective_max_budget_usd := input.limits.max_budget_usd if {
    input.limits.max_budget_usd != null
} else := input.max_budget_usd if {
    input.max_budget_usd != null
} else := default_max_budget_usd

effective_max_tokens := input.limits.max_tokens_per_session if {
    input.limits.max_tokens_per_session != null
} else := input.max_tokens_per_session if {
    input.max_tokens_per_session != null
} else := default_max_tokens_per_session

# Effective current accumulated usage
current_cost_usd := input.accumulated_cost_usd if {
    input.accumulated_cost_usd != null
} else := 0.0

current_tokens := input.token_count if {
    input.token_count != null
} else := input.total_tokens if {
    input.total_tokens != null
} else := 0

# Deny if any budget invariant is violated
allow := false if {
    count(violations) > 0
}

# ==============================================================================
# Budget Invariant Rules
# ==============================================================================
violations contains {
    "rule_id": "budget.cost_exceeded",
    "severity": "CRITICAL",
    "message": sprintf("Accumulated session cost $%.4f exceeds maximum budget limit of $%.4f", [current_cost_usd, effective_max_budget_usd]),
    "current_value": current_cost_usd,
    "limit_value": effective_max_budget_usd
} if {
    current_cost_usd > effective_max_budget_usd
}

violations contains {
    "rule_id": "budget.tokens_exceeded",
    "severity": "HIGH",
    "message": sprintf("Accumulated session tokens %v exceeds maximum limit of %v tokens", [current_tokens, effective_max_tokens]),
    "current_value": current_tokens,
    "limit_value": effective_max_tokens
} if {
    current_tokens > effective_max_tokens
}
