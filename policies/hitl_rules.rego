package traceforge.hitl

import future.keywords.contains
import future.keywords.if
import future.keywords.in

default require_hitl := false

# ==============================================================================
# Metadata
# ==============================================================================
metadata := {
    "name": "traceforge.hitl_rules",
    "version": "1.0.0",
    "description": "Human-in-the-Loop approval triggers for high-impact or irreversible operations"
}

# Require HITL if any trigger condition matches
require_hitl := true if {
    count(hitl_triggers) > 0
}

# ==============================================================================
# Helpers
# ==============================================================================
command_str := cmd if {
    cmd := input.tool_args.command
    is_string(cmd)
} else := cmd if {
    cmd := input.tool_args.cmd
    is_string(cmd)
} else := ""

tool_name := input.tool_name if {
    input.tool_name != null
} else := ""

# ==============================================================================
# HITL Triggers
# ==============================================================================

# 1. Git Force Push
hitl_triggers contains {
    "rule_id": "hitl.git_force_push",
    "severity": "HIGH",
    "operation": "git_push_force",
    "reason": "Force push can rewrite remote repository history and discard unmerged commits"
} if {
    command_str != ""
    regex.match("(?i)\\bgit\\s+.*push\\s+.*(--force|-f\\b|--force-with-lease)", command_str)
}

# 2. Git Hard Reset
hitl_triggers contains {
    "rule_id": "hitl.git_reset_hard",
    "severity": "HIGH",
    "operation": "git_reset_hard",
    "reason": "Hard reset permanently discards all uncommitted working tree files and staged changes"
} if {
    command_str != ""
    regex.match("(?i)\\bgit\\s+.*reset\\s+.*--hard", command_str)
}

# 3. Production Deployment
hitl_triggers contains {
    "rule_id": "hitl.deploy_production",
    "severity": "CRITICAL",
    "operation": "deploy_production",
    "reason": "Production deployment alters live user-facing workloads and requires operator verification"
} if {
    is_prod_tool
}

hitl_triggers contains {
    "rule_id": "hitl.deploy_production",
    "severity": "CRITICAL",
    "operation": "deploy_production",
    "reason": "Production deployment alters live user-facing workloads and requires operator verification"
} if {
    is_prod_command
}

is_prod_tool if {
    regex.match("(?i)^(deploy_prod|deploy_production|release_prod|release_production)$", tool_name)
}

is_prod_tool if {
    input.tool_args.environment == "production"
} else if {
    input.tool_args.env == "production"
} else if {
    input.tool_args.env == "prod"
}

is_prod_command if {
    command_str != ""
    regex.match("(?i)\\b(deploy|helm\\s+upgrade|kubectl\\s+apply)\\s+.*(--environment=production|--env=prod|-e\\s+prod|-e\\s+production|prod|production)\\b", command_str)
}

# 4. Modify Permissions / Privilege Escalation
hitl_triggers contains {
    "rule_id": "hitl.modify_permissions",
    "severity": "HIGH",
    "operation": "modify_permissions",
    "reason": "Modifying permissions, IAM roles, or ACLs changes security boundaries"
} if {
    is_permission_tool
}

hitl_triggers contains {
    "rule_id": "hitl.modify_permissions",
    "severity": "HIGH",
    "operation": "modify_permissions",
    "reason": "Modifying permissions, IAM roles, or ACLs changes security boundaries"
} if {
    is_permission_command
}

is_permission_tool if {
    regex.match("(?i)^(modify_permissions|grant_role|revoke_role|update_iam_policy|add_user_to_group)$", tool_name)
}

is_permission_command if {
    command_str != ""
    regex.match("(?i)\\b(chmod\\s+(\\+s|u\\+s|g\\+s)|chown\\s+.*root|setfacl|usermod\\s+.*-aG|visudo)\\b", command_str)
}
