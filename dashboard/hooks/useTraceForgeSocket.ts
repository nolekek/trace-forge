'use client';

import { useState, useEffect, useRef, useCallback } from 'react';
import { Node, Edge } from '@xyflow/react';
import {
  ConnectionStatus,
  SessionState,
  TraceEvent,
  ApprovalRequest,
  ApprovalDecision,
  DagNodeData,
  BudgetTelemetry,
  ContextBreakdown,
} from '@/types/traceforge';
import {
  INITIAL_NODES,
  INITIAL_EDGES,
  INITIAL_APPROVAL_REQUEST,
  INITIAL_TELEMETRY,
  INITIAL_CONTEXT_BREAKDOWN,
  INITIAL_TRACE_EVENTS,
} from '@/lib/mockData';

const DEFAULT_WS_URL =
  typeof window !== 'undefined'
    ? `ws://${window.location.hostname || 'localhost'}:8080/ws/control`
    : 'ws://localhost:8080/ws/control';

export function useTraceForgeSocket(url = DEFAULT_WS_URL) {
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>('disconnected');
  const [reconnectAttempt, setReconnectAttempt] = useState(0);
  const [sessionId, setSessionId] = useState('sess_89a1f4b2');
  const [sessionState, setSessionState] = useState<SessionState>('AWAITING_APPROVAL');

  const [nodes, setNodes] = useState<Node<DagNodeData>[]>(INITIAL_NODES);
  const [edges, setEdges] = useState<Edge[]>(INITIAL_EDGES);
  const [events, setEvents] = useState<TraceEvent[]>(INITIAL_TRACE_EVENTS);
  const [approvalRequest, setApprovalRequest] = useState<ApprovalRequest | null>(
    INITIAL_APPROVAL_REQUEST
  );

  const [budgetTelemetry, setBudgetTelemetry] = useState<BudgetTelemetry>(INITIAL_TELEMETRY);
  const [contextBreakdown, setContextBreakdown] =
    useState<ContextBreakdown>(INITIAL_CONTEXT_BREAKDOWN);

  const [breakpoints, setBreakpoints] = useState<Set<string>>(
    new Set(['execute_bash'])
  );

  const socketRef = useRef<WebSocket | null>(null);
  const reconnectTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const shouldReconnectRef = useRef(true);

  // Send message helper over WebSocket
  const sendMessage = useCallback((msg: Record<string, unknown>) => {
    if (socketRef.current && socketRef.current.readyState === WebSocket.OPEN) {
      socketRef.current.send(JSON.stringify(msg));
      return true;
    }
    return false;
  }, []);

  // Dispatch incoming WebSocket message
  const handleIncomingMessage = useCallback(
    (eventData: string) => {
      try {
        const data = JSON.parse(eventData);

        if (data.type === 'init') {
          if (data.session_id) setSessionId(data.session_id);
          if (data.state) setSessionState(data.state);
          if (data.budget) setBudgetTelemetry(data.budget);
          if (data.context) setContextBreakdown(data.context);
          if (Array.isArray(data.breakpoints)) {
            setBreakpoints(new Set(data.breakpoints));
          }
        } else if (data.type === 'trace_event') {
          const newEvt: TraceEvent = data.event;
          setEvents((prev) => [newEvt, ...prev].slice(0, 100));

          if (newEvt.cost_usd > 0) {
            setBudgetTelemetry((prev) => ({
              ...prev,
              spentUsd: +(prev.spentUsd + newEvt.cost_usd).toFixed(4),
            }));
          }
        } else if (data.type === 'approval_request') {
          setApprovalRequest(data.request);
          setSessionState('AWAITING_APPROVAL');
        } else if (data.type === 'state_change') {
          setSessionState(data.state);
        } else if (data.type === 'loop_alert') {
          // Loop detector alert
          const loopEvt: TraceEvent = {
            event_id: `loop_${Date.now()}`,
            session_id: sessionId,
            timestamp_ns: Date.now() * 1_000_000,
            latency_us: 120,
            kind: 'loop_detected',
            cost_usd: 0,
            payload: {
              tool_name: data.tool_name,
              similarity: data.similarity,
              repeats: data.consecutive_repeats,
            },
          };
          setEvents((prev) => [loopEvt, ...prev]);
        }
      } catch (err) {
        console.warn('Failed to parse incoming WebSocket message', err);
      }
    },
    [sessionId]
  );

  // WebSocket connection & exponential backoff
  useEffect(() => {
    shouldReconnectRef.current = true;

    function connect() {
      if (typeof window === 'undefined') return;

      try {
        setConnectionStatus((prev) => (prev === 'connected' ? 'reconnecting' : 'reconnecting'));
        const ws = new WebSocket(url);
        socketRef.current = ws;

        ws.onopen = () => {
          setConnectionStatus('connected');
          setReconnectAttempt(0);
          // Subscribe to session
          ws.send(JSON.stringify({ action: 'subscribe', session_id: sessionId }));
        };

        ws.onmessage = (e) => {
          handleIncomingMessage(e.data);
        };

        ws.onerror = () => {
          // Socket error will lead to close
        };

        ws.onclose = () => {
          setConnectionStatus('disconnected');
          if (shouldReconnectRef.current) {
            setReconnectAttempt((prev) => {
              const nextAttempt = prev + 1;
              const delay = Math.min(1000 * Math.pow(1.6, nextAttempt), 15000);
              reconnectTimeoutRef.current = setTimeout(connect, delay);
              return nextAttempt;
            });
          }
        };
      } catch {
        setConnectionStatus('disconnected');
      }
    }

    connect();

    return () => {
      shouldReconnectRef.current = false;
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
      }
      if (socketRef.current) {
        socketRef.current.close();
      }
    };
  }, [url, sessionId, handleIncomingMessage]);

  // Toggle Breakpoint on a Tool
  const toggleBreakpoint = useCallback(
    (toolName: string) => {
      setBreakpoints((prev) => {
        const next = new Set(prev);
        const willBeEnabled = !next.has(toolName);
        if (willBeEnabled) {
          next.add(toolName);
        } else {
          next.delete(toolName);
        }

        // Notify backend over WebSocket
        sendMessage({
          action: 'set_breakpoint',
          tool_name: toolName,
          enabled: willBeEnabled,
        });

        // Update existing node data in graph
        setNodes((currentNodes) =>
          currentNodes.map((n) => {
            if (n.type === 'tool_call' && n.data.tool_name === toolName) {
              return {
                ...n,
                data: {
                  ...n.data,
                  has_breakpoint: willBeEnabled,
                },
              };
            }
            return n;
          })
        );

        return next;
      });
    },
    [sendMessage]
  );

  // Send Operator Approval Decision
  const sendApproval = useCallback(
    (decision: ApprovalDecision) => {
      sendMessage({
        action: 'approval_decision',
        payload: decision,
      });

      // Update local state and graph
      setApprovalRequest(null);
      setSessionState(decision.decision === 'deny' ? 'TRIPPED' : 'RUNNING');

      // Update node states in graph
      setNodes((currentNodes) =>
        currentNodes.map((n) => {
          if (n.type === 'hitl_gate') {
            return {
              ...n,
              data: {
                ...n.data,
                status: decision.decision === 'approve' || decision.decision === 'modify' ? 'approved' : 'blocked',
                subtext:
                  decision.decision === 'approve'
                    ? 'Operator Approved'
                    : decision.decision === 'modify'
                    ? 'Modified & Approved'
                    : 'Denied by Operator',
              },
            };
          }
          if (n.type === 'tool_call' && n.data.status === 'awaiting_approval') {
            return {
              ...n,
              data: {
                ...n.data,
                status: decision.decision === 'approve' || decision.decision === 'modify' ? 'approved' : 'blocked',
                tool_args: decision.modified_command || n.data.tool_args,
              },
            };
          }
          return n;
        })
      );

      // Add a trace event for HITL decision
      const hitlEvt: TraceEvent = {
        event_id: `hitl_${Date.now()}`,
        session_id: sessionId,
        timestamp_ns: Date.now() * 1_000_000,
        latency_us: 250,
        kind: 'hitl_action',
        cost_usd: 0,
        payload: {
          decision: decision.decision,
          modified: Boolean(decision.modified_command),
          comment: decision.comment,
        },
      };
      setEvents((prev) => [hitlEvt, ...prev]);
    },
    [sendMessage, sessionId]
  );

  // Standalone Simulation Stepper
  const triggerMockStep = useCallback(() => {
    const stepNumber = nodes.length + 1;
    const newId = `node-tool-${stepNumber}`;
    const prevNode = nodes[nodes.length - 1];

    const newNode: Node<DagNodeData> = {
      id: newId,
      type: 'tool_call',
      position: {
        x: (prevNode?.position.x || 1000) + 320,
        y: (prevNode?.position.y || 200) + (Math.random() > 0.5 ? 40 : -40),
      },
      data: {
        id: newId,
        type: 'tool_call',
        label: `Tool: write_file`,
        subtext: 'Auto-executed',
        status: 'approved',
        timestamp: new Date().toLocaleTimeString(),
        latency_us: 1240,
        tool_name: 'write_file',
        tool_args: { path: '/var/log/audit.report.json', bytes: 1420 },
        has_breakpoint: breakpoints.has('write_file'),
      },
    };

    const newEdge: Edge = {
      id: `edge-${prevNode.id}-${newId}`,
      source: prevNode.id,
      target: newId,
      animated: true,
      style: { stroke: '#10b981', strokeWidth: 2 },
    };

    setNodes((prev) => [...prev, newNode]);
    setEdges((prev) => [...prev, newEdge]);

    // Update telemetry
    setBudgetTelemetry((prev) => ({
      ...prev,
      spentUsd: +(prev.spentUsd + 0.003).toFixed(4),
      totalTokens: prev.totalTokens + 380,
    }));

    setEvents((prev) => [
      {
        event_id: `evt_${Date.now()}`,
        session_id: sessionId,
        timestamp_ns: Date.now() * 1_000_000,
        latency_us: 1240,
        kind: 'tool_call',
        cost_usd: 0.003,
        payload: { tool_name: 'write_file', target: '/var/log/audit.report.json' },
      },
      ...prev,
    ]);
  }, [nodes, breakpoints, sessionId]);

  // Trigger Mock HITL Modal
  const triggerMockHitl = useCallback(() => {
    setApprovalRequest({
      id: `req_${Date.now()}`,
      session_id: sessionId,
      timestamp_ns: Date.now() * 1_000_000,
      tool_name: 'execute_bash',
      tool_args: { command: 'systemctl restart nginx.service' },
      rule_name: 'safety.service_mutation',
      reason: 'Agent is attempting service daemon manipulation requiring elevated sudo rights',
      risk_level: 'high',
      violations: ['Invariant check: safety.service_mutation tripped'],
    });
    setSessionState('AWAITING_APPROVAL');
  }, [sessionId]);

  const resetGraph = useCallback(() => {
    setNodes(INITIAL_NODES);
    setEdges(INITIAL_EDGES);
    setApprovalRequest(INITIAL_APPROVAL_REQUEST);
    setSessionState('AWAITING_APPROVAL');
  }, []);

  return {
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
    breakpoints,
    setNodes,
    setEdges,
    toggleBreakpoint,
    sendApproval,
    triggerMockStep,
    triggerMockHitl,
    resetGraph,
  };
}
