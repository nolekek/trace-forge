//! # HITL Session State Machine
//!
//! Manages execution states for active agent sessions (`RUNNING`, `PAUSED`, `AWAITING_APPROVAL`, `CIRCUIT_TRIPPED`).
//! Enforces valid state transitions and coordinates circuit breaker trips.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// Core execution states for an autonomous agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HitlSessionState {
    #[default]
    Running,
    Paused,
    AwaitingApproval,
    CircuitTripped,
}

impl std::fmt::Display for HitlSessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "RUNNING"),
            Self::Paused => write!(f, "PAUSED"),
            Self::AwaitingApproval => write!(f, "AWAITING_APPROVAL"),
            Self::CircuitTripped => write!(f, "CIRCUIT_TRIPPED"),
        }
    }
}

/// Errors raised during state machine transitions.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StateMachineError {
    #[error("Session '{session_id}' not found")]
    SessionNotFound { session_id: String },

    #[error("Invalid state transition for session '{session_id}': cannot move from {from} to {to}")]
    InvalidTransition {
        session_id: String,
        from: HitlSessionState,
        to: HitlSessionState,
    },

    #[error("Session '{session_id}' is currently paused")]
    SessionPaused { session_id: String },

    #[error("Session '{session_id}' circuit is tripped: {reason}")]
    CircuitTripped {
        session_id: String,
        reason: String,
    },
}

/// Thread-safe manager for tracking and transitioning agent session states.
#[derive(Debug, Clone, Default)]
pub struct SessionStateMachine {
    states: Arc<RwLock<HashMap<String, HitlSessionState>>>,
    tripped_reasons: Arc<RwLock<HashMap<String, String>>>,
}

