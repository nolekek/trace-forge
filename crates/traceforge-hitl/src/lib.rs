//! # TraceForge HITL (Human-In-The-Loop) Control Plane
//!
//! Provides execution state machine management, breakpoint inspection,
//! and real-time WebSocket communication for human verification and steering of LLM agents.

pub mod breakpoint;
pub mod state_machine;
pub mod ws_hub;

pub use breakpoint::BreakpointManager;
pub use state_machine::{HitlSessionState, SessionStateMachine, StateMachineError};
pub use ws_hub::{
    ws_control_handler, ApprovalDecision, ApprovalRequest, HitlError, WsHub, WsInboundMessage,
    WsOutboundMessage,
};
