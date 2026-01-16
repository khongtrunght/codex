//! Session Mode Context.
//!
//! Holds the current session workflow mode and related tracking state.
//! This tracks workflow state only - approval behavior is handled separately
//! by AskForApproval and SandboxPolicy in the approval system.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

use crate::session_mode::SessionMode;

/// Session-level mode context.
///
/// Tracks the current workflow mode and related state.
/// Approval behavior is handled separately by AskForApproval/SandboxPolicy.
///
/// Note: Plan file path is NOT stored here. It's derived from `plan_slug`
/// using `resolve_plan_file_path_with_slug()` in core when needed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionModeContext {
    /// Current session mode (workflow state).
    pub mode: SessionMode,

    /// Track if plan mode has been exited (for reentry detection).
    #[serde(default)]
    pub has_exited_plan_mode: bool,

    /// Flag to inject plan_mode_exit attachment on next turn.
    /// Set to true when mode changes from plan to non-plan via UI (shift+tab).
    /// The collector will check this flag and inject the exit notification once.
    #[serde(default)]
    pub needs_plan_mode_exit_attachment: bool,

    /// Plan slug for memorable file naming (e.g., "atomic-marinating-pumpkin").
    /// Stored per-session so it persists across plan mode entries.
    /// The plan file path is derived from this slug when needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_slug: Option<String>,
}

impl Default for SessionModeContext {
    fn default() -> Self {
        Self {
            mode: SessionMode::Default,
            has_exited_plan_mode: false,
            needs_plan_mode_exit_attachment: false,
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
            needs_plan_mode_exit_attachment: false,
            plan_slug: None,
        }
    }

    /// Set the session mode.
    ///
    /// Note: When entering Plan mode, ensure plan_slug is set first
    /// (either via set_plan_slug or get_or_create_plan_slug).
    pub fn set_mode(&mut self, mode: SessionMode) {
        // Track plan mode exit and set flag for exit attachment
        if self.mode.is_planning() && !mode.is_planning() {
            self.has_exited_plan_mode = true;
            self.needs_plan_mode_exit_attachment = true;
        }
        self.mode = mode;
    }

    /// Clear the needs_plan_mode_exit_attachment flag.
    /// Called after the exit attachment has been injected.
    pub fn clear_plan_mode_exit_attachment_flag(&mut self) {
        self.needs_plan_mode_exit_attachment = false;
    }

    /// Check if plan mode exit attachment is needed.
    pub fn needs_plan_mode_exit_attachment(&self) -> bool {
        self.needs_plan_mode_exit_attachment
    }

    /// Enter plan mode.
    ///
    /// Note: Ensure plan_slug is set before calling this. The slug
    /// should be generated once per session and reused across entries.
    pub fn enter_plan_mode(&mut self) {
        self.mode = SessionMode::Plan;
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
        self.plan_slug.get_or_insert_with(generate)
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
        assert!(!ctx.needs_plan_mode_exit_attachment);
    }

    #[test]
    fn test_subagent_context() {
        let ctx = SessionModeContext::for_subagent();
        assert!(matches!(ctx.mode, SessionMode::DontAsk));
        assert!(!ctx.has_exited_plan_mode);
        assert!(!ctx.needs_plan_mode_exit_attachment);
    }

    #[test]
    fn test_enter_exit_plan_mode() {
        let mut ctx = SessionModeContext::new();

        // Set slug first, then enter plan mode
        ctx.set_plan_slug("test-slug".to_string());
        ctx.enter_plan_mode();
        assert!(ctx.is_planning());
        assert_eq!(ctx.plan_slug(), Some("test-slug"));
        assert!(!ctx.has_exited_plan_mode);
        assert!(!ctx.needs_plan_mode_exit_attachment);

        ctx.exit_plan_mode();
        assert!(!ctx.is_planning());
        assert!(ctx.has_exited_plan_mode);
        assert!(ctx.needs_plan_mode_exit_attachment()); // Flag set on exit
        // slug persists after exiting plan mode
        assert_eq!(ctx.plan_slug(), Some("test-slug"));

        // Clear the flag
        ctx.clear_plan_mode_exit_attachment_flag();
        assert!(!ctx.needs_plan_mode_exit_attachment());
    }

    #[test]
    fn test_get_or_create_plan_slug() {
        let mut ctx = SessionModeContext::new();

        // First call creates the slug
        let slug1 = ctx.get_or_create_plan_slug(|| "generated-slug".to_string());
        assert_eq!(slug1, "generated-slug");

        // Second call returns the same slug (doesn't regenerate)
        let slug2 = ctx.get_or_create_plan_slug(|| "different-slug".to_string());
        assert_eq!(slug2, "generated-slug");
    }
}
