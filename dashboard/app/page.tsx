'use client';

import React, { useState } from 'react';
import { Header } from '@/components/ui/Header';
import { TraceDagCanvas } from '@/components/canvas/TraceDagCanvas';
import { ContextHeatmap } from '@/components/metrics/ContextHeatmap';
import { HitlApprovalModal } from '@/components/hitl/HitlApprovalModal';
import { useTraceForgeSocket } from '@/hooks/useTraceForgeSocket';

export default function DashboardPage() {
  const {
    connectionStatus,
    reconnectAttempt,
    sessionId,
    sessionState,
    budgetTelemetry,
    contextBreakdown,
    nodes,
    edges,
    events,
    approvalRequest,
    setNodes,
    setEdges,
    toggleBreakpoint,
    sendApproval,
    triggerMockStep,
    triggerMockHitl,
    resetGraph,
  } = useTraceForgeSocket();

  const [isSimulating, setIsSimulating] = useState(false);

  const handleSimulateStep = () => {
    setIsSimulating(true);
    triggerMockStep();
    setTimeout(() => setIsSimulating(false), 400);
  };

  return (
    <div className="flex flex-col h-screen w-screen overflow-hidden bg-slate-950 text-slate-100">
      {/* Top Telemetry & Control Status Bar */}
      <Header
        connectionStatus={connectionStatus}
        reconnectAttempt={reconnectAttempt}
        sessionId={sessionId}
        sessionState={sessionState}
        budgetTelemetry={budgetTelemetry}
        onTriggerMockHitl={triggerMockHitl}
        onTriggerMockStep={handleSimulateStep}
        onResetGraph={resetGraph}
      />

      {/* Main Workspace Split-Pane: Canvas (Left) + Telemetry & Context Heatmap (Right) */}
      <div className="flex-1 flex overflow-hidden relative">
        {/* Main DAG Canvas Panel */}
        <main className="flex-1 h-full relative">
          <TraceDagCanvas
            nodes={nodes}
            edges={edges}
            onNodesChange={setNodes}
            onEdgesChange={setEdges}
            onToggleBreakpoint={toggleBreakpoint}
            onTriggerSimulation={handleSimulateStep}
            isSimulating={isSimulating}
          />
        </main>

        {/* Right Side Panel: Telemetry, Token Heatmap & Events Log */}
        <ContextHeatmap
          contextBreakdown={contextBreakdown}
          budgetTelemetry={budgetTelemetry}
          events={events}
        />
      </div>

      {/* Operator HITL Approval Dialog */}
      {approvalRequest && (
        <HitlApprovalModal
          request={approvalRequest}
          onDecision={sendApproval}
          onClose={() => sendApproval({
            request_id: approvalRequest.id,
            decision: 'deny',
            comment: 'Modal dismissed by operator',
          })}
        />
      )}
    </div>
  );
}
