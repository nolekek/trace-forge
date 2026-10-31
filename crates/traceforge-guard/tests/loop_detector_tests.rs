use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use traceforge_guard::{
    cosine_similarity, ActionVectorizer, OnnxVectorizerAdapter, SemanticLoopDetector,
    DEFAULT_SIMILARITY_THRESHOLD, RECOVERY_PROMPT,
};

#[test]
fn test_loop_detection_positive_scenario_micro_variations() {
    let mut detector = SemanticLoopDetector::default();
    let session = "session_positive_test";

    // Call 1: initial action
    let res1 = detector.check_action(
        session,
        "cat",
        &json!({"command": "cat file.txt"}),
        Some("Let me read file.txt"),
    );
    assert!(!res1.is_loop, "First step should not be a loop");
    assert_eq!(res1.repeated_count, 1);
    assert_eq!(res1.similarity_score, 0.0);
    assert!(res1.recovery_prompt.is_none());

    // Call 2: micro-change in argument path: "./file.txt"
    let res2 = detector.check_action(
        session,
        "cat",
        &json!({"command": "cat ./file.txt"}),
        Some("Let me read ./file.txt"),
    );
    assert!(!res2.is_loop, "Second step should not trip threshold (need 3)");
    assert_eq!(res2.repeated_count, 2);
    assert!(
        res2.similarity_score >= DEFAULT_SIMILARITY_THRESHOLD,
        "Similarity should exceed threshold 0.92, got: {}",
        res2.similarity_score
    );
    assert!(res2.recovery_prompt.is_none());

    // Call 3: micro-change with extra whitespace: "cat  file.txt"
    let res3 = detector.check_action(
        session,
        "cat",
        &json!({"command": "cat  file.txt"}),
        Some("Let me read  file.txt"),
    );
    assert!(
        res3.is_loop,
        "Third consecutive repeated step MUST trip the loop detector"
    );
    assert_eq!(res3.repeated_count, 3);
    assert!(
        res3.similarity_score >= DEFAULT_SIMILARITY_THRESHOLD,
        "Similarity should exceed threshold 0.92, got: {}",
        res3.similarity_score
    );
    assert_eq!(
        res3.recovery_prompt.as_deref(),
        Some(RECOVERY_PROMPT),
        "Recovery prompt must match exact specification"
    );

    // Call 4: continuing the loop should continue reporting loop
    let res4 = detector.check_action(
        session,
        "cat",
        &json!({"command": "cat file.txt"}),
        None,
    );
    assert!(
        res4.is_loop,
        "Continuing repeating action must remain in loop state"
    );
    assert_eq!(res4.repeated_count, 4);
    assert!(res4.recovery_prompt.is_some());
}

#[test]
fn test_loop_detection_negative_scenario_diverse_actions() {
    let mut detector = SemanticLoopDetector::default();
    let session = "session_negative_test";

    let steps = vec![
        ("read_file", json!({"path": "src/main.rs"}), "Inspecting main module"),
        ("cargo_test", json!({"args": ["--lib"]}), "Running library unit tests"),
        ("git_status", json!({}), "Checking git working tree status"),
        ("edit_file", json!({"path": "src/main.rs", "line": 42}), "Updating logic"),
        ("cargo_check", json!({"workspace": true}), "Verifying workspace compiles"),
    ];

    for (tool, args, thought) in steps {
        let res = detector.check_action(session, tool, &args, Some(thought));
        assert!(
            !res.is_loop,
            "Valid distinct tool '{tool}' should never trigger a loop"
        );
        assert_eq!(
            res.repeated_count, 1,
            "Repeated count must remain 1 for tool '{tool}'"
        );
        assert!(
            res.similarity_score < DEFAULT_SIMILARITY_THRESHOLD,
            "Similarity for diverse actions should be well below 0.92, got {}",
            res.similarity_score
        );
        assert!(res.recovery_prompt.is_none());
    }
}

