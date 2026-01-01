//! Tool-specific permission checking types.
//!
//! This module defines types for tool-specific permission checks that follow
//! the Claude Code pattern with `Passthrough` support for chained checks.
//!
//! Unlike the protocol-level `PermissionDecision`, these types are internal
//! to the permission system and support the `Passthrough` behavior for
//! indicating "I don't have an opinion, continue to next check".

use codex_protocol::permission_context::PermissionContext;
use codex_protocol::permission_mode::PermissionMode;
use codex_protocol::permission_rules::RuleBehavior;

/// Permission behavior for tool-specific checks.
///
/// This enum follows the Claude Code pattern where each check in a chain
/// can return one of four behaviors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionBehavior {
    /// Allow the operation without prompting.
    Allow,
    /// Deny the operation with a reason.
    Deny,
    /// Need to prompt the user for approval.
    Ask,
    /// This check doesn't apply, continue to next check in chain.
    Passthrough,
}

/// Reason for a tool permission decision.
#[derive(Debug, Clone)]
pub enum ToolPermissionReason {
    /// Decision based on permission mode.
    Mode { mode: String },
    /// Decision based on a matching rule.
    Rule { rule: String, behavior: RuleBehavior },
    /// Tool is inherently safe (read-only, no side effects).
    Safe,
    /// Path is within allowed working directories.
    WorkingDirectory,
    /// Command is read-only.
    ReadOnlyCommand,
    /// Tool-specific validation (e.g., UNC path check).
    Validation { reason: String },
    /// No specific reason (passthrough or default).
    None,
}

/// Result of a tool-specific permission check.
///
/// This is the return type for `ToolHandler::check_permissions()`.
#[derive(Debug, Clone)]
pub struct ToolPermissionResult {
    /// The permission behavior.
    pub behavior: PermissionBehavior,
    /// Human-readable message (used for deny/ask).
    pub message: Option<String>,
    /// Reason for the decision.
    pub reason: ToolPermissionReason,
}

impl ToolPermissionResult {
    /// Create an allow result.
    pub fn allow(reason: ToolPermissionReason) -> Self {
        Self {
            behavior: PermissionBehavior::Allow,
            message: None,
            reason,
        }
    }

    /// Create an allow result with a custom message.
    pub fn allow_with_message(message: impl Into<String>, reason: ToolPermissionReason) -> Self {
        Self {
            behavior: PermissionBehavior::Allow,
            message: Some(message.into()),
            reason,
        }
    }

    /// Create a deny result.
    pub fn deny(message: impl Into<String>, reason: ToolPermissionReason) -> Self {
        Self {
            behavior: PermissionBehavior::Deny,
            message: Some(message.into()),
            reason,
        }
    }

    /// Create an ask result.
    pub fn ask(message: impl Into<String>) -> Self {
        Self {
            behavior: PermissionBehavior::Ask,
            message: Some(message.into()),
            reason: ToolPermissionReason::None,
        }
    }

    /// Create a passthrough result (this check doesn't apply).
    pub fn passthrough() -> Self {
        Self {
            behavior: PermissionBehavior::Passthrough,
            message: None,
            reason: ToolPermissionReason::None,
        }
    }

    /// Create a passthrough result with a message for debugging.
    pub fn passthrough_with_message(message: impl Into<String>) -> Self {
        Self {
            behavior: PermissionBehavior::Passthrough,
            message: Some(message.into()),
            reason: ToolPermissionReason::None,
        }
    }

    /// Check if this is an allow decision.
    pub fn is_allow(&self) -> bool {
        matches!(self.behavior, PermissionBehavior::Allow)
    }

    /// Check if this is a deny decision.
    pub fn is_deny(&self) -> bool {
        matches!(self.behavior, PermissionBehavior::Deny)
    }

    /// Check if this is an ask decision.
    pub fn is_ask(&self) -> bool {
        matches!(self.behavior, PermissionBehavior::Ask)
    }

    /// Check if this is a passthrough decision.
    pub fn is_passthrough(&self) -> bool {
        matches!(self.behavior, PermissionBehavior::Passthrough)
    }

    /// Check if this decision stops the chain (not passthrough).
    pub fn stops_chain(&self) -> bool {
        !self.is_passthrough()
    }
}

/// Evaluate permission based on mode only.
///
/// This is an early check that handles special modes like BypassPermissions.
pub fn evaluate_mode_permission(
    ctx: &PermissionContext,
    tool_name: &str,
) -> ToolPermissionResult {
    match &ctx.mode {
        PermissionMode::BypassPermissions => {
            ToolPermissionResult::allow(ToolPermissionReason::Mode {
                mode: "bypassPermissions".to_string(),
            })
        }

        PermissionMode::AcceptEdits => {
            if is_edit_tool(tool_name) {
                ToolPermissionResult::allow(ToolPermissionReason::Mode {
                    mode: "acceptEdits".to_string(),
                })
            } else {
                ToolPermissionResult::passthrough_with_message("Not an edit tool")
            }
        }

        PermissionMode::Plan { plan_file_path: _ } => {
            // Plan mode is handled by tool-specific checks for write tools
            ToolPermissionResult::passthrough_with_message("Plan mode handled by tool")
        }

        PermissionMode::DontAsk | PermissionMode::Default => {
            ToolPermissionResult::passthrough_with_message("Continue to rule checks")
        }
    }
}

