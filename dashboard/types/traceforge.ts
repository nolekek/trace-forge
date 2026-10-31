/**
 * Core domain types for TraceForge Web Canvas & Control Center.
 * Synchronized with crates/traceforge-core/src/types.rs and events.rs.
 */

export type EventKind =
  | 'llm_request'
  | 'llm_chunk'
  | 'tool_call'
  | 'policy_evaluation'
  | 'loop_detected'
  | 'hitl_action';

export type SessionState =
  | 'RUNNING'
  | 'STREAMING'
  | 'AWAITING_POLICY'
  | 'AWAITING_APPROVAL'
  | 'PAUSED'
  | 'TRIPPED'
  | 'COMPLETED';

export interface TokenUsage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  reasoning_tokens?: number;
}

export interface PolicyResult {
  allow: boolean;
  rule_name?: string;
  reason?: string;
  require_hitl: boolean;
  violations?: string[];
}

export interface TraceEvent {
  event_id: string;
  session_id: string;
  timestamp_ns: number;
  latency_us: number;
  kind: EventKind;
  token_usage?: TokenUsage;
  cost_usd: number;
  policy_result?: PolicyResult;
  payload: Record<string, any>;
}

export type RiskLevel = 'low' | 'medium' | 'high' | 'critical';

export interface ApprovalRequest {
  id: string;
  session_id: string;
  timestamp_ns: number;
  tool_name: string;
  tool_args: Record<string, any> | string;
  rule_name: string;
  reason: string;
  risk_level: RiskLevel;
  violations: string[];
}

export interface ApprovalDecision {
  request_id: string;
  decision: 'approve' | 'deny' | 'modify';
  modified_command?: string;
  comment?: string;
}

export type ConnectionStatus = 'connected' | 'disconnected' | 'reconnecting';

export type DagNodeType =
  | 'llm_prompt'
  | 'reasoning'
  | 'tool_call'
  | 'policy_check'
  | 'hitl_gate';

export type NodeExecutionStatus =
  | 'pending'
  | 'running'
  | 'approved'
  | 'blocked'
  | 'awaiting_approval';

export interface DagNodeData {
  id: string;
  type: DagNodeType;
  label: string;
  status: NodeExecutionStatus;
  subtext?: string;
  latency_us?: number;
  timestamp: string;
  tool_name?: string;
  tool_args?: Record<string, any> | string;
  has_breakpoint?: boolean;
  violations?: string[];
  tokens?: number;
  details?: Record<string, any>;
  onToggleBreakpoint?: (toolName: string) => void;
  [key: string]: unknown;
}

export interface ContextBreakdown {
  systemPromptTokens: number;
  historyTokens: number;
  toolTokens: number;
  reasoningTokens: number;
  totalTokens: number;
  maxTokens: number;
  fragmentationRatio: number; // 0.0 to 1.0 (Lost-in-the-Middle danger)
}

export interface BudgetTelemetry {
  spentUsd: number;
  maxCostUsd: number;
  totalTokens: number;
  maxTokens: number;
  estimatedCallsRemaining: number;
}