#[test]
fn test_sliding_window_eviction() {
    // Window size 3
    let mut detector = SemanticLoopDetector::new(3, 0.92, 3);
    let session = "session_window_eviction";

    for i in 1..=6 {
        let tool_name = format!("tool_step_{i}");
        detector.check_action(
            session,
            &tool_name,
            &json!({"step": i}),
            None,
        );
    }

    let history = detector.get_history(session).expect("History must exist");
    assert_eq!(
        history.len(),
        3,
        "History size must be capped at window_size=3"
    );

    // Steps 1..3 must have been evicted, steps 4..6 must be retained
    assert_eq!(history[0].tool_name, "tool_step_4");
    assert_eq!(history[1].tool_name, "tool_step_5");
    assert_eq!(history[2].tool_name, "tool_step_6");
}

#[test]
fn test_session_independence_and_reset() {
    let mut detector = SemanticLoopDetector::default();

    // Session A loops
    detector.check_action("sess_A", "bash", &json!({"cmd": "ls -la"}), None);
    detector.check_action("sess_A", "bash", &json!({"cmd": "ls  -la"}), None);
    let res_a = detector.check_action("sess_A", "bash", &json!({"cmd": "ls ./ -la"}), None);
    assert!(res_a.is_loop, "Session A must trigger loop");
    assert_eq!(res_a.repeated_count, 3);

    // Session B has distinct actions in parallel
    let res_b1 = detector.check_action("sess_B", "git_status", &json!({}), None);
    let res_b2 = detector.check_action("sess_B", "cargo_test", &json!({}), None);
    assert!(!res_b1.is_loop);
    assert!(!res_b2.is_loop);
    assert_eq!(res_b2.repeated_count, 1);

    // Resetting session A
    assert!(detector.reset_session("sess_A"));
    assert!(detector.get_history("sess_A").is_none());

    // Session B must be completely unaffected
    assert_eq!(
        detector.get_history("sess_B").map(|h| h.len()),
        Some(2),
        "Session B history must remain intact after resetting session A"
    );

    // Re-initiating session A should start with clean slate
    let res_a_new = detector.check_action("sess_A", "bash", &json!({"cmd": "ls -la"}), None);
    assert!(!res_a_new.is_loop);
    assert_eq!(res_a_new.repeated_count, 1);
}

#[test]
fn test_loop_break_and_recovery() {
    let mut detector = SemanticLoopDetector::default();
    let session = "session_recovery";

    // 3 repeating steps -> trip
    detector.check_action(session, "curl", &json!({"url": "http://api.internal/health"}), None);
    detector.check_action(session, "curl", &json!({"url": "http://api.internal/health"}), None);
    let tripped = detector.check_action(
        session,
        "curl",
        &json!({"url": "http://api.internal/health"}),
        None,
    );
    assert!(tripped.is_loop);
    assert_eq!(tripped.repeated_count, 3);

    // Agent changes strategy and reads configuration file instead
    let recovered = detector.check_action(
        session,
        "read_file",
        &json!({"path": "config/settings.yaml"}),
        Some("Changing strategy: inspecting configuration"),
    );
    assert!(
        !recovered.is_loop,
        "Loop must be cleared once strategy changes"
    );
    assert_eq!(recovered.repeated_count, 1);
    assert!(recovered.recovery_prompt.is_none());
}

