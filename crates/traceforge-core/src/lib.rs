//! # TraceForge Core
//!
//! Foundational models, domain event types, configuration definitions,
//! and error primitives for the TraceForge streaming proxy and guard engine.

pub mod config;
pub mod errors;
pub mod events;
pub mod types;

pub use config::{BudgetConfig, LatencyThresholds, LoggingConfig, TraceForgeConfig};
pub use errors::{CoreError, Result};
pub use events::{EventKind, TraceEvent};
pub use types::{
    FunctionCall, LatencyMetrics, PolicyResult, SessionState, StreamChoice, StreamChunk,
    StreamDelta, StreamFunctionDelta, StreamToolCallDelta, TokenUsage, ToolCall,
};
