package traceforge.safety

import future.keywords.contains
import future.keywords.if
import future.keywords.in

default allow := true

# ==============================================================================
# Metadata
# ==============================================================================
metadata := {
    "name": "traceforge.safety",
    "version": "1.0.0",
    "description": "Deterministic safety invariants: prevents destructive OS/SQL actions, unauthorized modifications, and path traversal"
}

# The policy denies execution if any safety violation is detected
allow := false if {
    count(violations) > 0
}

# ==============================================================================
# Violations Aggregation
# ==============================================================================
violations contains violation if {
    some violation in destructive_bash_violations
}

violations contains violation if {
    some violation in destructive_sql_violations
}

violations contains violation if {
    some violation in path_traversal_violations
}

violations contains violation if {
    some violation in system_modification_violations
}

# ==============================================================================
# Command & Argument Extraction Helpers
# ==============================================================================
command_str := cmd if {
    cmd := input.tool_args.command
    is_string(cmd)
} else := cmd if {
    cmd := input.tool_args.cmd
    is_string(cmd)
} else := cmd if {
    cmd := input.tool_args.script
    is_string(cmd)
} else := ""

sql_str := sql if {
    sql := input.tool_args.query
    is_string(sql)
} else := sql if {
    sql := input.tool_args.sql
    is_string(sql)
} else := sql if {
    sql := input.tool_args.statement
    is_string(sql)
} else := command_str

# ==============================================================================
# 1. Destructive Bash / Shell Invariants
# ==============================================================================
destructive_bash_violations contains {
    "rule_id": "safety.destructive_bash.rm_rf",
    "severity": "CRITICAL",
    "message": sprintf("Prohibited destructive recursive deletion command: %v", [command_str]),
    "target": command_str
} if {
    command_str != ""
    is_destructive_rm(command_str)
}

is_destructive_rm(cmd) if {
    regex.match("(?i)\\brm\\s+.*(-[a-zA-Z0-9]*r[a-zA-Z0-9]*f|-[a-zA-Z0-9]*f[a-zA-Z0-9]*r|--recursive\\s+--force|--force\\s+--recursive)", cmd)
}

is_destructive_rm(cmd) if {
    regex.match("(?i)\\brm\\b", cmd)
    regex.match("(?i)(-[a-zA-Z0-9]*r|--recursive)\\b", cmd)
    regex.match("(?i)(-[a-zA-Z0-9]*f|--force)\\b", cmd)
}

destructive_bash_violations contains {
    "rule_id": "safety.destructive_bash.mkfs",
    "severity": "CRITICAL",
    "message": sprintf("Prohibited filesystem creation/format command: %v", [command_str]),
    "target": command_str
} if {
    command_str != ""
    regex.match("(?i)\\b(mkfs(\\.[a-zA-Z0-9_-]+)?|mke2fs)\\b", command_str)
}

destructive_bash_violations contains {
    "rule_id": "safety.destructive_bash.dd_wipe",
    "severity": "CRITICAL",
    "message": sprintf("Prohibited raw device write or disk wipe command: %v", [command_str]),
    "target": command_str
} if {
    command_str != ""
    regex.match("(?i)\\bdd\\s+.*(of=/dev/|if=/dev/(zero|urandom))", command_str)
}

destructive_bash_violations contains {
    "rule_id": "safety.destructive_bash.system_shutdown",
    "severity": "CRITICAL",
    "message": sprintf("Prohibited host shutdown, reboot or halt command: %v", [command_str]),
    "target": command_str
} if {
    command_str != ""
    regex.match("(?i)\\b(shutdown|reboot|poweroff|halt|init\\s+[06])\\b", command_str)
}

destructive_bash_violations contains {
    "rule_id": "safety.destructive_bash.fork_bomb",
    "severity": "CRITICAL",
    "message": "Prohibited shell fork bomb detected",
    "target": command_str
} if {
    command_str != ""
    contains(replace(command_str, " ", ""), ":(){:|:&};:")
}