/// Evaluate permission based on rules only.
///
/// Checks deny, ask, and allow rules in order.
pub fn evaluate_rules_permission(
    ctx: &PermissionContext,
    tool_name: &str,
    input: &str,
) -> ToolPermissionResult {
    // Check deny rules first (deny > ask > allow)
    if let Some(rule) = ctx.match_rule(tool_name, input, RuleBehavior::Deny) {
        return ToolPermissionResult::deny(
            format!("Denied by permission rule: {rule}"),
            ToolPermissionReason::Rule {
                rule: rule.to_string(),
                behavior: RuleBehavior::Deny,
            },
        );
    }

    // Check ask rules
    if let Some(rule) = ctx.match_rule(tool_name, input, RuleBehavior::Ask) {
        return ToolPermissionResult::ask(format!(
            "Approval required by rule: {rule}"
        ));
    }

    // Check allow rules
    if let Some(rule) = ctx.match_rule(tool_name, input, RuleBehavior::Allow) {
        return ToolPermissionResult::allow(ToolPermissionReason::Rule {
            rule: rule.to_string(),
            behavior: RuleBehavior::Allow,
        });
    }

    // No rule matched
    ToolPermissionResult::passthrough_with_message("No matching rule")
}

/// Handle DontAsk mode for Ask decisions.
///
/// If in DontAsk mode and a check returns Ask, convert to Deny.
pub fn handle_dont_ask_mode(
    ctx: &PermissionContext,
    tool_name: &str,
    result: ToolPermissionResult,
) -> ToolPermissionResult {
    if matches!(result.behavior, PermissionBehavior::Ask)
        && matches!(ctx.mode, PermissionMode::DontAsk)
    {
        ToolPermissionResult::deny(
            format!("Permission to use {} has been auto-denied in DontAsk mode.", tool_name),
            ToolPermissionReason::Mode {
                mode: "dontAsk".to_string(),
            },
        )
    } else {
        result
    }
}

/// Check if a tool is an edit tool.
fn is_edit_tool(tool_name: &str) -> bool {
    matches!(tool_name, "Edit" | "edit_file" | "str_replace_editor")
}

/// Check if a tool is a write tool (creates/modifies files).
pub fn is_write_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "Edit"
            | "edit_file"
            | "str_replace_editor"
            | "Write"
            | "write_file"
            | "create_file"
            | "NotebookEdit"
    )
}

/// Check if a tool is inherently safe (read-only, no side effects).
pub fn is_safe_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "Read"
            | "read_file"
            | "Glob"
            | "Grep"
            | "LS"
            | "list_dir"
            | "View"
            | "view_image"
            | "WebSearch"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::permission_rules::RuleDestination;

    #[test]
    fn test_passthrough_result() {
        let result = ToolPermissionResult::passthrough();
        assert!(result.is_passthrough());
        assert!(!result.stops_chain());
    }

    #[test]
    fn test_allow_stops_chain() {
        let result = ToolPermissionResult::allow(ToolPermissionReason::Safe);
        assert!(result.is_allow());
        assert!(result.stops_chain());
    }

    #[test]
    fn test_deny_stops_chain() {
        let result = ToolPermissionResult::deny("test", ToolPermissionReason::None);
        assert!(result.is_deny());
        assert!(result.stops_chain());
    }

    #[test]
    fn test_bypass_mode_allows() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::BypassPermissions;

        let result = evaluate_mode_permission(&ctx, "Bash");
        assert!(result.is_allow());
    }

    #[test]
    fn test_accept_edits_mode() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::AcceptEdits;

        let result = evaluate_mode_permission(&ctx, "Edit");
        assert!(result.is_allow());

        let result = evaluate_mode_permission(&ctx, "Bash");
        assert!(result.is_passthrough());
    }

    #[test]
    fn test_rules_evaluation() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule("Read(*)", RuleBehavior::Allow, RuleDestination::Session);
        ctx.add_rule("Edit(.env)", RuleBehavior::Deny, RuleDestination::Session);

        let result = evaluate_rules_permission(&ctx, "Read", "any.txt");
        assert!(result.is_allow());

        let result = evaluate_rules_permission(&ctx, "Edit", ".env");
        assert!(result.is_deny());

        let result = evaluate_rules_permission(&ctx, "Bash", "ls");
        assert!(result.is_passthrough());
    }

    #[test]
    fn test_dont_ask_mode_converts_ask_to_deny() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::DontAsk;

        let ask_result = ToolPermissionResult::ask("Need approval");
        let result = handle_dont_ask_mode(&ctx, "Bash", ask_result);
        assert!(result.is_deny());
        assert!(result.message.unwrap().contains("auto-denied"));
    }

    #[test]
    fn test_dont_ask_mode_preserves_allow() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::DontAsk;

        let allow_result = ToolPermissionResult::allow(ToolPermissionReason::Safe);
        let result = handle_dont_ask_mode(&ctx, "Read", allow_result);
        assert!(result.is_allow());
    }
}
