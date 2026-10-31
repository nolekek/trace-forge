use std::sync::Arc;
use std::time::Duration;

use traceforge_core::TraceEvent;
use traceforge_hitl::{
    ApprovalDecision, BreakpointManager, HitlError, HitlSessionState,
    SessionStateMachine, WsHub, WsOutboundMessage,
};

#[tokio::test]
async fn test_state_machine_lifecycle() {
    let sm = SessionStateMachine::new();
    let session = "sess_test_1";

    assert_eq!(sm.get_state(session), HitlSessionState::Running);

    // Pause & Resume
    sm.pause(session).expect("pause session");
    assert_eq!(sm.get_state(session), HitlSessionState::Paused);
    assert!(sm.is_paused(session));

    sm.resume(session).expect("resume session");
    assert_eq!(sm.get_state(session), HitlSessionState::Running);

    // Enter approval and allow
    sm.enter_approval(session).expect("enter approval");
    assert_eq!(sm.get_state(session), HitlSessionState::AwaitingApproval);

    sm.resolve_approval_allow(session).expect("allow approval");
    assert_eq!(sm.get_state(session), HitlSessionState::Running);

    // Trip circuit
    sm.trip_circuit(session, "Destructive action blocked").expect("trip circuit");
    assert_eq!(sm.get_state(session), HitlSessionState::CircuitTripped);
    assert!(sm.is_tripped(session));
    assert_eq!(
        sm.get_tripped_reason(session).as_deref(),
        Some("Destructive action blocked")
    );

    // Reset circuit
    sm.reset_circuit(session).expect("reset circuit");
    assert_eq!(sm.get_state(session), HitlSessionState::Running);
    assert_eq!(sm.get_tripped_reason(session), None);
}

#[tokio::test]
async fn test_breakpoint_manager() {
    let bpm = BreakpointManager::new();

    bpm.add_breakpoint("execute_bash");
    bpm.add_breakpoint("delete_database");

    assert!(bpm.has_breakpoint("execute_bash"));
    assert!(bpm.has_breakpoint("EXECUTE_BASH"));
    assert!(bpm.should_break("s1", "execute_bash"));
    assert!(bpm.should_break("s2", "execute_bash"));
    assert!(!bpm.should_break("s1", "read_file"));

    let list = bpm.list_breakpoints();
    assert_eq!(list, vec!["delete_database", "execute_bash"]);

    assert!(bpm.remove_breakpoint("execute_bash"));
    assert!(!bpm.has_breakpoint("execute_bash"));
    assert!(!bpm.should_break("s1", "execute_bash"));

    // Session breakpoint
    bpm.add_session_breakpoint("sess_special", "read_file");
    assert!(bpm.should_break("sess_special", "read_file"));
    assert!(!bpm.should_break("sess_other", "read_file"));
}

#[tokio::test]
async fn test_ws_hub_approval_flow_allow() {
    let hub = Arc::new(WsHub::default());
    let hub_clone = hub.clone();

    let session_id = "sess_hitl_allow";
    let tool_name = "execute_bash";
    let tool_args = serde_json::json!({"command": "ls -la"});

    // Spawn async background task to simulate operator answering approval
    tokio::spawn(async move {
        // Wait briefly for request to register
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Subscribe or find pending approval
        // Wait until approval is in state machine
        while hub_clone.state_machine().get_state(session_id) != HitlSessionState::AwaitingApproval {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // We simulate reading the broadcast or resolving the active approval
        // Since we know the session entered approval, find and resolve it
        // Or inspect the pending approval ID
        // For testing, we can simulate an operator who approves
    });

    // Instead of polling state, let's use broadcast channel to listen for ApprovalRequired
    let mut rx = hub.subscribe();
    let hub_worker = hub.clone();

    tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let WsOutboundMessage::ApprovalRequired { request } = msg {
                assert_eq!(request.session_id, session_id);
                assert_eq!(request.tool_name, tool_name);
                hub_worker.resolve_approval(&request.id, ApprovalDecision::Allow);
                break;
            }
        }
    });

    let decision = hub
        .request_approval(
            session_id,
            tool_name,
            tool_args,
            "Needs human verification",
            Some(Duration::from_secs(2)),
        )
        .await
        .expect("approval resolved");

    assert_eq!(decision, ApprovalDecision::Allow);
    assert_eq!(hub.state_machine().get_state(session_id), HitlSessionState::Running);
}

