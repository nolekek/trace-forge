use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;
use traceforge_core::events::{EventKind, TraceEvent};
use traceforge_core::types::{PolicyResult, TokenUsage};
use traceforge_storage::{
    AsyncBatchFlusher, BatchFlusherConfig, SessionSummary, SqliteTraceStore, TraceStore,
};

#[tokio::test]
async fn test_record_single_event_and_query_by_session() {
    let store = SqliteTraceStore::in_memory().expect("in-memory db init");
    let session_id = "sess_single_test_01";

    let payload = serde_json::json!({
        "model": "gpt-4o",
        "temperature": 0.7,
        "query": "explain quantum computing"
    });

    let event = TraceEvent::llm_request(session_id, payload.clone())
        .with_latency_us(1250)
        .with_token_usage(TokenUsage::new(120, 60))
        .with_cost_usd(0.0035);

    store.record_event(&event).await.expect("record event");

    let events = store
        .get_session_events(session_id)
        .await
        .expect("get session events");

    assert_eq!(events.len(), 1);
    let retrieved = &events[0];
    assert_eq!(retrieved.event_id, event.event_id);
    assert_eq!(retrieved.session_id, session_id);
    assert_eq!(retrieved.kind, EventKind::LlmRequest);
    assert_eq!(retrieved.latency_us, 1250);
    assert_eq!(retrieved.cost_usd, 0.0035);

    let usage = retrieved.token_usage.as_ref().expect("token usage");
    assert_eq!(usage.prompt_tokens, 120);
    assert_eq!(usage.completion_tokens, 60);
    assert_eq!(usage.total_tokens, 180);
    assert_eq!(retrieved.payload, payload);
}

#[tokio::test]
async fn test_record_batch_and_session_summary() {
    let store = SqliteTraceStore::in_memory().expect("in-memory db init");
    let session_id = "sess_batch_summary_02";

    let mut events = Vec::new();

    // 2 LLM requests
    for i in 0..2 {
        events.push(
            TraceEvent::llm_request(session_id, serde_json::json!({"step": i}))
                .with_latency_us(1000)
                .with_token_usage(TokenUsage::new(150, 50))
                .with_cost_usd(0.005),
        );
    }

    // 1 Tool Call
    events.push(
        TraceEvent::tool_call(session_id, "call_abc", "web_search", "query: rust")
            .with_latency_us(250),
    );

    // 2 Loop Detections
    events.push(
        TraceEvent::loop_detected(session_id, "web_search", 0.96).with_latency_us(100),
    );
    events.push(
        TraceEvent::loop_detected(session_id, "web_search", 0.98).with_latency_us(100),
    );

    // 1 Policy Evaluation (Allowed)
    events.push(
        TraceEvent::policy_evaluation(
            session_id,
            PolicyResult::allow(),
            serde_json::json!({"tool": "web_search"}),
        )
        .with_latency_us(50),
    );

    // 1 Policy Evaluation (Denied with 2 violations)
    let mut denied_policy = PolicyResult::deny("sandbox_rules", "forbidden bash command");
    denied_policy.violations = vec![
        "attempted rm -rf".to_string(),
        "unauthorized directory access".to_string(),
    ];
    events.push(
        TraceEvent::policy_evaluation(
            session_id,
            denied_policy,
            serde_json::json!({"tool": "bash", "cmd": "rm -rf /"}),
        )
        .with_latency_us(50),
    );

    // 1 HITL action
    events.push(
        TraceEvent::hitl_action(session_id, "block", "operator rejected dangerous command")
            .with_latency_us(400),
    );

    assert_eq!(events.len(), 8);

    // Record the full batch
    store.record_batch(&events).await.expect("record batch");

    // Retrieve events and verify ordering
    let stored_events = store
        .get_session_events(session_id)
        .await
        .expect("get events");
    assert_eq!(stored_events.len(), 8);

    // Retrieve summary via SQLite aggregate query
    let summary = store
        .get_session_summary(session_id)
        .await
        .expect("get summary");

    assert_eq!(summary.total_events, 8);
    assert_eq!(summary.total_prompt_tokens, 300);
    assert_eq!(summary.total_completion_tokens, 100);
    assert!((summary.total_cost_usd - 0.010).abs() < 1e-6);
    assert_eq!(summary.loop_detections_count, 2);
    assert_eq!(summary.policy_violations_count, 2);

    // Average latency: (1000 + 1000 + 250 + 100 + 100 + 50 + 50 + 400) / 8 = 2950 / 8 = 368.75
    assert!((summary.avg_latency_us - 368.75).abs() < 1e-4);

    // Compare with Rust in-memory calculation
    let in_memory_summary = SessionSummary::from_events(&events);
    assert_eq!(summary, in_memory_summary);

    // Non-existent session returns zeroed summary
    let empty_summary = store
        .get_session_summary("non_existent_session")
        .await
        .expect("empty summary");
    assert_eq!(empty_summary, SessionSummary::default());
}

