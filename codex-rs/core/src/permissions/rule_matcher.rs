//! Permission rule matching logic.
//!
//! This module implements the core permission checking algorithm that evaluates
//! tool operations against the current permission context.

use codex_protocol::permission_context::PermissionContext;
use codex_protocol::permission_mode::PermissionMode;
use codex_protocol::permission_rules::RuleBehavior;
use codex_protocol::permission_updates::PermissionDecision;

/// Result of a permission check with additional context.
#[derive(Debug, Clone)]
pub struct PermissionCheckResult {
    /// The permission decision.
    pub decision: PermissionDecision,
    /// The reason for the decision.
    pub reason: PermissionCheckReason,
}

/// Reason why a permission decision was made.
#[derive(Debug, Clone)]
pub enum PermissionCheckReason {
    /// Decision based on the permission mode.
    Mode(String),
    /// Decision based on a matching rule.
    Rule { rule: String, behavior: RuleBehavior },
    /// Tool is inherently trusted.
    Trusted,
    /// Default behavior (no specific rule matched).
    Default,
    /// Plan mode restriction.
    PlanModeRestriction,
}

impl PermissionCheckResult {
    /// Create an allow result.
    pub fn allow(reason: PermissionCheckReason) -> Self {
        Self {
            decision: PermissionDecision::allow(),
            reason,
        }
    }

    /// Create a deny result.
    pub fn deny(message: impl Into<String>, reason: PermissionCheckReason) -> Self {
        Self {
            decision: PermissionDecision::deny(message),
            reason,
        }
    }

    /// Create an ask result.
    pub fn ask(reason: PermissionCheckReason) -> Self {
        Self {
            decision: PermissionDecision::ask(),
            reason,
        }
    }

    /// Check if this is an allow decision.
    pub fn is_allow(&self) -> bool {
        self.decision.is_allow()
    }

    /// Check if this is a deny decision.
    pub fn is_deny(&self) -> bool {
        self.decision.is_deny()
    }

    /// Check if this needs user approval.
    pub fn needs_approval(&self) -> bool {
        self.decision.needs_approval()
    }
}

/// Check permission for a tool operation.
///
/// This is the main entry point for permission checking. It evaluates the
/// tool operation against the current permission context and returns a
/// decision indicating whether the operation should be allowed, denied,
/// or requires user approval.
///
/// # Arguments
///
/// * `ctx` - The current permission context.
/// * `tool_name` - The name of the tool being invoked.
/// * `input` - The input/argument to the tool (e.g., file path, command).
///
/// # Returns
///
/// A `PermissionCheckResult` indicating the decision and reason.
///
/// # Decision Order
///
/// 1. Check mode-based decisions (BypassPermissions, DontAsk, AcceptEdits, Plan)
/// 2. Check explicit deny rules (deny always wins)
/// 3. Check explicit allow rules
/// 4. Default to asking for approval
pub fn check_permission(
    ctx: &PermissionContext,
    tool_name: &str,
    input: &str,
) -> PermissionCheckResult {
    // 1. Check mode-based decisions
    match &ctx.mode {
        // BypassPermissions: auto-approve everything
        PermissionMode::BypassPermissions => {
            return PermissionCheckResult::allow(PermissionCheckReason::Mode(
                "bypassPermissions".to_string(),
            ));
        }

        // DontAsk (subagent mode): we'll check rules first, then auto-deny if would need to ask
        PermissionMode::DontAsk => {
            // Continue to check rules first
        }

        // AcceptEdits: auto-approve file edits
        PermissionMode::AcceptEdits => {
            if is_edit_tool(tool_name) {
                return PermissionCheckResult::allow(PermissionCheckReason::Mode(
                    "acceptEdits".to_string(),
                ));
            }
        }

        // Plan mode: only allow writes to plan file
        PermissionMode::Plan { plan_file_path } => {
            if is_write_tool(tool_name) {
                if input == plan_file_path || input.ends_with(plan_file_path) {
                    return PermissionCheckResult::allow(PermissionCheckReason::Mode(
                        "plan".to_string(),
                    ));
                }
                // Other writes denied in plan mode
                return PermissionCheckResult::deny(
                    format!(
                        "Only the plan file ({}) can be written in Plan mode. Attempted to write: {}",
                        plan_file_path, input
                    ),
                    PermissionCheckReason::PlanModeRestriction,
                );
            }
        }

        PermissionMode::Default => {
            // Continue to check rules
        }
    }

    // 2. Check explicit deny rules (deny > ask > allow)
    if let Some(rule) = ctx.match_rule(tool_name, input, RuleBehavior::Deny) {
        return PermissionCheckResult::deny(
            format!("Denied by permission rule: {rule}"),
            PermissionCheckReason::Rule {
                rule: rule.to_string(),
                behavior: RuleBehavior::Deny,
            },
        );
    }

    // 3. Check explicit ask rules (force prompt even if would otherwise be allowed)
    if ctx.match_rule(tool_name, input, RuleBehavior::Ask).is_some() {
        // For DontAsk mode, this becomes a deny
        if matches!(ctx.mode, PermissionMode::DontAsk) {
            return PermissionCheckResult::deny(
                format!("Permission to use {tool_name} auto-denied in non-interactive mode"),
                PermissionCheckReason::Mode("dontAsk".to_string()),
            );
        }
        // Otherwise, ask for approval
        return PermissionCheckResult::ask(PermissionCheckReason::Default);
    }

    // 4. Check explicit allow rules
    if let Some(rule) = ctx.match_rule(tool_name, input, RuleBehavior::Allow) {
        return PermissionCheckResult::allow(PermissionCheckReason::Rule {
            rule: rule.to_string(),
            behavior: RuleBehavior::Allow,
        });
    }

    // 5. Would need to ask - but check if in DontAsk mode
    if matches!(ctx.mode, PermissionMode::DontAsk) {
        return PermissionCheckResult::deny(
            format!("Permission to use {tool_name} auto-denied in non-interactive mode"),
            PermissionCheckReason::Mode("dontAsk".to_string()),
        );
    }

    // 6. Default: need to ask for approval
    PermissionCheckResult::ask(PermissionCheckReason::Default)
}

