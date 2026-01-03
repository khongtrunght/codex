//! Session Mode Context.
//!
//! Holds the current session workflow mode and related tracking state.
//! This tracks workflow state only - approval behavior is handled separately
//! by AskForApproval and SandboxPolicy in the approval system.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::session_mode::SessionMode;

/// Session-level mode context.
///
/// Tracks the current workflow mode and related state.
/// Approval behavior is handled separately by AskForApproval/SandboxPolicy.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionModeContext {
    /// Current session mode (workflow state).
    pub mode: SessionMode,

    /// Track if plan mode has been exited (for reentry detection).
    #[serde(default)]
    pub has_exited_plan_mode: bool,

    /// Plan slug for memorable file naming (e.g., "atomic-marinating-pumpkin").
    /// Stored per-session so it persists across plan mode entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_slug: Option<String>,
}

impl Default for SessionModeContext {
    fn default() -> Self {
        Self {
            mode: SessionMode::Default,
            has_exited_plan_mode: false,
            plan_slug: None,
        }
    }
}

impl SessionModeContext {
    /// Create a new mode context with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a mode context for non-interactive (subagent) mode.
    pub fn for_subagent() -> Self {
        Self {
            mode: SessionMode::DontAsk,
            has_exited_plan_mode: false,
            plan_slug: None,
        }
    }

    /// Set the session mode.
    pub fn set_mode(&mut self, mode: SessionMode) {
        // Track plan mode exit
        if self.mode.is_planning() && !mode.is_planning() {
            self.has_exited_plan_mode = true;
        }
        self.mode = mode;
    }

    /// Enter plan mode with the given plan file path.
    pub fn enter_plan_mode(&mut self, plan_file_path: String) {
        self.set_mode(SessionMode::Plan { plan_file_path });
    }

    /// Exit plan mode and return to default mode.
    pub fn exit_plan_mode(&mut self) {
        self.set_mode(SessionMode::Default);
    }

    /// Check if in planning mode.
    pub fn is_planning(&self) -> bool {
        self.mode.is_planning()
    }

    /// Alias for is_planning().
    pub fn is_in_plan_mode(&self) -> bool {
        self.is_planning()
    }

    /// Get the plan file path if in planning mode.
    pub fn plan_file_path(&self) -> Option<&str> {
        self.mode.plan_file_path()
    }

    /// Check if in non-interactive mode.
    pub fn is_non_interactive(&self) -> bool {
        self.mode.is_non_interactive()
    }

    /// Get the plan slug if set.
    pub fn plan_slug(&self) -> Option<&str> {
        self.plan_slug.as_deref()
    }

    /// Set the plan slug.
    pub fn set_plan_slug(&mut self, slug: String) {
        self.plan_slug = Some(slug);
    }

    /// Get or generate a plan slug.
    /// If no slug exists, generates one and stores it.
    pub fn get_or_create_plan_slug<F>(&mut self, generate: F) -> &str
    where
        F: FnOnce() -> String,
    {
        if self.plan_slug.is_none() {
            self.plan_slug = Some(generate());
        }
        self.plan_slug.as_deref().expect("slug was just set")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_context() {
        let ctx = SessionModeContext::default();
        assert!(matches!(ctx.mode, SessionMode::Default));
        assert!(!ctx.has_exited_plan_mode);
    }

    #[test]
    fn test_subagent_context() {
        let ctx = SessionModeContext::for_subagent();
        assert!(matches!(ctx.mode, SessionMode::DontAsk));
        assert!(!ctx.has_exited_plan_mode);
    }

    #[test]
    fn test_enter_exit_plan_mode() {
        let mut ctx = SessionModeContext::new();

        ctx.enter_plan_mode("/tmp/plan.md".to_string());
        assert!(ctx.is_planning());
        assert_eq!(ctx.plan_file_path(), Some("/tmp/plan.md"));
        assert!(!ctx.has_exited_plan_mode);

        ctx.exit_plan_mode();
        assert!(!ctx.is_planning());
        assert!(ctx.has_exited_plan_mode);
    }
}
