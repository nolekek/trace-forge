'use client';

import React, { memo } from 'react';
import { Handle, Position, NodeProps } from '@xyflow/react';
import { UserCheck, ShieldAlert, Sparkles, CheckCircle2 } from 'lucide-react';
import { DagNodeData } from '@/types/traceforge';
import { cn } from '@/lib/utils';

export const HitlGateNode = memo(({ data }: NodeProps) => {
  const nodeData = data as DagNodeData;
  const isAwaiting = nodeData.status === 'awaiting_approval';
  const isApproved = nodeData.status === 'approved';
  const isBlocked = nodeData.status === 'blocked';

  return (
    <div
      className={cn(
        'w-84 rounded-xl bg-slate-900/95 border-2 p-3.5 shadow-2xl backdrop-blur-md transition-all',
        isAwaiting
          ? 'border-amber-400 bg-amber-950/20 shadow-amber-500/30 animate-pulse-amber'
          : isBlocked
          ? 'border-rose-500 bg-rose-950/20 shadow-rose-500/20'
          : 'border-emerald-500/60 bg-emerald-950/20 shadow-emerald-500/10'
      )}
    >
      {/* Target Handle */}
      <Handle
        type="target"
        position={Position.Left}
        className={cn(
          'w-3.5 h-3.5 !border-2 !border-slate-900',
          isAwaiting ? '!bg-amber-400' : isBlocked ? '!bg-rose-500' : '!bg-emerald-400'
        )}
      />

      <div className="flex items-center justify-between pb-2 mb-2 border-b border-slate-800">
        <div className="flex items-center space-x-2">
          <div
            className={cn(
              'p-1.5 rounded-lg border',
              isAwaiting
                ? 'bg-amber-950/80 text-amber-300 border-amber-600/60'
                : isBlocked
                ? 'bg-rose-950/80 text-rose-300 border-rose-600/60'
                : 'bg-emerald-950/80 text-emerald-300 border-emerald-600/60'
            )}
          >
            <UserCheck className="w-4 h-4" />
          </div>
          <div>
            <h4 className="text-xs font-bold uppercase tracking-wider text-slate-100">
              {nodeData.label}
            </h4>
            <span className="text-[10px] text-slate-400 font-mono">
              {nodeData.timestamp}
            </span>
          </div>
        </div>

        {isAwaiting && (
          <span className="flex items-center space-x-1 px-2 py-0.5 rounded text-[10px] font-mono font-bold bg-amber-400 text-slate-950 animate-bounce">
            <Sparkles className="w-3 h-3" />
            <span>ACTION REQ</span>
          </span>
        )}
      </div>

      <div className="p-2.5 rounded-lg bg-slate-950/90 border border-slate-800 text-xs">
        <div className="text-[10px] uppercase font-mono tracking-wider text-slate-400 mb-1">
          Safety Gate Trigger:
        </div>
        <p className="font-mono text-xs text-amber-300 line-clamp-2">
          {nodeData.violations?.[0] || 'Manual Breakpoint or Invariant Guard tripped execution'}
        </p>
      </div>

      <div className="mt-2.5 flex items-center justify-between text-[11px] font-mono">
        <div className="flex items-center space-x-1 text-slate-400">
          <span className="text-[10px] text-slate-500">Target:</span>
          <span className="text-slate-300 font-semibold">{nodeData.tool_name || 'execute_bash'}</span>
        </div>

        {isAwaiting ? (
          <span className="text-amber-400 font-semibold text-[11px]">
            Modal Active
          </span>
        ) : isApproved ? (
          <span className="flex items-center space-x-1 text-emerald-400 font-semibold">
            <CheckCircle2 className="w-3 h-3" />
            <span className="text-[10px]">OPERATOR OVERRIDE</span>
          </span>
        ) : (
          <span className="flex items-center space-x-1 text-rose-400 font-semibold">
            <ShieldAlert className="w-3 h-3" />
            <span className="text-[10px]">BLOCKED</span>
          </span>
        )}
      </div>

      {/* Source Handle */}
      <Handle
        type="source"
        position={Position.Right}
        className={cn(
          'w-3.5 h-3.5 !border-2 !border-slate-900',
          isAwaiting ? '!bg-amber-400' : isBlocked ? '!bg-rose-500' : '!bg-emerald-400'
        )}
      />
    </div>
  );
});

HitlGateNode.displayName = 'HitlGateNode';
