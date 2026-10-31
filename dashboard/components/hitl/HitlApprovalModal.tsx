'use client';

import React, { useState, useEffect } from 'react';
import {
  ShieldAlert,
  AlertTriangle,
  CheckCircle2,
  XCircle,
  Edit3,
  Terminal,
  Clock,
  Layers,
  Sparkles,
  X,
} from 'lucide-react';
import { ApprovalRequest, ApprovalDecision } from '@/types/traceforge';
import { cn } from '@/lib/utils';

interface HitlApprovalModalProps {
  request: ApprovalRequest | null;
  onDecision: (decision: ApprovalDecision) => void;
  onClose?: () => void;
}

export function HitlApprovalModal({
  request,
  onDecision,
  onClose,
}: HitlApprovalModalProps) {
  const [isModifying, setIsModifying] = useState(false);
  const [adjustedCommand, setAdjustedCommand] = useState('');
  const [operatorComment, setOperatorComment] = useState('');

  // Synchronize adjusted command whenever request changes
  useEffect(() => {
    if (request) {
      if (typeof request.tool_args === 'string') {
        setAdjustedCommand(request.tool_args);
      } else if (request.tool_args && 'command' in request.tool_args) {
        setAdjustedCommand(String(request.tool_args.command));
      } else {
        setAdjustedCommand(JSON.stringify(request.tool_args, null, 2));
      }
      setIsModifying(false);
      setOperatorComment('');
    }
  }, [request]);

  if (!request) return null;

  const isCritical = request.risk_level === 'critical';

  const handleApprove = () => {
    if (isModifying) {
      onDecision({
        request_id: request.id,
        decision: 'modify',
        modified_command: adjustedCommand,
        comment: operatorComment || 'Adjusted and approved by operator',
      });
    } else {
      onDecision({
        request_id: request.id,
        decision: 'approve',
        comment: operatorComment || 'Approved by operator',
      });
    }
  };

  const handleDeny = () => {
    onDecision({
      request_id: request.id,
      decision: 'deny',
      comment: operatorComment || 'Blocked by operator invariant policy',
    });
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="hitl-modal-title"
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-950/80 backdrop-blur-md animate-fadeIn"
    >
      <div className="relative w-full max-w-2xl bg-slate-900 border-2 border-amber-500/80 rounded-2xl shadow-2xl shadow-amber-500/20 overflow-hidden flex flex-col">
        {/* Modal Top Banner */}
        <div className="bg-amber-950/70 border-b border-amber-800/60 p-4 flex items-center justify-between">
          <div className="flex items-center space-x-3">
            <div
              className={cn(
                'p-2 rounded-xl border flex items-center justify-center',
                isCritical
                  ? 'bg-rose-950/90 text-rose-400 border-rose-700/60 animate-pulse'
                  : 'bg-amber-950/90 text-amber-400 border-amber-700/60'
              )}
            >
              {isCritical ? (
                <ShieldAlert className="w-5 h-5 text-rose-400" />
              ) : (
                <AlertTriangle className="w-5 h-5 text-amber-400" />
              )}
            </div>
            <div>
              <div className="flex items-center space-x-2">
                <h3
                  id="hitl-modal-title"
                  className="text-base font-bold text-slate-100 tracking-tight"
                >
                  Human-In-The-Loop Approval Gate
                </h3>
                <span
                  className={cn(
                    'text-[10px] font-mono font-bold uppercase px-2 py-0.5 rounded-full border',
                    isCritical
                      ? 'bg-rose-950/80 text-rose-300 border-rose-600 animate-pulse-rose'
                      : 'bg-amber-950/80 text-amber-300 border-amber-600'
                  )}
                >
                  {request.risk_level} RISK
                </span>
              </div>
              <span className="text-xs font-mono text-slate-400">
                Session: <strong className="text-slate-300">{request.session_id}</strong> • Target Tool: <strong className="text-cyan-300">{request.tool_name}</strong>
              </span>
            </div>
          </div>

          {onClose && (
            <button
              type="button"
              onClick={onClose}
              aria-label="Close modal"
              className="p-2 rounded-lg text-slate-400 hover:text-white hover:bg-slate-800 transition-colors cursor-pointer"
            >
              <X className="w-5 h-5" />
            </button>
          )}
        </div>

        {/* Modal Content */}
        <div className="p-6 space-y-4 max-h-[75vh] overflow-y-auto">
          {/* Policy Trigger Banner */}
          <div className="p-3.5 rounded-xl bg-slate-950 border border-amber-500/30 space-y-2">
            <div className="flex items-center justify-between text-xs font-mono">
              <span className="text-slate-400">Triggered Policy Rule:</span>
              <span className="text-amber-400 font-bold bg-amber-950/80 px-2 py-0.5 rounded border border-amber-700/40">
                {request.rule_name}
              </span>
            </div>
            <p className="text-xs text-slate-200 font-sans leading-relaxed">
              {request.reason}
            </p>
            {request.violations && request.violations.length > 0 && (
              <div className="pt-2 border-t border-slate-800/80 space-y-1">
                <span className="text-[10px] font-mono uppercase text-slate-500">
                  Violations Audit:
                </span>
                {request.violations.map((v, i) => (
                  <div key={i} className="text-[11px] font-mono text-rose-300 flex items-start space-x-1.5">
                    <span className="text-rose-400 font-bold">•</span>
                    <span>{v}</span>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Dangerous Command / Parameters Preview */}
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <label
                htmlFor="command-editor"
                className="text-xs font-mono uppercase tracking-wider text-slate-300 flex items-center space-x-1.5"
              >
                <Terminal className="w-3.5 h-3.5 text-cyan-400" />
                <span>Payload Arguments {isModifying ? '(Editable Mode)' : ''}</span>
              </label>

              <button
                type="button"
                onClick={() => setIsModifying(!isModifying)}
                className="flex items-center space-x-1 text-xs font-mono text-cyan-400 hover:text-cyan-300 px-2 py-1 rounded bg-slate-800/60 hover:bg-slate-800 border border-slate-700 cursor-pointer transition-colors"
              >
                <Edit3 className="w-3 h-3" />
                <span>{isModifying ? 'Revert to Original' : 'Modify Command'}</span>
              </button>
            </div>

            {isModifying ? (
              <textarea
                id="command-editor"
                rows={4}
                value={adjustedCommand}
                onChange={(e) => setAdjustedCommand(e.target.value)}
                className="w-full bg-slate-950 border-2 border-cyan-500 rounded-xl p-3 font-mono text-xs text-emerald-300 focus:outline-none focus:ring-2 focus:ring-cyan-400/50"
                placeholder="Adjust the tool command before confirming..."
              />
            ) : (
              <div className="rounded-xl bg-slate-950 border border-slate-800 p-3 font-mono text-xs text-emerald-400 leading-relaxed overflow-x-auto select-text">
                <span className="text-slate-500 select-none">$ </span>
                {adjustedCommand}
              </div>
            )}
          </div>

          {/* Optional Operator Comment */}
          <div className="space-y-1.5">
            <label
              htmlFor="operator-comment"
              className="text-xs font-mono text-slate-400"
            >
              Operator Audit Note (Optional):
            </label>
            <input
              id="operator-comment"
              type="text"
              value={operatorComment}
              onChange={(e) => setOperatorComment(e.target.value)}
              placeholder="e.g. Verified target directory is non-critical test cache."
              className="w-full bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 text-xs text-slate-200 placeholder-slate-600 focus:outline-none focus:border-amber-400"
            />
          </div>
        </div>

        {/* Modal Actions Footer */}
        <div className="bg-slate-950 border-t border-slate-800 p-4 flex items-center justify-between">
          <div className="text-[11px] font-mono text-slate-500 flex items-center space-x-1">
            <Clock className="w-3 h-3" />
            <span>Agent execution is FROZEN until action taken</span>
          </div>

          <div className="flex items-center space-x-3">
            {/* Deny Button */}
            <button
              type="button"
              onClick={handleDeny}
              className="min-h-[44px] min-w-[120px] px-4 py-2.5 rounded-xl font-medium text-xs text-rose-300 bg-rose-950/80 hover:bg-rose-900 border border-rose-700/80 flex items-center justify-center space-x-2 transition-all cursor-pointer shadow-md hover:shadow-rose-900/40"
            >
              <XCircle className="w-4 h-4 text-rose-400" />
              <span>Заблокировать (Deny)</span>
            </button>

            {/* Approve Button */}
            <button
              type="button"
              onClick={handleApprove}
              className="min-h-[44px] min-w-[140px] px-5 py-2.5 rounded-xl font-bold text-xs text-slate-950 bg-emerald-400 hover:bg-emerald-300 flex items-center justify-center space-x-2 transition-all cursor-pointer shadow-lg shadow-emerald-500/20 hover:scale-[1.02]"
            >
              <CheckCircle2 className="w-4 h-4 text-slate-950" />
              <span>
                {isModifying ? 'Скорректировать и одобрить' : 'Одобрить действие (Approve)'}
              </span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