#[tokio::test]
async fn test_ws_hub_approval_flow_deny() {
    let hub = Arc::new(WsHub::default());
    let mut rx = hub.subscribe();
    let hub_worker = hub.clone();

    let session_id = "sess_hitl_deny";

    tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let WsOutboundMessage::ApprovalRequired { request } = msg {
                hub_worker.resolve_approval(
                    &request.id,
                    ApprovalDecision::Deny {
                        reason: Some("Operation dangerous".to_string()),
                    },
                );
                break;
            }
        }
    });

    let decision = hub
        .request_approval(
            session_id,
            "execute_bash",
            serde_json::json!({"command": "rm -rf /"}),
            "Destructive command detected",
            Some(Duration::from_secs(2)),
        )
        .await
        .expect("approval processed");

    match decision {
        ApprovalDecision::Deny { reason } => {
            assert_eq!(reason.as_deref(), Some("Operation dangerous"));
        }
        _ => panic!("Expected Deny decision"),
    }

    assert_eq!(
        hub.state_machine().get_state(session_id),
        HitlSessionState::CircuitTripped
    );
}

#[tokio::test]
async fn test_ws_hub_approval_flow_modify() {
    let hub = Arc::new(WsHub::default());
    let mut rx = hub.subscribe();
    let hub_worker = hub.clone();

    let session_id = "sess_hitl_modify";

    tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let WsOutboundMessage::ApprovalRequired { request } = msg {
                hub_worker.resolve_approval(
                    &request.id,
                    ApprovalDecision::Modify {
                        modified_args: serde_json::json!({"command": "echo safe"}),
                    },
                );
                break;
            }
        }
    });

    let decision = hub
        .request_approval(
            session_id,
            "execute_bash",
            serde_json::json!({"command": "echo original"}),
            "Modify check",
            Some(Duration::from_secs(2)),
        )
        .await
        .expect("approval processed");

    match decision {
        ApprovalDecision::Modify { modified_args } => {
            assert_eq!(modified_args["command"], "echo safe");
        }
        _ => panic!("Expected Modify decision"),
    }

    assert_eq!(hub.state_machine().get_state(session_id), HitlSessionState::Running);
}

#[tokio::test]
async fn test_ws_hub_approval_timeout() {
    let hub = Arc::new(WsHub::default());
    let session_id = "sess_timeout";

    let result = hub
        .request_approval(
            session_id,
            "execute_bash",
            serde_json::json!({}),
            "Will timeout",
            Some(Duration::from_millis(50)),
        )
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        HitlError::ApprovalTimeout(_) => (),
        other => panic!("Expected ApprovalTimeout, got: {:?}", other),
    }

    assert_eq!(
        hub.state_machine().get_state(session_id),
        HitlSessionState::CircuitTripped
    );
}

#[tokio::test]
async fn test_ws_hub_broadcast_event() {
    let hub = Arc::new(WsHub::default());
    let mut rx = hub.subscribe();

    let evt = TraceEvent::tool_call("s1", "call_1", "execute_bash", "ls");
    hub.broadcast_event(evt.clone());

    let received = rx.recv().await.expect("received broadcast");
    match received {
        WsOutboundMessage::TraceEvent { event } => {
            assert_eq!(event.session_id, "s1");
            assert_eq!(event.payload["tool_name"], "execute_bash");
        }
        _ => panic!("Expected TraceEvent message"),
    }
}
