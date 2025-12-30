//! Permission Mode types.
//!
//! Unified permission mode controlling approval behavior following Claude Code's architecture.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Unified permission mode controlling approval behavior.
///
/// This enum replaces the separate `PlanModeState` and `AskForApproval` systems
/// with a comprehensive permission mode following Claude Code's architecture.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase", tag = "mode")]
#[ts(rename_all = "camelCase", tag = "mode")]
pub enum PermissionMode {
    /// Standard approval flow - ask when needed.
    #[default]
    Default,

    /// Auto-approve file edits without confirmation.
    AcceptEdits,

    /// Plan mode - exploration phase with restricted writes.
    /// Only the plan file can be written.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Plan {
        /// Path to the plan file where the model can write.
        /// Always resolved when entering plan mode.
        plan_file_path: String,
    },

    /// Bypass all permissions - auto-approve everything.
    /// Only available when explicitly enabled via CLI flag.
    BypassPermissions,

    /// Subagent/non-interactive mode - auto-DENY if approval needed.
    /// Used by Task tool agents, hook agents, and built-in agents.
    /// These contexts cannot show permission prompts to users.
    DontAsk,
}

impl PermissionMode {
    /// Human-readable display name for the mode.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::AcceptEdits => "Accept Edits",
            Self::Plan { .. } => "Plan Mode",
            Self::BypassPermissions => "Bypass Permissions",
            Self::DontAsk => "Don't Ask",
        }
    }

    /// Icon representation for UI display.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Default => "",
            Self::AcceptEdits => "⏵⏵",
            Self::Plan { .. } => "⏸",
            Self::BypassPermissions => "⏵⏵",
            Self::DontAsk => "⏵⏵",
        }
    }

    /// Color theme identifier for UI styling.
    pub fn color_theme(&self) -> &'static str {
        match self {
            Self::Default => "text",
            Self::AcceptEdits => "autoAccept",
            Self::Plan { .. } => "planMode",
            Self::BypassPermissions => "error",
            Self::DontAsk => "error",
        }
    }

    /// Get next mode in cycle (shift+tab) - for interactive TUI only.
    ///
    /// The `resolve_plan_path` closure is called lazily only when transitioning
    /// to Plan mode. This allows the caller to provide context (like session_id)
    /// for path resolution without the enum needing to know about it.
    ///
    /// Note: DontAsk is NOT in the cycle - it's only set programmatically for subagents.
    pub fn next_mode<F>(&self, is_bypass_available: bool, resolve_plan_path: F) -> Self
    where
        F: FnOnce() -> String,
    {
        match self {
            Self::Default => Self::AcceptEdits,
            Self::AcceptEdits => Self::Plan {
                plan_file_path: resolve_plan_path(),
            },
            Self::Plan { .. } => {
                if is_bypass_available {
                    Self::BypassPermissions
                } else {
                    Self::Default
                }
            }
            Self::BypassPermissions => Self::Default,
            // DontAsk cycles back to Default (shouldn't happen in TUI)
            Self::DontAsk => Self::Default,
        }
    }

    /// Check if we are currently in planning mode.
    pub fn is_planning(&self) -> bool {
        matches!(self, Self::Plan { .. })
    }

    /// Get the plan file path if in planning mode.
    /// Returns None if not in plan mode.
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

    /// Check if this mode auto-approves all operations.
    pub fn is_bypass(&self) -> bool {
        matches!(self, Self::BypassPermissions)
    }

    /// Check if this mode auto-approves file edits.
    pub fn auto_approves_edits(&self) -> bool {
        matches!(self, Self::AcceptEdits | Self::BypassPermissions)
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

/// Event emitted when permission mode changes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct PermissionModeChangedEvent {
    /// The new permission mode.
    pub mode: PermissionMode,
    /// The previous permission mode.
    pub previous_mode: PermissionMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_mode() {
        let mode = PermissionMode::default();
        assert!(matches!(mode, PermissionMode::Default));
        assert!(!mode.is_planning());
        assert!(!mode.is_non_interactive());
        assert!(!mode.is_bypass());
    }

    #[test]
    fn test_plan_mode() {
        let mode = PermissionMode::Plan {
            plan_file_path: "/tmp/plan.md".to_string(),
        };
        assert!(mode.is_planning());
        assert_eq!(mode.plan_file_path(), Some("/tmp/plan.md"));
        assert_eq!(mode.display_name(), "Plan Mode");
        assert_eq!(mode.icon(), "⏸");
    }

    #[test]
    fn test_mode_cycling() {
        let mode = PermissionMode::Default;
        let resolve_path = || "/tmp/test-plan.md".to_string();

        // Without bypass available
        let next = mode.next_mode(false, resolve_path);
        assert!(matches!(next, PermissionMode::AcceptEdits));

        let next = next.next_mode(false, resolve_path);
        assert!(matches!(next, PermissionMode::Plan { .. }));
        assert_eq!(next.plan_file_path(), Some("/tmp/test-plan.md"));

        let next = next.next_mode(false, resolve_path);
        assert!(matches!(next, PermissionMode::Default));

        // With bypass available
        let plan_mode = PermissionMode::Plan {
            plan_file_path: "/tmp/plan.md".to_string(),
        };
        let next = plan_mode.next_mode(true, resolve_path);
        assert!(matches!(next, PermissionMode::BypassPermissions));

        let next = next.next_mode(true, resolve_path);
        assert!(matches!(next, PermissionMode::Default));
    }

    #[test]
    fn test_dont_ask_mode() {
        let mode = PermissionMode::DontAsk;
        assert!(mode.is_non_interactive());
        assert!(!mode.is_bypass());
        assert_eq!(mode.display_name(), "Don't Ask");
        // DontAsk cycles back to Default
        assert!(matches!(
            mode.next_mode(true, || panic!("should not be called")),
            PermissionMode::Default
        ));
    }

    #[test]
    fn test_serialization() {
        let mode = PermissionMode::Plan {
            plan_file_path: "/tmp/test.md".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        assert!(json.contains("plan"));
        assert!(json.contains("planFilePath"));

        let deserialized: PermissionMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, deserialized);
    }
}
