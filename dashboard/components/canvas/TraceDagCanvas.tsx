'use client';

import React, { useMemo, useCallback } from 'react';
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  Panel,
  Node,
  Edge,
  OnNodesChange,
  OnEdgesChange,
  applyNodeChanges,
  applyEdgeChanges,
  BackgroundVariant,
  useReactFlow,
  ReactFlowProvider,
} from '@xyflow/react';
import { Maximize2, Shield, CircleDot, Play, Sparkles } from 'lucide-react';
import { DagNodeData } from '@/types/traceforge';
import { PromptNode } from './nodes/PromptNode';
import { ReasoningNode } from './nodes/ReasoningNode';
import { ToolCallNode } from './nodes/ToolCallNode';
import { PolicyCheckNode } from './nodes/PolicyCheckNode';
import { HitlGateNode } from './nodes/HitlGateNode';

interface TraceDagCanvasProps {
  nodes: Node<DagNodeData>[];
  edges: Edge[];
  onNodesChange?: (nodes: Node<DagNodeData>[]) => void;
  onEdgesChange?: (edges: Edge[]) => void;
  onToggleBreakpoint?: (toolName: string) => void;
  onTriggerSimulation?: () => void;
  isSimulating?: boolean;
}

const nodeTypes = {
  llm_prompt: PromptNode,
  reasoning: ReasoningNode,
  tool_call: ToolCallNode,
  policy_check: PolicyCheckNode,
  hitl_gate: HitlGateNode,
};

function CanvasInner({
  nodes,
  edges,
  onNodesChange,
  onEdgesChange,
  onToggleBreakpoint,
  onTriggerSimulation,
  isSimulating,
}: TraceDagCanvasProps) {
  const { fitView } = useReactFlow();

  const handleNodesChange: OnNodesChange<Node<DagNodeData>> = useCallback(
    (changes) => {
      if (onNodesChange) {
        onNodesChange(applyNodeChanges(changes, nodes) as Node<DagNodeData>[]);
      }
    },
    [nodes, onNodesChange]
  );

  const handleEdgesChange: OnEdgesChange = useCallback(
    (changes) => {
      if (onEdgesChange) {
        onEdgesChange(applyEdgeChanges(changes, edges));
      }
    },
    [edges, onEdgesChange]
  );

  // Inject onToggleBreakpoint callback into tool nodes data
  const augmentedNodes = useMemo(() => {
    return nodes.map((node) => {
      if (node.type === 'tool_call') {
        return {
          ...node,
          data: {
            ...node.data,
            onToggleBreakpoint,
          },
        };
      }
      return node;
    });
  }, [nodes, onToggleBreakpoint]);

  return (
    <div className="relative w-full h-full bg-slate-950 select-none overflow-hidden">
      <ReactFlow
        nodes={augmentedNodes}
        edges={edges}
        nodeTypes={nodeTypes}
        onNodesChange={handleNodesChange}
        onEdgesChange={handleEdgesChange}
        fitView
        fitViewOptions={{ padding: 0.2 }}
        minZoom={0.2}
        maxZoom={1.8}
        proOptions={{ hideAttribution: true }}
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={24}
          size={1.5}
          color="#1e293b"
          className="opacity-75"
        />

        <Controls
          showInteractive={false}
          className="!bg-slate-900 !border-slate-800 !text-slate-300"
        />

        <MiniMap
          nodeStrokeWidth={3}
          nodeColor={(node) => {
            switch (node.type) {
              case 'llm_prompt':
                return '#06b6d4';
              case 'reasoning':
                return '#6366f1';
              case 'tool_call':
                return '#10b981';
              case 'policy_check':
                return '#3b82f6';
              case 'hitl_gate':
                return '#f59e0b';
              default:
                return '#64748b';
            }
          }}
          maskColor="rgba(2, 6, 23, 0.75)"
          className="!bottom-4 !right-4 !border !border-slate-800 !rounded-xl !bg-slate-950/90"
        />

        {/* Top Floating Control Bar */}
        <Panel position="top-left" className="!m-4">
          <div className="flex items-center space-x-2 bg-slate-900/90 backdrop-blur-md border border-slate-800 px-3 py-2 rounded-xl shadow-xl">
            <div className="flex items-center space-x-2 mr-2 border-r border-slate-800 pr-3">
              <Shield className="w-4 h-4 text-emerald-400" />
              <span className="text-xs font-bold tracking-wider text-slate-200">
                DAG CANVAS
              </span>
              <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-slate-800 text-slate-400">
                {nodes.length} nodes
              </span>
            </div>

            <button
              type="button"
              onClick={() => fitView({ duration: 400, padding: 0.25 })}
              className="flex items-center space-x-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-slate-300 hover:text-white bg-slate-800/80 hover:bg-slate-700/80 border border-slate-700/60 transition-all cursor-pointer"
            >
              <Maximize2 className="w-3.5 h-3.5 text-cyan-400" />
              <span>Fit View</span>
            </button>

            {onTriggerSimulation && (
              <button
                type="button"
                onClick={onTriggerSimulation}
                className="flex items-center space-x-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-slate-200 hover:text-white bg-emerald-950/80 hover:bg-emerald-900/80 border border-emerald-700/60 transition-all cursor-pointer"
              >
                {isSimulating ? (
                  <>
                    <Sparkles className="w-3.5 h-3.5 text-amber-400 animate-spin" />
                    <span>Simulating...</span>
                  </>
                ) : (
                  <>
                    <Play className="w-3.5 h-3.5 text-emerald-400" />
                    <span>Trigger Step</span>
                  </>
                )}
              </button>
            )}

            <div className="flex items-center space-x-1.5 text-[11px] font-mono text-slate-400 pl-2">
              <CircleDot className="w-3 h-3 text-rose-400" />
              <span>Click Tool node to toggle Breakpoint</span>
            </div>
          </div>
        </Panel>
      </ReactFlow>
    </div>
  );
}

export function TraceDagCanvas(props: TraceDagCanvasProps) {
  return (
    <ReactFlowProvider>
      <CanvasInner {...props} />
    </ReactFlowProvider>
  );
}
