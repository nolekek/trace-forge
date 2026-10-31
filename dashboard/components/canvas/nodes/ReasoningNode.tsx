'use client';

import React, { memo } from 'react';
import { Handle, Position, NodeProps } from '@xyflow/react';
import { Brain, Cpu, Clock } from 'lucide-react';
import { DagNodeData } from '@/types/traceforge';
import { formatLatency } from '@/lib/utils';

export const ReasoningNode = memo(({ data }: NodeProps) => {
  const nodeData = data as DagNodeData;
  const thought = (nodeData.details?.thought as string) || nodeData.subtext || '';

  return (
    <div className="w-80 rounded-xl bg-slate-900/90 border border-indigo-500/40 p-3.5 shadow-lg backdrop-blur-md transition-all hover:border-indigo-400 hover:shadow-indigo-500/10 hover:shadow-xl">
      {/* Target Handle */}
      <Handle
        type="target"
        position={Position.Left}
        className="w-3 h-3 !bg-indigo-400 !border-2 !border-slate-900"
      />

      <div className="flex items-center justify-between pb-2 mb-2 border-b border-slate-800">
        <div className="flex items-center space-x-2">
          <div className="p-1.5 rounded-lg bg-indigo-950/80 text-indigo-400 border border-indigo-700/50">
            <Brain className="w-4 h-4 animate-pulse" />
          </div>
          <div>
            <h4 className="text-xs font-semibold uppercase tracking-wider text-indigo-300">
              {nodeData.label}
            </h4>
            <span className="text-[10px] text-slate-400 font-mono">
              {nodeData.timestamp}
            </span>
          </div>
        </div>
        {nodeData.latency_us ? (
          <div className="flex items-center space-x-1 text-[11px] font-mono text-indigo-300 bg-indigo-950/60 px-2 py-0.5 rounded border border-indigo-800/40">
            <Clock className="w-3 h-3 text-indigo-400" />
            <span>{formatLatency(nodeData.latency_us)}</span>
          </div>
        ) : null}
      </div>

      <div className="p-2.5 rounded-lg bg-slate-950/70 border border-indigo-950/50 font-mono text-xs text-indigo-100/90 leading-relaxed line-clamp-3">
        <span className="text-indigo-400 font-semibold mr-1.5">&gt;</span>
        {thought}
      </div>

      <div className="mt-2.5 flex items-center justify-between text-[10px] text-slate-400 font-mono">
        <div className="flex items-center space-x-1">
          <Cpu className="w-3 h-3 text-indigo-400" />
          <span>Semantic Vector: <strong className="text-indigo-300">ort/MiniLM</strong></span>
        </div>
        {nodeData.tokens && (
          <span className="text-slate-300">{nodeData.tokens} tokens</span>
        )}
      </div>

      {/* Source Handle */}
      <Handle
        type="source"
        position={Position.Right}
        className="w-3 h-3 !bg-indigo-400 !border-2 !border-slate-900"
      />
    </div>
  );
});

ReasoningNode.displayName = 'ReasoningNode';
