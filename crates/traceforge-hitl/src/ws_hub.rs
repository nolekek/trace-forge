//! # HITL WebSocket Control Hub
//!
//! Real-time WebSocket hub connecting TraceForge to Next.js UI dashboards,
//! broadcasting telemetry events and dispatching operator approval requests.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, warn};
use ulid::Ulid;

use traceforge_core::TraceEvent;

use crate::breakpoint::BreakpointManager;
use crate::state_machine::{HitlSessionState, SessionStateMachine, StateMachineError};

/// Default channel capacity for the WebSocket broadcast ring buffer.
pub const DEFAULT_BROADCAST_CAPACITY: usize = 2048;

/// Errors occurring within the HITL control plane.
#[derive(Debug, Error)]
pub enum HitlError {
    #[error("State machine error: {0}")]
    StateMachine(#[from] StateMachineError),

    #[error("Approval request timed out after {0:?}")]
    ApprovalTimeout(Duration),

    #[error("Approval request channel was cancelled or dropped")]
    ApprovalCancelled,

    #[error("Approval request '{0}' was not found or already answered")]
    ApprovalNotFound(String),

    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Operator decision responding to a requested agent tool execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Proceed with the tool execution unmodified.
    Allow,
    /// Prohibit tool execution with an optional explanation.
    Deny {
        #[serde(default)]
        reason: Option<String>,
    },
    /// Substitute tool arguments with operator-modified parameters.
    Modify {
        modified_args: serde_json::Value,
    },
}

/// Structured request awaiting human verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub session_id: String,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub reason: String,
    pub timestamp_ms: u64,
}

impl ApprovalRequest {
    pub fn new(
        session_id: impl Into<String>,
        tool_name: impl Into<String>,
        tool_args: serde_json::Value,
        reason: impl Into<String>,
    ) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Self {
            id: Ulid::new().to_string(),
            session_id: session_id.into(),
            tool_name: tool_name.into(),
            tool_args,
            reason: reason.into(),
            timestamp_ms,
        }
    }
}

/// Outbound WebSocket protocol messages emitted from TraceForge to UI clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsOutboundMessage {
    TraceEvent {
        event: TraceEvent,
    },
    ApprovalRequired {
        request: ApprovalRequest,
    },
    StateChanged {
        session_id: String,
        state: HitlSessionState,
    },
    BreakpointsList {
        breakpoints: Vec<String>,
    },
    Pong,
    Error {
        message: String,
    },
}

/// Inbound WebSocket protocol messages sent from UI clients to TraceForge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsInboundMessage {
    ApprovalResponse {
        request_id: String,
        decision: ApprovalDecision,
    },
    SetBreakpoint {
        tool_name: String,
    },
    RemoveBreakpoint {
        tool_name: String,
    },
    PauseSession {
        session_id: String,
    },
    ResumeSession {
        session_id: String,
    },
    ListBreakpoints,
    Ping,
}

/// Central WebSocket and HITL Control Hub coordinating real-time UI interactions.
pub struct WsHub {
    state_machine: Arc<SessionStateMachine>,
    breakpoint_manager: Arc<BreakpointManager>,
    broadcast_tx: broadcast::Sender<WsOutboundMessage>,
    pending_approvals: Arc<RwLock<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
}

impl Default for WsHub {
    fn default() -> Self {
        Self::new(
            Arc::new(SessionStateMachine::new()),
            Arc::new(BreakpointManager::new()),
        )
    }
}

