//! # Breakpoint Manager
//!
//! Provides granular execution breakpoints on tool calls (e.g. `execute_bash`),
//! enabling step-by-step agent debugging and Human-In-The-Loop review.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// Thread-safe manager for global and session-specific tool execution breakpoints.
#[derive(Debug, Clone, Default)]
pub struct BreakpointManager {
    global_tool_breakpoints: Arc<RwLock<HashSet<String>>>,
    session_tool_breakpoints: Arc<RwLock<HashSet<(String, String)>>>,
}

impl BreakpointManager {
    /// Creates a new empty `BreakpointManager`.
    pub fn new() -> Self {
        Self {
            global_tool_breakpoints: Arc::new(RwLock::new(HashSet::new())),
            session_tool_breakpoints: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Adds a global breakpoint on any call to `tool_name` across all sessions.
    pub fn add_breakpoint(&self, tool_name: impl AsRef<str>) {
        let normalized = tool_name.as_ref().trim().to_lowercase();
        if !normalized.is_empty() {
            let mut guard = self.global_tool_breakpoints.write().unwrap();
            guard.insert(normalized);
        }
    }

    /// Removes a global breakpoint on `tool_name`.
    pub fn remove_breakpoint(&self, tool_name: impl AsRef<str>) -> bool {
        let normalized = tool_name.as_ref().trim().to_lowercase();
        let mut guard = self.global_tool_breakpoints.write().unwrap();
        guard.remove(&normalized)
    }

    /// Checks if a global breakpoint is set on `tool_name`.
    pub fn has_breakpoint(&self, tool_name: impl AsRef<str>) -> bool {
        let normalized = tool_name.as_ref().trim().to_lowercase();
        let guard = self.global_tool_breakpoints.read().unwrap();
        guard.contains(&normalized)
    }

    /// Adds a session-specific breakpoint on a tool name.
    pub fn add_session_breakpoint(&self, session_id: impl AsRef<str>, tool_name: impl AsRef<str>) {
        let sess = session_id.as_ref().trim().to_string();
        let tool = tool_name.as_ref().trim().to_lowercase();
        if !sess.is_empty() && !tool.is_empty() {
            let mut guard = self.session_tool_breakpoints.write().unwrap();
            guard.insert((sess, tool));
        }
    }

    /// Removes a session-specific breakpoint.
    pub fn remove_session_breakpoint(
        &self,
        session_id: impl AsRef<str>,
        tool_name: impl AsRef<str>,
    ) -> bool {
        let sess = session_id.as_ref().trim();
        let tool = tool_name.as_ref().trim().to_lowercase();
        let mut guard = self.session_tool_breakpoints.write().unwrap();
        guard.remove(&(sess.to_string(), tool))
    }

    /// Determines whether execution should pause on this tool call.
    pub fn should_break(&self, session_id: &str, tool_name: &str) -> bool {
        let normalized_tool = tool_name.trim().to_lowercase();

        // Check global breakpoints
        {
            let global = self.global_tool_breakpoints.read().unwrap();
            if global.contains(&normalized_tool) {
                return true;
            }
        }

        // Check session-specific breakpoints
        {
            let session = self.session_tool_breakpoints.read().unwrap();
            if session.contains(&(session_id.trim().to_string(), normalized_tool)) {
                return true;
            }
        }

        false
    }

    /// Returns a list of all currently configured global tool breakpoints.
    pub fn list_breakpoints(&self) -> Vec<String> {
        let guard = self.global_tool_breakpoints.read().unwrap();
        let mut list: Vec<String> = guard.iter().cloned().collect();
        list.sort();
        list
    }

    /// Clears all global and session-specific breakpoints.
    pub fn clear(&self) {
        self.global_tool_breakpoints.write().unwrap().clear();
        self.session_tool_breakpoints.write().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_breakpoint_matching() {
        let bpm = BreakpointManager::new();
        bpm.add_breakpoint("execute_bash");

        assert!(bpm.has_breakpoint("execute_bash"));
        assert!(bpm.has_breakpoint("EXECUTE_BASH"));
        assert!(bpm.should_break("sess_1", "execute_bash"));
        assert!(bpm.should_break("sess_2", "execute_bash"));
        assert!(!bpm.should_break("sess_1", "read_file"));

        assert!(bpm.remove_breakpoint("execute_bash"));
        assert!(!bpm.has_breakpoint("execute_bash"));
        assert!(!bpm.should_break("sess_1", "execute_bash"));
    }

    #[test]
    fn test_session_breakpoint_matching() {
        let bpm = BreakpointManager::new();
        bpm.add_session_breakpoint("sess_target", "delete_file");

        assert!(bpm.should_break("sess_target", "delete_file"));
        assert!(!bpm.should_break("other_session", "delete_file"));
    }
}
