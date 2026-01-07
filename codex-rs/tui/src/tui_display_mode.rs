//! TUI Display Mode
//!
//! Represents the combined display mode for the TUI, which combines:
//! - SessionMode (workflow state: Default, Plan, DontAsk)
//! - Approval settings (AcceptEdits, Bypass via AskForApproval/SandboxPolicy)
//!
//! This is a UI-only concept for the Shift+Tab cycle and visual display.

use ratatui::style::Color;

/// TUI display mode for visual display and mode cycling.
///
/// This combines SessionMode (workflow) with approval settings for display.
/// The cycle is: Default -> AcceptEdits -> Plan -> Bypass (if available) -> Default
#[derive(Debug, Clone, PartialEq)]
pub enum TuiDisplayMode {
    /// Default mode - normal approval flow.
    Default,

    /// Accept edits mode - auto-approve file writes.
    /// Maps to: SessionMode::Default + some approval behavior
    AcceptEdits,

    /// Plan mode - exploration phase with restricted writes.
    /// Maps to: SessionMode::Plan
    /// Note: Plan file path is managed by the backend (derived from plan_slug).
    Plan,

    /// Bypass mode - skip all permission checks.
    /// Maps to: SessionMode::Default + DangerFullAccess + Never
    Bypass,
}

impl Default for TuiDisplayMode {
    fn default() -> Self {
        Self::Default
    }
}

impl TuiDisplayMode {
    /// Human-readable display name for the mode.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::AcceptEdits => "Accept Edits",
            Self::Plan => "Plan Mode",
            Self::Bypass => "Bypass",
        }
    }

    /// Icon representation for UI display.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Default => "",
            Self::AcceptEdits => "\u{23F5}\u{23F5}", // ⏵⏵
            Self::Plan => "\u{23F8}",                 // ⏸
            Self::Bypass => "\u{23F5}\u{23F5}",      // ⏵⏵
        }
    }

    /// Color for UI display.
    pub fn color(&self) -> Color {
        match self {
            Self::Default => Color::Reset,
            Self::AcceptEdits => Color::Yellow,
            Self::Plan => Color::Cyan,
            Self::Bypass => Color::Red,
        }
    }

    /// Get next mode in cycle (shift+tab).
    pub fn next_mode(&self, is_bypass_available: bool) -> Self {
        match self {
            Self::Default => Self::AcceptEdits,
            Self::AcceptEdits => Self::Plan,
            Self::Plan => {
                if is_bypass_available {
                    Self::Bypass
                } else {
                    Self::Default
                }
            }
            Self::Bypass => Self::Default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_cycling() {
        let mode = TuiDisplayMode::Default;

        // Without bypass
        let next = mode.next_mode(false);
        assert!(matches!(next, TuiDisplayMode::AcceptEdits));

        let next = next.next_mode(false);
        assert!(matches!(next, TuiDisplayMode::Plan));

        let next = next.next_mode(false);
        assert!(matches!(next, TuiDisplayMode::Default));

        // With bypass available
        let plan_mode = TuiDisplayMode::Plan;
        let next = plan_mode.next_mode(true);
        assert!(matches!(next, TuiDisplayMode::Bypass));

        let next = next.next_mode(true);
        assert!(matches!(next, TuiDisplayMode::Default));
    }
}
