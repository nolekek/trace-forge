//! # Storage Traits
//!
//! Pluggable telemetry store interface for TraceForge.

use async_trait::async_trait;
use traceforge_core::events::TraceEvent;
use crate::models::{SessionSummary, StorageError};

/// Pluggable storage abstraction for recording and querying agent trace events.
#[async_trait]
pub trait TraceStore: Send + Sync + 'static {
    /// Records a single trace event into the backend storage.
    async fn record_event(&self, event: &TraceEvent) -> Result<(), StorageError> {
        self.record_batch(std::slice::from_ref(event)).await
    }

    /// Records a batch of trace events atomically within a single transaction.
    async fn record_batch(&self, events: &[TraceEvent]) -> Result<(), StorageError>;

    /// Retrieves all trace events for a given session, ordered chronologically.
    async fn get_session_events(&self, session_id: &str) -> Result<Vec<TraceEvent>, StorageError>;

    /// Calculates aggregated metrics and summary statistics for a given session.
    async fn get_session_summary(&self, session_id: &str) -> Result<SessionSummary, StorageError>;
}
