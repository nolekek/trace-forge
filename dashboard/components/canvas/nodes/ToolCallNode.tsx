'use client';

import React, { memo } from 'react';
import { Handle, Position, NodeProps } from '@xyflow/react';
import { Terminal, Clock, ShieldAlert, CheckCircle2, PauseCircle, CircleDot } from 'lucide-react';
import { DagNodeData } from '@/types/traceforge';
import { cn, formatLatency } from '@/lib/utils';

export const ToolCallNode = memo(({ data }: NodeProps) => {
  const nodeData = data as DagNodeData;
  const toolName = nodeData.tool_name || 'unknown_tool';
  const hasBreakpoint = Boolean(nodeData.has_breakpoint);

  // Status-based styling
  const isAwaiting = nodeData.status === 'awaiting_approval';
  const isBlocked = nodeData.status === 'blocked';
  const isApproved = nodeData.status === 'approved';

  const borderColor = isBlocked
    ? 'border-rose-500/80 shadow-rose-500/20'
    : isAwaiting
    ? 'border-amber-400 shadow-amber-500/30 animate-pulse-amber'
    : 'border-emerald-500/50 shadow-emerald-500/10';

  const headerBg = isBlocked
    ? 'bg-rose-950/80 text-rose-300'
    : isAwaiting
    ? 'bg-amber-950/80 text-amber-300'
    : 'bg-emerald-950/80 text-emerald-300';

  // Format arguments preview
  let argsPreview = '';
  if (typeof nodeData.tool_args === 'string') {
    argsPreview = nodeData.tool_args;
  } else if (nodeData.tool_args) {
    if ('command' in nodeData.tool_args) {
      argsPreview = String(nodeData.tool_args.command);
    } else if ('path' in nodeData.tool_args) {
      argsPreview = String(nodeData.tool_args.path);
    } else {
      argsPreview = JSON.stringify(nodeData.tool_args);
    }
  }

  const handleBreakpointClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (nodeData.onToggleBreakpoint) {
      nodeData.onToggleBreakpoint(toolName);
    }
  };

  return (
    <div
      className={cn(
        'w-84 rounded-xl bg-slate-900/95 border-2 p-3.5 shadow-xl backdrop-blur-md transition-all',
        borderColor
      )}
    >
      {/* Target Handle */}
      <Handle
        type="target"
        position={Position.Left}
        className={cn(
          'w-3.5 h-3.5 !border-2 !border-slate-900',
          isBlocked ? '!bg-rose-500' : isAwaiting ? '!bg-amber-400' : '!bg-emerald-400'
        )}
      />

      {/* Node Header */}
      <div className="flex items-center justify-between pb-2 mb-2 border-b border-slate-800">
        <div className="flex items-center space-x-2">
          <div className={cn('p-1.5 rounded-lg border', headerBg)}>
            <Terminal className="w-4 h-4" />
          </div>
          <div>
            <div className="flex items-center space-x-1.5">
              <h4 className="text-xs font-bold text-slate-100 font-mono tracking-tight">
                {toolName}
              </h4>
            </div>
            <span className="text-[10px] text-slate-400 font-mono">
              {nodeData.timestamp}
            </span>
          </div>
        </div>

        {/* Breakpoint Action Button (Minimum 44px clickable target) */}
        <button
          type="button"
          onClick={handleBreakpointClick}
          title={hasBreakpoint ? 'Breakpoint Active (Click to remove)' : 'Click to Set Breakpoint'}
          className={cn(
            'flex items-center space-x-1 px-2.5 py-1.5 rounded-lg text-xs font-mono transition-all cursor-pointer border',
            hasBreakpoint
              ? 'bg-rose-950/80 text-rose-300 border-rose-600/70 shadow-sm shadow-rose-500/50'
              : 'bg-slate-800/80 text-slate-400 border-slate-700/60 hover:text-slate-200 hover:border-slate-500'
          )}
        >
          {hasBreakpoint ? (
            <>
              <PauseCircle className="w-3.5 h-3.5 text-rose-400 animate-pulse" />
              <span className="text-[11px] font-semibold">BP ON</span>
            </>
          ) : (
            <>
              <CircleDot className="w-3.5 h-3.5 text-slate-400" />
              <span className="text-[11px]">BP</span>
            </>
          )}
        </button>
      </div>

      {/* Tool Arguments / Code Box */}
      <div className="p-2.5 rounded-lg bg-slate-950/90 border border-slate-800 font-mono text-xs text-slate-200 leading-snug">
        <div className="text-[10px] uppercase tracking-wider text-slate-500 mb-1">Command / Params:</div>
        <div className="text-emerald-400 break-all line-clamp-2">
          $ {argsPreview || 'none'}
        </div>
      </div>

      {/* Violations Alert if blocked or waiting */}
      {nodeData.violations && nodeData.violations.length > 0 && (
        <div className="mt-2 p-2 rounded-lg bg-rose-950/40 border border-rose-800/50 flex items-start space-x-1.5">
          <ShieldAlert className="w-3.5 h-3.5 text-rose-400 shrink-0 mt-0.5" />
          <span className="text-[10px] font-mono text-rose-300 line-clamp-2">
            {nodeData.violations[0]}
          </span>
        </div>
      )}

      {/* Footer Metrics */}
      <div className="mt-2.5 flex items-center justify-between text-[11px] font-mono">
        <div className="flex items-center space-x-1 text-slate-400">
          <Clock className="w-3 h-3 text-slate-500" />
          <span>{nodeData.latency_us ? formatLatency(nodeData.latency_us) : '< 1ms'}</span>
        </div>

        {isApproved && (
          <span className="flex items-center space-x-1 text-emerald-400">
            <CheckCircle2 className="w-3 h-3" />
            <span className="text-[10px] font-semibold uppercase">Approved</span>
          </span>
        )}
        {isAwaiting && (
          <span className="flex items-center space-x-1 text-amber-300 font-semibold animate-pulse">
            <span className="w-1.5 h-1.5 rounded-full bg-amber-400"></span>
            <span className="text-[10px] uppercase">Awaiting Approval</span>
          </span>
        )}
        {isBlocked && (
          <span className="flex items-center space-x-1 text-rose-400 font-semibold">
            <ShieldAlert className="w-3 h-3" />
            <span className="text-[10px] uppercase">Blocked</span>
          </span>
        )}
      </div>

      {/* Source Handle */}
      <Handle
        type="source"
        position={Position.Right}
        className={cn(
          'w-3.5 h-3.5 !border-2 !border-slate-900',
          isBlocked ? '!bg-rose-500' : isAwaiting ? '!bg-amber-400' : '!bg-emerald-400'
        )}
      />
    </div>
  );
});

ToolCallNode.displayName = 'ToolCallNode';