#[test]
fn test_multithreaded_concurrency() {
    let detector = Arc::new(Mutex::new(SemanticLoopDetector::default()));
    let num_threads = 8;
    let iterations_per_thread = 20;

    let handles: Vec<_> = (0..num_threads)
        .map(|thread_id| {
            let detector_clone = Arc::clone(&detector);
            std::thread::spawn(move || {
                let session_id = format!("concurrent_sess_{thread_id}");
                for step in 0..iterations_per_thread {
                    let tool = if thread_id % 2 == 0 {
                        // Even threads: repeating tool calls
                        "repeat_tool"
                    } else {
                        // Odd threads: alternating tool calls
                        if step % 2 == 0 {
                            "tool_alpha"
                        } else {
                            "tool_beta"
                        }
                    };

                    let args = if thread_id % 2 == 0 {
                        json!({"action": "poll_status", "thread": thread_id})
                    } else {
                        json!({"action": format!("task_{step}"), "thread": thread_id})
                    };

                    let mut guard = detector_clone.lock().unwrap();
                    let res = guard.check_action(
                        &session_id,
                        tool,
                        &args,
                        None,
                    );

                    if thread_id % 2 == 0 && step >= 2 {
                        assert!(res.is_loop, "Thread {thread_id} step {step} should be loop");
                    } else if thread_id % 2 != 0 {
                        assert!(!res.is_loop, "Thread {thread_id} step {step} should not loop");
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread must not panic");
    }

    let final_guard = detector.lock().unwrap();
    assert_eq!(final_guard.session_count(), num_threads);
}

#[test]
fn test_cosine_similarity_edge_cases() {
    // Identical
    let v1 = vec![0.6f32, 0.8f32];
    assert!((cosine_similarity(&v1, &v1) - 1.0).abs() < 1e-5);

    // Opposite
    let v2 = vec![-0.6f32, -0.8f32];
    assert!((cosine_similarity(&v1, &v2) - (-1.0)).abs() < 1e-5);

    // Orthogonal
    let v3 = vec![-0.8f32, 0.6f32];
    assert!(cosine_similarity(&v1, &v3).abs() < 1e-5);

    // Zero vectors
    let zero = vec![0.0f32, 0.0f32];
    assert_eq!(cosine_similarity(&zero, &v1), 0.0);
    assert_eq!(cosine_similarity(&zero, &zero), 0.0);

    // Empty and mismatched lengths
    assert_eq!(cosine_similarity(&[], &v1), 0.0);
    assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
}

#[test]
fn test_onnx_adapter_with_mock_and_fallback() {
    // 1. Adapter with successful custom inference
    let adapter = OnnxVectorizerAdapter::with_inference_fn("mock_minilm", 384, |_text| {
        let mut v = vec![0.0f32; 384];
        v[0] = 1.0;
        Ok(v)
    });
    assert_eq!(adapter.name(), "mock_minilm");
    assert_eq!(adapter.dimension(), 384);
    let emb = adapter.vectorize("hello world");
    assert_eq!(emb.len(), 384);
    assert_eq!(emb[0], 1.0);

    // 2. Adapter with failing inference falls back to feature hashing
    let fallback_adapter = OnnxVectorizerAdapter::with_inference_fn("failing_minilm", 384, |_text| {
        Err("Inference session failure".to_string())
    });
    let fb_emb = fallback_adapter.vectorize("cat file.txt");
    assert_eq!(fb_emb.len(), 384);
    // Norm of fallback embedding must be ~1.0
    let norm: f32 = fb_emb.iter().map(|&x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-4);
}

#[test]
fn test_latency_benchmark_submillisecond() {
    let mut detector = SemanticLoopDetector::default();
    let session = "bench_session";
    let iterations = 1000;

    let start = Instant::now();
    for i in 0..iterations {
        let _ = detector.check_action(
            session,
            "execute_bash",
            &json!({"command": format!("cat src/loop_detector_{}.rs", i % 3)}),
            Some("Analyzing loop detection latency in sub-millisecond benchmark"),
        );
    }
    let elapsed = start.elapsed();
    let total_micros = elapsed.as_micros() as f64;
    let avg_micros_per_check = total_micros / iterations as f64;

    println!(
        "\n⚡ Latency Benchmark: {} iterations in {:.2?} (avg: {:.2} µs / {:.4} ms per check)",
        iterations, elapsed, avg_micros_per_check, avg_micros_per_check / 1000.0
    );

    // SLA: sub-millisecond (< 1000 µs).
    // In practice, this runs in < 25 µs (0.025 ms), well below 1.0 ms!
    assert!(
        avg_micros_per_check < 1000.0,
        "Check action must be sub-millisecond (< 1000 µs), was: {:.2} µs",
        avg_micros_per_check
    );
}