impl SessionStateMachine {
    /// Creates a new empty `SessionStateMachine`.
    pub fn new() -> Self {
        Self {
            states: Arc::new(RwLock::new(HashMap::new())),
            tripped_reasons: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Retrieves the current state of a session, defaulting to `HitlSessionState::Running` if new.
    pub fn get_state(&self, session_id: &str) -> HitlSessionState {
        let guard = self.states.read().unwrap();
        guard.get(session_id).copied().unwrap_or(HitlSessionState::Running)
    }

    /// Sets the state for a session with transition validation.
    pub fn transition_to(
        &self,
        session_id: &str,
        to: HitlSessionState,
    ) -> Result<HitlSessionState, StateMachineError> {
        let mut guard = self.states.write().unwrap();
        let from = guard.get(session_id).copied().unwrap_or(HitlSessionState::Running);

        if from == to {
            return Ok(to);
        }

        if !Self::is_valid_transition(from, to) {
            return Err(StateMachineError::InvalidTransition {
                session_id: session_id.to_string(),
                from,
                to,
            });
        }

        guard.insert(session_id.to_string(), to);
        Ok(to)
    }

    /// Pauses an active session (`Running` -> `Paused`).
    pub fn pause(&self, session_id: &str) -> Result<(), StateMachineError> {
        self.transition_to(session_id, HitlSessionState::Paused)?;
        Ok(())
    }

    /// Resumes a paused session (`Paused` -> `Running`).
    pub fn resume(&self, session_id: &str) -> Result<(), StateMachineError> {
        self.transition_to(session_id, HitlSessionState::Running)?;
        Ok(())
    }

    /// Transitions session to `AwaitingApproval` when human intervention is triggered.
    pub fn enter_approval(&self, session_id: &str) -> Result<(), StateMachineError> {
        self.transition_to(session_id, HitlSessionState::AwaitingApproval)?;
        Ok(())
    }

    /// Resolves approval, returning session back to `Running`.
    pub fn resolve_approval_allow(&self, session_id: &str) -> Result<(), StateMachineError> {
        self.transition_to(session_id, HitlSessionState::Running)?;
        Ok(())
    }

    /// Rejects approval and trips circuit or returns to running.
    pub fn resolve_approval_deny(
        &self,
        session_id: &str,
        reason: impl Into<String>,
    ) -> Result<(), StateMachineError> {
        self.trip_circuit(session_id, reason)
    }

    /// Trips circuit breaker for the session, recording the failure reason.
    pub fn trip_circuit(
        &self,
        session_id: &str,
        reason: impl Into<String>,
    ) -> Result<(), StateMachineError> {
        let reason_str = reason.into();
        {
            let mut reasons_guard = self.tripped_reasons.write().unwrap();
            reasons_guard.insert(session_id.to_string(), reason_str);
        }
        self.transition_to(session_id, HitlSessionState::CircuitTripped)?;
        Ok(())
    }

    /// Resets a tripped circuit breaker back to `Running`.
    pub fn reset_circuit(&self, session_id: &str) -> Result<(), StateMachineError> {
        {
            let mut reasons_guard = self.tripped_reasons.write().unwrap();
            reasons_guard.remove(session_id);
        }
        self.transition_to(session_id, HitlSessionState::Running)?;
        Ok(())
    }

    /// Checks if a session is currently paused.
    pub fn is_paused(&self, session_id: &str) -> bool {
        self.get_state(session_id) == HitlSessionState::Paused
    }

    /// Checks if a session is currently in a circuit tripped state.
    pub fn is_tripped(&self, session_id: &str) -> bool {
        self.get_state(session_id) == HitlSessionState::CircuitTripped
    }

    /// Returns the reason why a session's circuit was tripped, if any.
    pub fn get_tripped_reason(&self, session_id: &str) -> Option<String> {
        let guard = self.tripped_reasons.read().unwrap();
        guard.get(session_id).cloned()
    }

    /// Verifies if a transition between two states is permitted.
    pub fn is_valid_transition(from: HitlSessionState, to: HitlSessionState) -> bool {
        match (from, to) {
            // Same state is always a no-op
            (a, b) if a == b => true,

            // From RUNNING: can pause, wait approval, or trip circuit
            (HitlSessionState::Running, HitlSessionState::Paused) => true,
            (HitlSessionState::Running, HitlSessionState::AwaitingApproval) => true,
            (HitlSessionState::Running, HitlSessionState::CircuitTripped) => true,

            // From PAUSED: can resume to Running or trip circuit
            (HitlSessionState::Paused, HitlSessionState::Running) => true,
            (HitlSessionState::Paused, HitlSessionState::CircuitTripped) => true,

            // From AWAITING_APPROVAL: can resume to Running, trip circuit, or pause
            (HitlSessionState::AwaitingApproval, HitlSessionState::Running) => true,
            (HitlSessionState::AwaitingApproval, HitlSessionState::CircuitTripped) => true,
            (HitlSessionState::AwaitingApproval, HitlSessionState::Paused) => true,

            // From CIRCUIT_TRIPPED: can manually reset to Running
            (HitlSessionState::CircuitTripped, HitlSessionState::Running) => true,

            // All other transitions are invalid
            _ => false,
        }
    }

    /// Clears all recorded states and reasons.
    pub fn clear(&self) {
        self.states.write().unwrap().clear();
        self.tripped_reasons.write().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_running() {
        let sm = SessionStateMachine::new();
        assert_eq!(sm.get_state("sess_1"), HitlSessionState::Running);
    }

    #[test]
    fn test_valid_transitions() {
        let sm = SessionStateMachine::new();

        // Running -> AwaitingApproval -> Running
        assert!(sm.enter_approval("s1").is_ok());
        assert_eq!(sm.get_state("s1"), HitlSessionState::AwaitingApproval);

        assert!(sm.resolve_approval_allow("s1").is_ok());
        assert_eq!(sm.get_state("s1"), HitlSessionState::Running);

        // Running -> Paused -> Running
        assert!(sm.pause("s1").is_ok());
        assert!(sm.is_paused("s1"));

        assert!(sm.resume("s1").is_ok());
        assert_eq!(sm.get_state("s1"), HitlSessionState::Running);

        // Running -> CircuitTripped -> Reset -> Running
        assert!(sm.trip_circuit("s1", "Policy violation").is_ok());
        assert!(sm.is_tripped("s1"));
        assert_eq!(sm.get_tripped_reason("s1").as_deref(), Some("Policy violation"));

        assert!(sm.reset_circuit("s1").is_ok());
        assert_eq!(sm.get_state("s1"), HitlSessionState::Running);
    }

    #[test]
    fn test_invalid_transition() {
        let sm = SessionStateMachine::new();
        assert!(sm.pause("s1").is_ok());
        // Paused cannot directly transition to AwaitingApproval without resuming
        let err = sm.transition_to("s1", HitlSessionState::AwaitingApproval);
        assert!(err.is_err());
        match err.unwrap_err() {
            StateMachineError::InvalidTransition { from, to, .. } => {
                assert_eq!(from, HitlSessionState::Paused);
                assert_eq!(to, HitlSessionState::AwaitingApproval);
            }
            _ => panic!("Expected InvalidTransition"),
        }
    }
}