# ==============================================================================
# 2. Destructive SQL Invariants
# ==============================================================================
destructive_sql_violations contains {
    "rule_id": "safety.destructive_sql.drop_table",
    "severity": "CRITICAL",
    "message": sprintf("Prohibited DROP TABLE execution: %v", [sql_str]),
    "target": sql_str
} if {
    sql_str != ""
    regex.match("(?i)\\bDROP\\s+TABLE\\b", sql_str)
}

destructive_sql_violations contains {
    "rule_id": "safety.destructive_sql.drop_database",
    "severity": "CRITICAL",
    "message": sprintf("Prohibited DROP DATABASE/SCHEMA execution: %v", [sql_str]),
    "target": sql_str
} if {
    sql_str != ""
    regex.match("(?i)\\bDROP\\s+(DATABASE|SCHEMA)\\b", sql_str)
}

destructive_sql_violations contains {
    "rule_id": "safety.destructive_sql.truncate",
    "severity": "CRITICAL",
    "message": sprintf("Prohibited TRUNCATE execution: %v", [sql_str]),
    "target": sql_str
} if {
    sql_str != ""
    regex.match("(?i)\\bTRUNCATE(\\s+TABLE)?\\b", sql_str)
}

# ==============================================================================
# 3. Path Traversal & Sensitive System Path Invariants
# ==============================================================================
# Check command line directly
path_traversal_violations contains {
    "rule_id": "safety.path_traversal.command",
    "severity": "CRITICAL",
    "message": sprintf("Access to sensitive path or traversal in command: %v", [command_str]),
    "target": command_str
} if {
    command_str != ""
    contains_sensitive_path(command_str)
}

# Check any argument in input.tool_args (JSON map)
path_traversal_violations contains {
    "rule_id": "safety.path_traversal.argument",
    "severity": "CRITICAL",
    "message": sprintf("Path traversal or sensitive target in argument '%v': %v", [arg_key, arg_val]),
    "target": arg_val
} if {
    some arg_key, arg_val in input.tool_args
    is_string(arg_val)
    contains_sensitive_path(arg_val)
}

# Helper to identify sensitive paths
contains_sensitive_path(str) if {
    # Directory traversal sequence
    regex.match("(\\.\\.[\\/\\\\]|[\\/\\\\]\\.\\.)", str)
}

contains_sensitive_path(str) if {
    # Unix credentials and shadow files
    regex.match("(?i)(\\/etc\\/(shadow|passwd|sudoers|master\\.passwd)|\\/proc\\/self\\/environ)", str)
}

contains_sensitive_path(str) if {
    # SSH private keys and authorized_keys
    regex.match("(?i)(\\.ssh[\\/\\\\](id_rsa|id_ed25519|id_ecdsa|authorized_keys|known_hosts)|~[\\/\\\\]\\.ssh)", str)
}

contains_sensitive_path(str) if {
    # Windows registry SAM / SYSTEM hive extraction
    regex.match("(?i)(system32[\\\\\\/]config[\\\\\\/](SAM|SYSTEM|SECURITY)|ntds\\.dit|reg(\\.(?:exe))?\\s+save\\s+(?:HKLM|HKEY_LOCAL_MACHINE)[\\\\\\/](?:SAM|SYSTEM|SECURITY))", str)
}

# ==============================================================================
# 4. System Modification Invariants (chmod 777 on critical directories)
# ==============================================================================
system_modification_violations contains {
    "rule_id": "safety.system_modification.chmod_system_path",
    "severity": "CRITICAL",
    "message": sprintf("Prohibited unsafe permission modification on critical system path: %v", [command_str]),
    "target": command_str
} if {
    command_str != ""
    regex.match("(?i)\\bchmod\\s+(-[a-zA-Z0-9-]*\\s+)*(777|a\\+[rwx]*w[rwx]*)\\s+(\\/|\\/etc|\\/usr|\\/bin|\\/sbin|\\/lib|\\/var|\\/boot|\\/root|C:[\\\\\\/]|C:[\\\\\\/]Windows)\\b", command_str)
}
