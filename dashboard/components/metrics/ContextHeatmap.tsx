'use client';

import React, { useState } from 'react';
import {
  DollarSign,
  Layers,
  Activity,
  AlertTriangle,
  ChevronDown,
  ChevronUp,
  Cpu,
  Clock,
  Sparkles,
  Search,
} from 'lucide-react';
import { ContextBreakdown, BudgetTelemetry, TraceEvent } from '@/types/traceforge';
import { cn, formatCost, formatLatency, formatTokens } from '@/lib/utils';

interface ContextHeatmapProps {
  contextBreakdown: ContextBreakdown;
  budgetTelemetry: BudgetTelemetry;
  events: TraceEvent[];
}

export function ContextHeatmap({
  contextBreakdown,
  budgetTelemetry,
  events,
}: ContextHeatmapProps) {
  const [selectedEventId, setSelectedEventId] = useState<string | null>(null);
  const [filterQuery, setFilterQuery] = useState('');

  // Calculations for token distribution
  const totalTokens = contextBreakdown.totalTokens || 1;
  const sysPct = Math.round((contextBreakdown.systemPromptTokens / totalTokens) * 100);
  const histPct = Math.round((contextBreakdown.historyTokens / totalTokens) * 100);
  const toolPct = Math.round((contextBreakdown.toolTokens / totalTokens) * 100);
  const reasonPct = Math.round((contextBreakdown.reasoningTokens / totalTokens) * 100);

  // Budget calculations
  const spentPct = Math.min(
    100,
    Math.round((budgetTelemetry.spentUsd / (budgetTelemetry.maxCostUsd || 1)) * 100)
  );

  const filteredEvents = events.filter((e) => {
    if (!filterQuery) return true;
    const q = filterQuery.toLowerCase();
    return (
      e.kind.toLowerCase().includes(q) ||
      e.session_id.toLowerCase().includes(q) ||
      JSON.stringify(e.payload).toLowerCase().includes(q)
    );
  });

  return (
    <aside className="w-96 flex flex-col h-full bg-slate-900/90 border-l border-slate-800 select-none overflow-hidden backdrop-blur-md">
      {/* Panel Header */}
      <div className="p-4 border-b border-slate-800 bg-slate-950/60 flex items-center justify-between">
        <div className="flex items-center space-x-2">
          <Activity className="w-4 h-4 text-cyan-400" />
          <h2 className="text-sm font-bold uppercase tracking-wider text-slate-100">
            Telemetry & Guard
          </h2>
        </div>
        <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-emerald-950/80 text-emerald-400 border border-emerald-800/40">
          Live &lt; 5ms SLA
        </span>
      </div>

      <div className="flex-1 overflow-y-auto p-4 space-y-5">
        {/* SECTION 1: Token Context Heatmap & Lost-in-the-Middle Audit */}
        <div className="rounded-xl bg-slate-950/60 border border-slate-800/80 p-3.5 space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center space-x-1.5">
              <Layers className="w-3.5 h-3.5 text-cyan-400" />
              <h3 className="text-xs font-semibold uppercase tracking-wider text-slate-200">
                Context Window Map
              </h3>
            </div>
            <span className="text-xs font-mono text-cyan-300">
              {formatTokens(contextBreakdown.totalTokens)} / {formatTokens(contextBreakdown.maxTokens)}
            </span>
          </div>

          {/* Segmented Distribution Bar */}
          <div className="w-full h-3 rounded-full overflow-hidden flex bg-slate-800 shadow-inner">
            <div
              style={{ width: `${sysPct}%` }}
              className="bg-cyan-500 transition-all duration-500 hover:brightness-125"
              title={`System Prompt: ${contextBreakdown.systemPromptTokens} tokens (${sysPct}%)`}
            />
            <div
              style={{ width: `${histPct}%` }}
              className="bg-indigo-500 transition-all duration-500 hover:brightness-125"
              title={`Context History: ${contextBreakdown.historyTokens} tokens (${histPct}%)`}
            />
            <div
              style={{ width: `${toolPct}%` }}
              className="bg-emerald-500 transition-all duration-500 hover:brightness-125"
              title={`Tool Outputs: ${contextBreakdown.toolTokens} tokens (${toolPct}%)`}
            />
            <div
              style={{ width: `${reasonPct}%` }}
              className="bg-purple-500 transition-all duration-500 hover:brightness-125"
              title={`Reasoning CoT: ${contextBreakdown.reasoningTokens} tokens (${reasonPct}%)`}
            />
          </div>

          {/* Segment Legend */}
          <div className="grid grid-cols-2 gap-2 text-[11px] font-mono">
            <div className="flex items-center space-x-1.5 text-slate-300">
              <span className="w-2.5 h-2.5 rounded-sm bg-cyan-500 shrink-0"></span>
              <span className="truncate">System ({sysPct}%)</span>
            </div>
            <div className="flex items-center space-x-1.5 text-slate-300">
              <span className="w-2.5 h-2.5 rounded-sm bg-indigo-500 shrink-0"></span>
              <span className="truncate">History ({histPct}%)</span>
            </div>
            <div className="flex items-center space-x-1.5 text-slate-300">
              <span className="w-2.5 h-2.5 rounded-sm bg-emerald-500 shrink-0"></span>
              <span className="truncate">Tools ({toolPct}%)</span>
            </div>
            <div className="flex items-center space-x-1.5 text-slate-300">
              <span className="w-2.5 h-2.5 rounded-sm bg-purple-500 shrink-0"></span>
              <span className="truncate">Reasoning ({reasonPct}%)</span>
            </div>
          </div>

          {/* Lost-in-the-Middle Audit Indicator */}
          <div className="pt-2 border-t border-slate-800/80 flex items-center justify-between text-[11px] font-mono">
            <span className="text-slate-400">Context Fragmentation:</span>
            <span
              className={cn(
                'px-1.5 py-0.5 rounded font-semibold',
                contextBreakdown.fragmentationRatio > 0.4
                  ? 'bg-amber-950/80 text-amber-300 border border-amber-700/50'
                  : 'bg-emerald-950/80 text-emerald-400 border border-emerald-800/40'
              )}
            >
              {(contextBreakdown.fragmentationRatio * 100).toFixed(0)}% (
              {contextBreakdown.fragmentationRatio > 0.4 ? 'Degraded Attention' : 'Optimal'})
            </span>
          </div>
        </div>

        {/* SECTION 2: Budget Expense Indicator & Hard Limit Gauge */}
        <div className="rounded-xl bg-slate-950/60 border border-slate-800/80 p-3.5 space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center space-x-1.5">
              <DollarSign className="w-3.5 h-3.5 text-emerald-400" />
              <h3 className="text-xs font-semibold uppercase tracking-wider text-slate-200">
                Budget Guard
              </h3>
            </div>
            <span className="text-xs font-mono font-bold text-emerald-400">
              {formatCost(budgetTelemetry.spentUsd)} / {formatCost(budgetTelemetry.maxCostUsd)}
            </span>
          </div>

          {/* Progress bar */}
          <div className="w-full bg-slate-800 h-2.5 rounded-full overflow-hidden">
            <div
              style={{ width: `${spentPct}%` }}
              className={cn(
                'h-full transition-all duration-500 rounded-full',
                spentPct > 80 ? 'bg-rose-500' : spentPct > 50 ? 'bg-amber-400' : 'bg-emerald-500'
              )}
            />
          </div>

          <div className="flex items-center justify-between text-[11px] font-mono text-slate-400">
            <span>Used: <strong className="text-slate-200">{spentPct}%</strong></span>
            <span>Est. Remaining Calls: <strong className="text-cyan-300">{budgetTelemetry.estimatedCallsRemaining}</strong></span>
          </div>
        </div>

        {/* SECTION 3: Real-Time Tracing Events Table */}
        <div className="rounded-xl bg-slate-950/60 border border-slate-800/80 p-3.5 space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center space-x-1.5">
              <Cpu className="w-3.5 h-3.5 text-indigo-400" />
              <h3 className="text-xs font-semibold uppercase tracking-wider text-slate-200">
                Recent Trace Events
              </h3>
            </div>
            <span className="text-[10px] font-mono text-slate-400">
              {filteredEvents.length} events
            </span>
          </div>

          {/* Quick search input */}
          <div className="relative">
            <Search className="w-3 h-3 absolute left-2.5 top-2.5 text-slate-500" />
            <input
              type="text"
              placeholder="Filter events..."
              value={filterQuery}
              onChange={(e) => setFilterQuery(e.target.value)}
              className="w-full bg-slate-900 border border-slate-800 rounded-lg pl-7 pr-2.5 py-1 text-xs text-slate-200 placeholder-slate-500 focus:outline-none focus:border-cyan-500"
            />
          </div>

          {/* Events List */}
          <div className="space-y-2 max-h-64 overflow-y-auto pr-1">
            {filteredEvents.map((evt) => {
              const isSelected = selectedEventId === evt.event_id;
              const isTool = evt.kind === 'tool_call';
              const isPolicy = evt.kind === 'policy_evaluation';
              const isLlm = evt.kind === 'llm_request' || evt.kind === 'llm_chunk';

              return (
                <div
                  key={evt.event_id}
                  onClick={() => setSelectedEventId(isSelected ? null : evt.event_id)}
                  className={cn(
                    'p-2.5 rounded-lg border text-xs cursor-pointer transition-all',
                    isSelected
                      ? 'bg-slate-800/90 border-cyan-500/70 shadow-md'
                      : 'bg-slate-900/60 border-slate-800/70 hover:bg-slate-850 hover:border-slate-700'
                  )}
                >
                  <div className="flex items-center justify-between">
                    <div className="flex items-center space-x-1.5">
                      <span
                        className={cn(
                          'w-2 h-2 rounded-full',
                          isTool
                            ? 'bg-emerald-400'
                            : isPolicy
                            ? 'bg-indigo-400'
                            : isLlm
                            ? 'bg-cyan-400'
                            : 'bg-amber-400'
                        )}
                      />
                      <span className="font-mono font-medium text-slate-200">
                        {evt.kind}
                      </span>
                    </div>

                    <div className="flex items-center space-x-2 font-mono text-[11px] text-slate-400">
                      <span className="flex items-center space-x-0.5">
                        <Clock className="w-2.5 h-2.5 text-slate-500" />
                        <span>{formatLatency(evt.latency_us)}</span>
                      </span>
                      {isSelected ? (
                        <ChevronUp className="w-3 h-3 text-slate-400" />
                      ) : (
                        <ChevronDown className="w-3 h-3 text-slate-500" />
                      )}
                    </div>
                  </div>

                  {/* Summary row */}
                  <div className="mt-1 flex items-center justify-between text-[11px] font-mono text-slate-400">
                    <span className="truncate max-w-[200px]">
                      {evt.payload.tool_name
                        ? `tool: ${evt.payload.tool_name}`
                        : evt.payload.model
                        ? `model: ${evt.payload.model}`
                        : evt.policy_result?.rule_name
                        ? `rule: ${evt.policy_result.rule_name}`
                        : `cost: ${formatCost(evt.cost_usd)}`}
                    </span>
                    {evt.cost_usd > 0 && (
                      <span className="text-emerald-400 font-medium">
                        {formatCost(evt.cost_usd)}
                      </span>
                    )}
                  </div>

                  {/* Expanded JSON inspection drawer */}
                  {isSelected && (
                    <div className="mt-2 pt-2 border-t border-slate-800 text-[10px] font-mono">
                      <div className="text-slate-400 mb-1">Payload Envelope:</div>
                      <pre className="p-2 rounded bg-slate-950 border border-slate-800 text-cyan-300 overflow-x-auto max-h-36">
                        {JSON.stringify(
                          {
                            event_id: evt.event_id,
                            session_id: evt.session_id,
                            latency_us: evt.latency_us,
                            policy_result: evt.policy_result,
                            payload: evt.payload,
                          },
                          null,
                          2
                        )}
                      </pre>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </aside>
  );
}