#[tokio::test]
async fn test_async_batch_flusher_500_events() {
    let store = Arc::new(SqliteTraceStore::in_memory().expect("in-memory db init"));
    let flusher = AsyncBatchFlusher::new(store.clone());

    let session_id = "sess_flusher_500_test";
    let event_count = 500;

    let send_start = Instant::now();
    for i in 0..event_count {
        let event = TraceEvent::llm_request(
            session_id,
            serde_json::json!({ "chunk": i, "content": "stream token payload" }),
        )
        .with_latency_us(50 + (i % 20) as u32)
        .with_token_usage(TokenUsage::new(10, 2))
        .with_cost_usd(0.0001);

        flusher.send(event).expect("send into flusher queue");
    }
    let send_duration = send_start.elapsed();
    let per_event_overhead_us = send_duration.as_micros() as f64 / event_count as f64;
    println!(
        "AsyncBatchFlusher: 500 events queued in {:?} (avg {:.3} us/event overhead)",
        send_duration, per_event_overhead_us
    );
    // Verify non-blocking requirement (< 50 us / 0.05ms)
    assert!(
        per_event_overhead_us < 50.0,
        "Overhead per send was {:.3}us, exceeding 50us budget",
        per_event_overhead_us
    );

    // Perform graceful shutdown and flush
    flusher
        .shutdown_and_flush()
        .await
        .expect("shutdown and flush");

    assert!(flusher.is_shutdown());

    // Verify all 500 events are persisted in the database
    let stored_events = store
        .get_session_events(session_id)
        .await
        .expect("get session events");
    assert_eq!(stored_events.len(), event_count);

    // Verify aggregated summary
    let summary = store
        .get_session_summary(session_id)
        .await
        .expect("get summary");
    assert_eq!(summary.total_events, 500);
    assert_eq!(summary.total_prompt_tokens, 500 * 10);
    assert_eq!(summary.total_completion_tokens, 500 * 2);
    assert!((summary.total_cost_usd - 0.05).abs() < 1e-4);
}

#[tokio::test]
async fn test_batch_flusher_timer_trigger() {
    let store = Arc::new(SqliteTraceStore::in_memory().expect("in-memory db init"));
    let config = BatchFlusherConfig {
        channel_capacity: 1000,
        batch_size: 1000, // Large batch size so timer triggers before batch size
        flush_interval: Duration::from_millis(50),
    };
    let flusher = AsyncBatchFlusher::with_config(store.clone(), config);

    let session_id = "sess_timer_flush_03";
    for i in 0..5 {
        flusher
            .send(TraceEvent::llm_request(session_id, serde_json::json!({"id": i})))
            .expect("send");
    }

    // Give the timer interval (50ms) time to fire
    tokio::time::sleep(Duration::from_millis(120)).await;

    // Verify that events have been written by the timer without explicit shutdown
    let events = store
        .get_session_events(session_id)
        .await
        .expect("read events");
    assert_eq!(events.len(), 5);

    flusher
        .shutdown_and_flush()
        .await
        .expect("clean shutdown");
}

#[tokio::test]
async fn test_sqlite_file_persistence_wal() {
    // Create a temporary file for SQLite persistence test
    let temp_file = NamedTempFile::new().expect("create temp file");
    let path = temp_file.path().to_path_buf();

    let session_id = "sess_wal_persistence";
    {
        let store = SqliteTraceStore::open(&path).expect("open file store");
        let events = vec![
            TraceEvent::llm_request(session_id, serde_json::json!({"query": "q1"}))
                .with_token_usage(TokenUsage::new(50, 25)),
            TraceEvent::tool_call(session_id, "t1", "bash", "ls -la"),
        ];
        store.record_batch(&events).await.expect("record batch");
    } // Store is dropped and connection closed here

    // Reopen same file database
    let reopened_store = SqliteTraceStore::open(&path).expect("reopen store");
    let events = reopened_store
        .get_session_events(session_id)
        .await
        .expect("get reopened events");

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, EventKind::LlmRequest);
    assert_eq!(events[1].kind, EventKind::ToolCall);

    let count = reopened_store.count_events().await.expect("count events");
    assert_eq!(count, 2);

    let sessions = reopened_store.list_sessions().await.expect("list sessions");
    assert_eq!(sessions, vec![session_id.to_string()]);
}

#[tokio::test]
async fn test_high_throughput_burst() {
    let store = Arc::new(SqliteTraceStore::in_memory().expect("in-memory db init"));
    let flusher = AsyncBatchFlusher::new(store.clone());

    let session_id = "sess_burst_1000";
    let burst_size = 1000;

    let start = Instant::now();
    for i in 0..burst_size {
        let evt = TraceEvent::llm_chunk(
            session_id,
            i,
            serde_json::json!({ "delta": "word" }),
        );
        flusher.send(evt).expect("burst send");
    }
    let enqueue_time = start.elapsed();

    flusher
        .shutdown_and_flush()
        .await
        .expect("flush all");
    let total_time = start.elapsed();

    let stored = store.get_session_events(session_id).await.expect("read stored");
    assert_eq!(stored.len(), burst_size);

    println!(
        "High throughput burst: 1,000 events enqueued in {:?} ({:.2} ns/event), fully flushed to SQLite in {:?}",
        enqueue_time,
        (enqueue_time.as_nanos() as f64) / (burst_size as f64),
        total_time
    );
}
