'use client';

import React, { memo } from 'react';
import { Handle, Position, NodeProps } from '@xyflow/react';
import { ShieldCheck, ShieldAlert, Cpu, Clock } from 'lucide-react';
import { DagNodeData } from '@/types/traceforge';
import { cn, formatLatency } from '@/lib/utils';

export const PolicyCheckNode = memo(({ data }: NodeProps) => {
  const nodeData = data as DagNodeData;
  const isApproved = nodeData.status === 'approved';
  const isBlocked = nodeData.status === 'blocked';
  const isAwaiting = nodeData.status === 'awaiting_approval';

  const rulesChecked = (nodeData.details?.rulesChecked as string[]) || ['safety.rego'];

  return (
    <div
      className={cn(
        'w-76 rounded-xl bg-slate-900/90 border p-3.5 shadow-lg backdrop-blur-md transition-all',
        isBlocked
          ? 'border-rose-500/60 shadow-rose-500/10'
          : isAwaiting
          ? 'border-amber-400/80 shadow-amber-500/20'
          : 'border-emerald-500/40 shadow-emerald-500/10'
      )}
    >
      {/* Target Handle */}
      <Handle
        type="target"
        position={Position.Left}
        className="w-3 h-3 !bg-emerald-400 !border-2 !border-slate-900"
      />

      <div className="flex items-center justify-between pb-2 mb-2 border-b border-slate-800">
        <div className="flex items-center space-x-2">
          <div
            className={cn(
              'p-1.5 rounded-lg border',
              isBlocked
                ? 'bg-rose-950/80 text-rose-400 border-rose-800/50'
                : 'bg-emerald-950/80 text-emerald-400 border-emerald-800/50'
            )}
          >
            {isBlocked ? <ShieldAlert className="w-4 h-4" /> : <ShieldCheck className="w-4 h-4" />}
          </div>
          <div>
            <h4 className="text-xs font-semibold uppercase tracking-wider text-slate-200">
              {nodeData.label}
            </h4>
            <span className="text-[10px] text-slate-400 font-mono">
              {nodeData.timestamp}
            </span>
          </div>
        </div>

        {nodeData.latency_us ? (
          <div className="flex items-center space-x-1 text-[11px] font-mono text-emerald-300 bg-emerald-950/60 px-2 py-0.5 rounded border border-emerald-800/40">
            <Clock className="w-3 h-3 text-emerald-400" />
            <span>{formatLatency(nodeData.latency_us)}</span>
          </div>
        ) : null}
      </div>

      <div className="p-2 rounded-lg bg-slate-950/80 border border-slate-800 font-mono text-xs">
        <div className="text-[10px] text-slate-500 mb-1">Evaluated Rego Rules:</div>
        <div className="flex flex-wrap gap-1">
          {rulesChecked.map((rule, idx) => (
            <span
              key={idx}
              className="text-[10px] bg-slate-800/80 text-slate-300 px-1.5 py-0.5 rounded border border-slate-700 font-mono"
            >
              {rule}
            </span>
          ))}
        </div>
      </div>

      <div className="mt-2.5 flex items-center justify-between text-[10px] text-slate-400 font-mono">
        <div className="flex items-center space-x-1">
          <Cpu className="w-3 h-3 text-slate-500" />
          <span>Wasmtime Engine</span>
        </div>
        <span
          className={cn(
            'font-semibold uppercase tracking-wider',
            isApproved ? 'text-emerald-400' : isAwaiting ? 'text-amber-400' : 'text-rose-400'
          )}
        >
          {isApproved ? '✓ PASS' : isAwaiting ? '⚠ HITL REQUIRED' : '✗ VIOLATION'}
        </span>
      </div>

      {/* Source Handle */}
      <Handle
        type="source"
        position={Position.Right}
        className="w-3 h-3 !bg-emerald-400 !border-2 !border-slate-900"
      />
    </div>
  );
});

PolicyCheckNode.displayName = 'PolicyCheckNode';