impl WsHub {
    /// Creates a new `WsHub` with injected state machine and breakpoint manager.
    pub fn new(
        state_machine: Arc<SessionStateMachine>,
        breakpoint_manager: Arc<BreakpointManager>,
    ) -> Self {
        let (broadcast_tx, _) = broadcast::channel(DEFAULT_BROADCAST_CAPACITY);
        Self {
            state_machine,
            breakpoint_manager,
            broadcast_tx,
            pending_approvals: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn state_machine(&self) -> &Arc<SessionStateMachine> {
        &self.state_machine
    }

    pub fn breakpoint_manager(&self) -> &Arc<BreakpointManager> {
        &self.breakpoint_manager
    }

    /// Subscribes to the broadcast channel for outbound control plane messages.
    pub fn subscribe(&self) -> broadcast::Receiver<WsOutboundMessage> {
        self.broadcast_tx.subscribe()
    }

    /// Broadcasts an observability trace event to all connected WebSocket clients.
    pub fn broadcast_event(&self, event: TraceEvent) {
        let _ = self.broadcast_tx.send(WsOutboundMessage::TraceEvent { event });
    }

    /// Broadcasts any outbound control message to all connected WebSocket clients.
    pub fn broadcast_message(&self, message: WsOutboundMessage) {
        let _ = self.broadcast_tx.send(message);
    }

    /// Registers a pending approval request and blocks asynchronously until an operator responds
    /// via WebSocket or the optional timeout expires.
    pub async fn request_approval(
        &self,
        session_id: &str,
        tool_name: &str,
        tool_args: serde_json::Value,
        reason: &str,
        timeout_opt: Option<Duration>,
    ) -> Result<ApprovalDecision, HitlError> {
        let req = ApprovalRequest::new(session_id, tool_name, tool_args, reason);
        let request_id = req.id.clone();
        let (tx, rx) = oneshot::channel::<ApprovalDecision>();

        // Register pending oneshot channel
        {
            let mut guard = self.pending_approvals.write().unwrap();
            guard.insert(request_id.clone(), tx);
        }

        // Transition session state to AWAITING_APPROVAL
        self.state_machine.enter_approval(session_id)?;
        self.broadcast_message(WsOutboundMessage::StateChanged {
            session_id: session_id.to_string(),
            state: HitlSessionState::AwaitingApproval,
        });

        // Broadcast approval prompt to all connected UI clients
        self.broadcast_message(WsOutboundMessage::ApprovalRequired {
            request: req.clone(),
        });

        // Await decision from human operator
        let decision_result: Result<ApprovalDecision, HitlError> = match timeout_opt {
            Some(duration) => match tokio::time::timeout(duration, rx).await {
                Ok(Ok(decision)) => Ok(decision),
                Ok(Err(_)) => Err(HitlError::ApprovalCancelled),
                Err(_) => {
                    let mut guard = self.pending_approvals.write().unwrap();
                    guard.remove(&request_id);
                    Err(HitlError::ApprovalTimeout(duration))
                }
            },
            None => match rx.await {
                Ok(decision) => Ok(decision),
                Err(_) => Err(HitlError::ApprovalCancelled),
            },
        };

        match decision_result {
            Ok(decision) => {
                match &decision {
                    ApprovalDecision::Allow | ApprovalDecision::Modify { .. } => {
                        let _ = self.state_machine.resolve_approval_allow(session_id);
                        self.broadcast_message(WsOutboundMessage::StateChanged {
                            session_id: session_id.to_string(),
                            state: HitlSessionState::Running,
                        });
                    }
                    ApprovalDecision::Deny { reason } => {
                        let denial_reason = reason.clone().unwrap_or_else(|| "Rejected by human operator".to_string());
                        let _ = self.state_machine.resolve_approval_deny(session_id, &denial_reason);
                        self.broadcast_message(WsOutboundMessage::StateChanged {
                            session_id: session_id.to_string(),
                            state: HitlSessionState::CircuitTripped,
                        });
                    }
                }
                Ok(decision)
            }
            Err(e) => {
                let _ = self.state_machine.trip_circuit(session_id, format!("Approval failed: {e}"));
                self.broadcast_message(WsOutboundMessage::StateChanged {
                    session_id: session_id.to_string(),
                    state: HitlSessionState::CircuitTripped,
                });
                Err(e)
            }
        }
    }

    /// Resolves an active approval request with the given decision.
    pub fn resolve_approval(&self, request_id: &str, decision: ApprovalDecision) -> bool {
        let sender_opt = {
            let mut guard = self.pending_approvals.write().unwrap();
            guard.remove(request_id)
        };

        if let Some(tx) = sender_opt {
            tx.send(decision).is_ok()
        } else {
            false
        }
    }

    /// Handles an established WebSocket connection.
    pub async fn handle_socket(self: Arc<Self>, mut socket: WebSocket) {
        let mut rx_broadcast = self.broadcast_tx.subscribe();

        loop {
            tokio::select! {
                // Outbound messages to client
                broadcast_msg = rx_broadcast.recv() => {
                    match broadcast_msg {
                        Ok(msg) => {
                            if let Ok(json_str) = serde_json::to_string(&msg) {
                                if socket.send(Message::from(json_str)).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!("WebSocket client lagged, missed {} messages", skipped);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }

                // Inbound messages from client
                client_msg = socket.recv() => {
                    match client_msg {
                        Some(Ok(msg)) => {
                            if let Ok(text) = msg.to_text() {
                                self.handle_incoming_text(text, &mut socket).await;
                            }
                        }
                        _ => {
                            break;
                        }
                    }
                }
            }
        }

        debug!("WebSocket client disconnected");
    }

    async fn handle_incoming_text(&self, text: &str, socket: &mut WebSocket) {
        let inbound: WsInboundMessage = match serde_json::from_str(text) {
            Ok(msg) => msg,
            Err(e) => {
                let err_msg = WsOutboundMessage::Error {
                    message: format!("Invalid message format: {e}"),
                };
                if let Ok(json) = serde_json::to_string(&err_msg) {
                    let _ = socket.send(Message::from(json)).await;
                }
                return;
            }
        };

        match inbound {
            WsInboundMessage::ApprovalResponse { request_id, decision } => {
                let success = self.resolve_approval(&request_id, decision);
                if !success {
                    let err_msg = WsOutboundMessage::Error {
                        message: format!("Approval request '{request_id}' not found or already answered"),
                    };
                    if let Ok(json) = serde_json::to_string(&err_msg) {
                        let _ = socket.send(Message::from(json)).await;
                    }
                }
            }
            WsInboundMessage::SetBreakpoint { tool_name } => {
                self.breakpoint_manager.add_breakpoint(&tool_name);
                let list = self.breakpoint_manager.list_breakpoints();
                self.broadcast_message(WsOutboundMessage::BreakpointsList {
                    breakpoints: list,
                });
            }
            WsInboundMessage::RemoveBreakpoint { tool_name } => {
                self.breakpoint_manager.remove_breakpoint(&tool_name);
                let list = self.breakpoint_manager.list_breakpoints();
                self.broadcast_message(WsOutboundMessage::BreakpointsList {
                    breakpoints: list,
                });
            }
            WsInboundMessage::ListBreakpoints => {
                let list = self.breakpoint_manager.list_breakpoints();
                let msg = WsOutboundMessage::BreakpointsList {
                    breakpoints: list,
                };
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = socket.send(Message::from(json)).await;
                }
            }
            WsInboundMessage::PauseSession { session_id } => {
                if self.state_machine.pause(&session_id).is_ok() {
                    self.broadcast_message(WsOutboundMessage::StateChanged {
                        session_id,
                        state: HitlSessionState::Paused,
                    });
                }
            }
            WsInboundMessage::ResumeSession { session_id } => {
                if self.state_machine.resume(&session_id).is_ok() {
                    self.broadcast_message(WsOutboundMessage::StateChanged {
                        session_id,
                        state: HitlSessionState::Running,
                    });
                }
            }
            WsInboundMessage::Ping => {
                let msg = WsOutboundMessage::Pong;
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = socket.send(Message::from(json)).await;
                }
            }
        }
    }
}

/// Axum WebSocket endpoint handler registering at `/ws/control`.
pub async fn ws_control_handler(
    State(hub): State<Arc<WsHub>>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| hub.handle_socket(socket))
}
