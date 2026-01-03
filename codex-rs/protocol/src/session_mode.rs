//! Session Mode types.
//!
//! Defines the workflow modes that control session behavior.
//! These are purely workflow states (Default, Plan, DontAsk).
//!
//! Approval behavior (AcceptEdits, Bypass) is handled separately by
//! AskForApproval and SandboxPolicy in the approval system.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Session workflow mode.
///
/// Defines the high-level workflow state of the session.
/// This is separate from approval behavior (AskForApproval/SandboxPolicy).
///
/// - Default: Normal workflow, approval determined by policy
/// - Plan: Planning phase with restricted writes (only plan file)
/// - DontAsk: Non-interactive mode for subagents (auto-deny if approval needed)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase", tag = "mode")]
#[ts(rename_all = "camelCase", tag = "mode")]
pub enum SessionMode {
    /// Standard mode - normal workflow.
    /// Approval behavior determined by AskForApproval/SandboxPolicy.
    #[default]
    Default,

    /// Plan mode - exploration phase with restricted writes.
    /// Only the plan file can be written during this phase.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Plan {
        /// Path to the plan file where the model can write.
        plan_file_path: String,
    },

    /// Non-interactive mode - auto-DENY if approval would be needed.
    /// Used by subagents, hook agents, and built-in agents.
    /// These contexts cannot show permission prompts to users.
    DontAsk,
}

impl SessionMode {
    /// Human-readable display name for the mode.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Plan { .. } => "Plan Mode",
            Self::DontAsk => "Don't Ask",
        }
    }

    /// Icon representation for UI display.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Plan { .. } => "\u{23F8}", // ⏸
            Self::DontAsk => "",
        }
    }

    /// Color theme identifier for UI styling.
    pub fn color_theme(&self) -> &'static str {
        match self {
            Self::Default => "text",
            Self::Plan { .. } => "planMode",
            Self::DontAsk => "error",
        }
    }

    /// Check if we are currently in planning mode.
    pub fn is_planning(&self) -> bool {
        matches!(self, Self::Plan { .. })
    }

    /// Get the plan file path if in planning mode.
    pub fn plan_file_path(&self) -> Option<&str> {
        match self {
            Self::Plan { plan_file_path } => Some(plan_file_path.as_str()),
            _ => None,
        }
    }

    /// Check if this mode is for non-interactive contexts (subagents).
    pub fn is_non_interactive(&self) -> bool {
        matches!(self, Self::DontAsk)
    }
}

/// Event emitted when entering plan mode.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct EnteredPlanModeEvent {
    /// Path to the plan file.
    pub plan_file_path: String,
}

/// Event emitted when exiting plan mode.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExitedPlanModeEvent {
    /// The plan content read from the plan file.
    pub plan: String,
    /// Path to the plan file.
    pub plan_file_path: String,
}

/// Event emitted when session mode changes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionModeChangedEvent {
    /// The new session mode.
    pub mode: SessionMode,
    /// The previous session mode.
    pub previous_mode: SessionMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_mode() {
        let mode = SessionMode::default();
        assert!(matches!(mode, SessionMode::Default));
        assert!(!mode.is_planning());
        assert!(!mode.is_non_interactive());
    }

    #[test]
    fn test_plan_mode() {
        let mode = SessionMode::Plan {
            plan_file_path: "/tmp/plan.md".to_string(),
        };
        assert!(mode.is_planning());
        assert_eq!(mode.plan_file_path(), Some("/tmp/plan.md"));
        assert_eq!(mode.display_name(), "Plan Mode");
    }

    #[test]
    fn test_dont_ask_mode() {
        let mode = SessionMode::DontAsk;
        assert!(mode.is_non_interactive());
        assert_eq!(mode.display_name(), "Don't Ask");
    }

    #[test]
    fn test_serialization() {
        let mode = SessionMode::Plan {
            plan_file_path: "/tmp/test.md".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        assert!(json.contains("plan"));
        assert!(json.contains("planFilePath"));

        let deserialized: SessionMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, deserialized);
    }
}
