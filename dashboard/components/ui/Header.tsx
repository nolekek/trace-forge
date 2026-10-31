'use client';

import React, { useState } from 'react';
import {
  Shield,
  Activity,
  DollarSign,
  Copy,
  Check,
  AlertTriangle,
  Play,
  RotateCcw,
  Sparkles,
} from 'lucide-react';
import { ConnectionStatus, SessionState, BudgetTelemetry } from '@/types/traceforge';
import { cn, formatCost } from '@/lib/utils';

interface HeaderProps {
  connectionStatus: ConnectionStatus;
  reconnectAttempt: number;
  sessionId: string;
  sessionState: SessionState;
  budgetTelemetry: BudgetTelemetry;
  onTriggerMockHitl: () => void;
  onTriggerMockStep: () => void;
  onResetGraph: () => void;
}

export function Header({
  connectionStatus,
  reconnectAttempt,
  sessionId,
  sessionState,
  budgetTelemetry,
  onTriggerMockHitl,
  onTriggerMockStep,
  onResetGraph,
}: HeaderProps) {
  const [copied, setCopied] = useState(false);

  const copySessionId = () => {
    navigator.clipboard.writeText(sessionId);
    setCopied(true);
    setTimeout(() => setCopied(false), 1800);
  };

  const isConnected = connectionStatus === 'connected';
  const isReconnecting = connectionStatus === 'reconnecting';

  return (
    <header className="h-14 bg-slate-950 border-b border-slate-800 px-4 flex items-center justify-between select-none shrink-0 z-20">
      {/* Brand & Identity */}
      <div className="flex items-center space-x-3">
        <div className="flex items-center space-x-2">
          <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-cyan-500 to-emerald-500 flex items-center justify-center shadow-lg shadow-cyan-500/20">
            <Shield className="w-4 h-4 text-slate-950 font-bold" />
          </div>
          <div>
            <div className="flex items-center space-x-1.5">
              <span className="text-sm font-black tracking-tight text-white">
                TRACE<span className="text-cyan-400">FORGE</span>
              </span>
              <span className="text-[10px] font-mono px-1.5 py-0.2 rounded bg-cyan-950/80 text-cyan-400 border border-cyan-800/50">
                v0.1-GUARD
              </span>
            </div>
          </div>
        </div>

        <div className="h-5 w-px bg-slate-800 mx-1" />

        {/* Connection Status Pill */}
        <div
          className={cn(
            'flex items-center space-x-2 px-2.5 py-1 rounded-full text-xs font-mono border transition-all',
            isConnected
              ? 'bg-emerald-950/60 border-emerald-700/60 text-emerald-300'
              : isReconnecting
              ? 'bg-amber-950/60 border-amber-700/60 text-amber-300'
              : 'bg-slate-900 border-slate-700/70 text-slate-400'
          )}
        >
          <span
            className={cn(
              'w-2 h-2 rounded-full',
              isConnected
                ? 'bg-emerald-400 animate-pulse'
                : isReconnecting
                ? 'bg-amber-400 animate-ping'
                : 'bg-slate-500'
            )}
          />
          <span className="text-[11px] font-medium">
            {isConnected
              ? 'Connected (ws://localhost:8080)'
              : isReconnecting
              ? `Reconnecting... (attempt ${reconnectAttempt})`
              : 'Offline / Standalone Mode'}
          </span>
        </div>

        {/* Session State Badge */}
        <div
          className={cn(
            'flex items-center space-x-1.5 px-2.5 py-1 rounded-lg text-[11px] font-mono font-semibold border',
            sessionState === 'AWAITING_APPROVAL'
              ? 'bg-amber-950/70 border-amber-600 text-amber-300 animate-pulse'
              : sessionState === 'TRIPPED'
              ? 'bg-rose-950/70 border-rose-600 text-rose-300'
              : 'bg-slate-900 border-slate-800 text-slate-300'
          )}
        >
          <Activity className="w-3 h-3" />
          <span>{sessionState}</span>
        </div>
      </div>

      {/* Middle & Right: Session ID, Budget Pill & Actions */}
      <div className="flex items-center space-x-3">
        {/* Active Session Pill */}
        <button
          type="button"
          onClick={copySessionId}
          title="Click to copy Session ID"
          className="flex items-center space-x-1.5 px-2.5 py-1 rounded-lg text-xs font-mono bg-slate-900/90 hover:bg-slate-850 border border-slate-800 text-slate-300 transition-colors cursor-pointer"
        >
          <span className="text-slate-500">Session:</span>
          <span className="text-cyan-300 font-semibold">{sessionId}</span>
          {copied ? (
            <Check className="w-3.5 h-3.5 text-emerald-400 ml-0.5" />
          ) : (
            <Copy className="w-3.5 h-3.5 text-slate-500 hover:text-slate-300 ml-0.5" />
          )}
        </button>

        {/* Budget Pill */}
        <div className="flex items-center space-x-1.5 px-2.5 py-1 rounded-lg text-xs font-mono bg-slate-900/90 border border-slate-800 text-slate-300">
          <DollarSign className="w-3.5 h-3.5 text-emerald-400" />
          <span className="text-slate-400">Budget:</span>
          <span className="text-emerald-400 font-bold">
            {formatCost(budgetTelemetry.spentUsd)}
          </span>
          <span className="text-slate-500">/ {formatCost(budgetTelemetry.maxCostUsd)}</span>
        </div>

        <div className="h-5 w-px bg-slate-800 mx-1" />

        {/* Control Center Simulation Buttons */}
        <div className="flex items-center space-x-1.5">
          <button
            type="button"
            onClick={onTriggerMockHitl}
            className="flex items-center space-x-1 px-2.5 py-1.5 rounded-lg text-xs font-mono font-medium text-amber-300 bg-amber-950/70 hover:bg-amber-900/80 border border-amber-600/70 transition-all cursor-pointer shadow-sm shadow-amber-500/10"
          >
            <AlertTriangle className="w-3 h-3 text-amber-400" />
            <span>Simulate HITL</span>
          </button>

          <button
            type="button"
            onClick={onTriggerMockStep}
            className="flex items-center space-x-1 px-2.5 py-1.5 rounded-lg text-xs font-mono font-medium text-slate-200 bg-slate-800/80 hover:bg-slate-700 border border-slate-700 transition-all cursor-pointer"
          >
            <Play className="w-3 h-3 text-cyan-400" />
            <span>Step</span>
          </button>

          <button
            type="button"
            onClick={onResetGraph}
            title="Reset DAG to initial demonstration state"
            className="p-1.5 rounded-lg text-slate-400 hover:text-white bg-slate-900 hover:bg-slate-800 border border-slate-800 transition-colors cursor-pointer"
          >
            <RotateCcw className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </header>
  );
}