/// Check if a tool is an edit/write tool.
fn is_edit_tool(tool_name: &str) -> bool {
    matches!(tool_name, "Edit" | "edit_file" | "str_replace_editor")
}

/// Check if a tool is a write tool (creates/modifies files).
fn is_write_tool(tool_name: &str) -> bool {
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
#[allow(dead_code)]
fn is_safe_tool(tool_name: &str) -> bool {
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
    fn test_bypass_mode_allows_everything() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::BypassPermissions;

        let result = check_permission(&ctx, "Bash", "rm -rf /");
        assert!(result.is_allow());
    }

    #[test]
    fn test_dont_ask_mode_denies_when_would_ask() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::DontAsk;

        let result = check_permission(&ctx, "Bash", "ls -la");
        assert!(result.is_deny());
    }

    #[test]
    fn test_dont_ask_mode_allows_with_rule() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::DontAsk;
        ctx.add_rule("Bash(*)", RuleBehavior::Allow, RuleDestination::Session);

        let result = check_permission(&ctx, "Bash", "ls -la");
        assert!(result.is_allow());
    }

    #[test]
    fn test_accept_edits_mode() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::AcceptEdits;

        let result = check_permission(&ctx, "Edit", "src/main.rs");
        assert!(result.is_allow());

        // Non-edit tools still require approval
        let result = check_permission(&ctx, "Bash", "ls");
        assert!(result.needs_approval());
    }

    #[test]
    fn test_plan_mode_allows_plan_file() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::Plan {
            plan_file_path: "/tmp/plan.md".to_string(),
        };

        let result = check_permission(&ctx, "Edit", "/tmp/plan.md");
        assert!(result.is_allow());
    }

    #[test]
    fn test_plan_mode_denies_other_writes() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::Plan {
            plan_file_path: "/tmp/plan.md".to_string(),
        };

        let result = check_permission(&ctx, "Edit", "src/main.rs");
        assert!(result.is_deny());
    }

    #[test]
    fn test_deny_rule_overrides_allow() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule("Edit(*)", RuleBehavior::Allow, RuleDestination::Session);
        ctx.add_rule("Edit(.env)", RuleBehavior::Deny, RuleDestination::Session);

        let result = check_permission(&ctx, "Edit", "src/main.rs");
        assert!(result.is_allow());

        let result = check_permission(&ctx, "Edit", ".env");
        assert!(result.is_deny());
    }

    #[test]
    fn test_allow_rule_prevents_asking() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule("Read(*)", RuleBehavior::Allow, RuleDestination::Session);

        let result = check_permission(&ctx, "Read", "any/file.txt");
        assert!(result.is_allow());
    }

    #[test]
    fn test_default_asks_for_approval() {
        let ctx = PermissionContext::default();

        let result = check_permission(&ctx, "Bash", "ls -la");
        assert!(result.needs_approval());
    }

    #[test]
    fn test_glob_pattern_matching() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule("Edit(src/**)", RuleBehavior::Allow, RuleDestination::Session);

        let result = check_permission(&ctx, "Edit", "src/main.rs");
        assert!(result.is_allow());

        let result = check_permission(&ctx, "Edit", "src/lib/mod.rs");
        assert!(result.is_allow());

        let result = check_permission(&ctx, "Edit", "tests/test.rs");
        assert!(result.needs_approval());
    }
}
