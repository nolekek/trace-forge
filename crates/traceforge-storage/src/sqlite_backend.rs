//! # SQLite Storage Backend
//!
//! Embedded, zero-infrastructure trace storage engine powered by SQLite.
//! Supports WAL (Write-Ahead-Logging) mode with sub-millisecond commits,
//! optimized indexing, and both on-disk and in-memory operation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use async_trait::async_trait;
use rusqlite::Connection;
use tokio::sync::Mutex;
use traceforge_core::events::{EventKind, TraceEvent};

use crate::models::{SessionSummary, StorageError};
use crate::traits::TraceStore;

/// Default database file path for local trace storage.
pub const DEFAULT_DB_PATH: &str = ".traceforge/traces.db";

/// Embedded SQLite implementation of `TraceStore`.
#[derive(Clone)]
pub struct SqliteTraceStore {
    conn: Arc<Mutex<Connection>>,
    db_path: Option<PathBuf>,
}

impl SqliteTraceStore {
    /// Opens or creates an in-memory SQLite database (`:memory:`).
    /// Highly optimized for integration testing and ephemeral sessions.
    pub fn in_memory() -> Result<Self, StorageError> {
        let mut conn = Connection::open_in_memory()?;
        Self::init_schema(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: None,
        })
    }

    /// Alias for `in_memory()`.
    pub fn new_in_memory() -> Result<Self, StorageError> {
        Self::in_memory()
    }

    /// Opens or creates a file-backed SQLite database at the specified path.
    /// Automatically provisions parent directories if they do not exist.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let mut conn = Connection::open(path)?;
        Self::init_schema(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: Some(path.to_path_buf()),
        })
    }

    /// Opens the default SQLite database file at `.traceforge/traces.db`.
    pub fn open_default() -> Result<Self, StorageError> {
        Self::open(DEFAULT_DB_PATH)
    }

    /// Returns the database path, or `None` if running purely in-memory.
    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    /// Configures pragmas and initializes required schema tables and indexes.
    fn init_schema(conn: &mut Connection) -> Result<(), StorageError> {
        // Configure pragmas safely ignoring returned pragma status rows
        let _ = conn.query_row("PRAGMA journal_mode = WAL;", [], |_| Ok(()));
        let _ = conn.query_row("PRAGMA synchronous = NORMAL;", [], |_| Ok(()));
        let _ = conn.query_row("PRAGMA foreign_keys = ON;", [], |_| Ok(()));
        let _ = conn.busy_timeout(std::time::Duration::from_millis(5000));

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS trace_events (
                event_id TEXT PRIMARY KEY NOT NULL,
                session_id TEXT NOT NULL,
                timestamp_ns INTEGER NOT NULL,
                latency_us INTEGER NOT NULL,
                event_kind TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                cost_usd REAL NOT NULL DEFAULT 0.0,
                policy_allowed INTEGER,
                policy_violations INTEGER NOT NULL DEFAULT 0,
                payload_json TEXT NOT NULL,
                full_event_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_trace_events_session ON trace_events(session_id);
            CREATE INDEX IF NOT EXISTS idx_trace_events_timestamp ON trace_events(timestamp_ns);
            CREATE INDEX IF NOT EXISTS idx_trace_events_kind ON trace_events(event_kind);
            CREATE INDEX IF NOT EXISTS idx_trace_events_session_ts ON trace_events(session_id, timestamp_ns);
            "#
        )?;

        Ok(())
    }

    /// Counts total recorded events across all sessions in the store.
    pub async fn count_events(&self) -> Result<u64, StorageError> {
        let conn = self.conn.lock().await;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM trace_events", [], |r| r.get(0))?;
        Ok(count as u64)
    }

    /// Lists distinct session IDs stored in the database.
    pub async fn list_sessions(&self) -> Result<Vec<String>, StorageError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT session_id FROM trace_events ORDER BY session_id ASC"
        )?;
        let mut rows = stmt.query([])?;
        let mut sessions = Vec::new();
        while let Some(row) = rows.next()? {
            sessions.push(row.get(0)?);
        }
        Ok(sessions)
    }

    /// Clears all events for a specific session.
    pub async fn clear_session(&self, session_id: &str) -> Result<u64, StorageError> {
        let conn = self.conn.lock().await;
        let affected = conn.execute(
            "DELETE FROM trace_events WHERE session_id = ?1",
            rusqlite::params![session_id],
        )?;
        Ok(affected as u64)
    }

    /// Clears all events across the entire database.
    pub async fn clear_all(&self) -> Result<u64, StorageError> {
        let conn = self.conn.lock().await;
        let affected = conn.execute("DELETE FROM trace_events", [])?;
        Ok(affected as u64)
    }
}

