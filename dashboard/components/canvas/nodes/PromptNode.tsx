'use client';

import React, { memo } from 'react';
import { Handle, Position, NodeProps } from '@xyflow/react';
import { MessageSquareCode, Sparkles } from 'lucide-react';
import { DagNodeData } from '@/types/traceforge';

export const PromptNode = memo(({ data }: NodeProps) => {
  const nodeData = data as DagNodeData;
  const promptText = (nodeData.details?.prompt as string) || nodeData.subtext || '';

  return (
    <div className="w-80 rounded-xl bg-slate-900/90 border border-cyan-500/40 p-3.5 shadow-lg backdrop-blur-md transition-all hover:border-cyan-400 hover:shadow-cyan-500/10 hover:shadow-xl">
      <div className="flex items-center justify-between pb-2 mb-2 border-b border-slate-800">
        <div className="flex items-center space-x-2">
          <div className="p-1.5 rounded-lg bg-cyan-950/80 text-cyan-400 border border-cyan-700/50">
            <MessageSquareCode className="w-4 h-4" />
          </div>
          <div>
            <h4 className="text-xs font-semibold uppercase tracking-wider text-cyan-400">
              {nodeData.label}
            </h4>
            <span className="text-[10px] text-slate-400 font-mono">
              {nodeData.timestamp}
            </span>
          </div>
        </div>
        {nodeData.tokens && (
          <div className="flex items-center space-x-1 text-[11px] font-mono text-cyan-300 bg-cyan-950/60 px-2 py-0.5 rounded border border-cyan-800/40">
            <Sparkles className="w-3 h-3 text-cyan-400" />
            <span>{nodeData.tokens} tok</span>
          </div>
        )}
      </div>

      <p className="text-xs text-slate-200 line-clamp-3 leading-relaxed font-sans bg-slate-950/60 p-2 rounded-lg border border-slate-800/80">
        {promptText}
      </p>

      {nodeData.details?.model && (
        <div className="mt-2.5 flex items-center justify-between text-[10px] text-slate-400 font-mono">
          <span>Model: <strong className="text-cyan-300 font-medium">{String(nodeData.details.model)}</strong></span>
          <span className="text-emerald-400">● Ingress OK</span>
        </div>
      )}

      {/* React Flow Source Handle */}
      <Handle
        type="source"
        position={Position.Right}
        className="w-3 h-3 !bg-cyan-400 !border-2 !border-slate-900"
      />
    </div>
  );
});

PromptNode.displayName = 'PromptNode';
