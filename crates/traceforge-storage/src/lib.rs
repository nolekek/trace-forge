//! # TraceForge Storage
//!
//! Embedded trace storage and asynchronous telemetry flushing engine for TraceForge.
//!
//! Provides zero-infrastructure persistence with SQLite WAL mode, asynchronous
//! multi-channel batching with Tokio, and pluggable storage traits for enterprise backends.

pub mod batch_flusher;
pub mod models;
pub mod sqlite_backend;
pub mod traits;

// Re-export primary types for ergonomic usage
pub use batch_flusher::AsyncBatchFlusher;
pub use models::{BatchFlusherConfig, SessionSummary, StorageError};
pub use sqlite_backend::{SqliteTraceStore, DEFAULT_DB_PATH};
pub use traits::TraceStore;