fn kind_to_str(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::LlmRequest => "llm_request",
        EventKind::LlmChunk => "llm_chunk",
        EventKind::ToolCall => "tool_call",
        EventKind::PolicyEvaluation => "policy_evaluation",
        EventKind::LoopDetected => "loop_detected",
        EventKind::HitlAction => "hitl_action",
    }
}

#[async_trait]
impl TraceStore for SqliteTraceStore {
    async fn record_event(&self, event: &TraceEvent) -> Result<(), StorageError> {
        self.record_batch(std::slice::from_ref(event)).await
    }

    async fn record_batch(&self, events: &[TraceEvent]) -> Result<(), StorageError> {
        if events.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock().await;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                r#"
                INSERT OR REPLACE INTO trace_events (
                    event_id,
                    session_id,
                    timestamp_ns,
                    latency_us,
                    event_kind,
                    prompt_tokens,
                    completion_tokens,
                    total_tokens,
                    cost_usd,
                    policy_allowed,
                    policy_violations,
                    payload_json,
                    full_event_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                "#
            )?;

            for event in events {
                let event_id_str = event.event_id.to_string();
                let (prompt_tokens, completion_tokens, total_tokens) = match &event.token_usage {
                    Some(u) => (u.prompt_tokens as i64, u.completion_tokens as i64, u.total_tokens as i64),
                    None => (0i64, 0i64, 0i64),
                };

                let policy_allowed: Option<i64> = event.policy_result.as_ref().map(|p| if p.allow { 1 } else { 0 });
                let policy_violations: i64 = match &event.policy_result {
                    Some(p) => {
                        if !p.allow {
                            (p.violations.len() as i64).max(1)
                        } else {
                            p.violations.len() as i64
                        }
                    }
                    None => 0,
                };

                let event_kind_str = kind_to_str(&event.kind);
                let payload_json = serde_json::to_string(&event.payload)?;
                let full_event_json = serde_json::to_string(event)?;

                stmt.execute(rusqlite::params![
                    event_id_str,
                    event.session_id,
                    event.timestamp_ns as i64,
                    event.latency_us as i64,
                    event_kind_str,
                    prompt_tokens,
                    completion_tokens,
                    total_tokens,
                    event.cost_usd,
                    policy_allowed,
                    policy_violations,
                    payload_json,
                    full_event_json,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    async fn get_session_events(&self, session_id: &str) -> Result<Vec<TraceEvent>, StorageError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT full_event_json FROM trace_events WHERE session_id = ?1 ORDER BY timestamp_ns ASC, rowid ASC"
        )?;
        let mut rows = stmt.query(rusqlite::params![session_id])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            let json_str: String = row.get(0)?;
            let event: TraceEvent = serde_json::from_str(&json_str)?;
            events.push(event);
        }
        Ok(events)
    }

    async fn get_session_summary(&self, session_id: &str) -> Result<SessionSummary, StorageError> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            r#"
            SELECT 
                COUNT(*),
                COALESCE(SUM(prompt_tokens), 0),
                COALESCE(SUM(completion_tokens), 0),
                COALESCE(SUM(cost_usd), 0.0),
                COALESCE(AVG(latency_us), 0.0),
                COALESCE(SUM(CASE WHEN event_kind = 'loop_detected' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(policy_violations), 0)
            FROM trace_events
            WHERE session_id = ?1
            "#
        )?;

        let summary = stmt.query_row(rusqlite::params![session_id], |row| {
            let total_events: i64 = row.get(0)?;
            let total_prompt: i64 = row.get(1)?;
            let total_comp: i64 = row.get(2)?;
            let total_cost: f64 = row.get(3)?;
            let avg_latency: f64 = row.get(4)?;
            let loop_count: i64 = row.get(5)?;
            let policy_count: i64 = row.get(6)?;

            Ok(SessionSummary {
                total_events: total_events as u64,
                total_prompt_tokens: total_prompt as u64,
                total_completion_tokens: total_comp as u64,
                total_cost_usd: total_cost,
                avg_latency_us: avg_latency,
                loop_detections_count: loop_count as u64,
                policy_violations_count: policy_count as u64,
            })
        })?;

        Ok(summary)
    }
}
