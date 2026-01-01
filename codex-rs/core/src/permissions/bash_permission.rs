//! Bash/Shell-specific permission evaluation functions.
//!
//! This module provides helper functions for evaluating shell command permissions
//! following the Claude Code pattern where each tool extracts its own input.

use codex_protocol::permission_context::PermissionContext;

use super::tool_permission::evaluate_rules_permission;
use super::tool_permission::ToolPermissionReason;
use super::tool_permission::ToolPermissionResult;
use crate::is_safe_command::is_known_safe_command;

/// Evaluate permission for bash/shell commands.
///
/// Permission check order (first non-passthrough wins):
/// 1. Deny rules matching Bash(command)
/// 2. Allow rules matching Bash(command)
/// 3. Read-only command auto-allow
/// 4. Passthrough (defer to existing approval flow)
///
/// # Arguments
///
/// * `command` - The shell command string (e.g., "ls -la", "npm test")
/// * `command_args` - The parsed command as a Vec<String> for safe command check
/// * `ctx` - Permission context with rules and mode
pub fn evaluate_bash_permission(
    command: &str,
    command_args: &[String],
    ctx: &PermissionContext,
) -> ToolPermissionResult {
    // Check rules first (deny > ask > allow)
    let rules_result = evaluate_rules_permission(ctx, "Bash", command);
    if rules_result.stops_chain() {
        return rules_result;
    }

    // Auto-allow read-only/safe commands
    if is_known_safe_command(command_args) {
        return ToolPermissionResult::allow(ToolPermissionReason::ReadOnlyCommand);
    }

    // Passthrough to existing approval flow
    ToolPermissionResult::passthrough()
}

/// Evaluate permission for shell/shell_command tool.
///
/// Same as bash but with "Shell" as the tool name for rule matching.
pub fn evaluate_shell_permission(
    command: &str,
    command_args: &[String],
    ctx: &PermissionContext,
) -> ToolPermissionResult {
    // Check rules first (deny > ask > allow)
    let rules_result = evaluate_rules_permission(ctx, "Shell", command);
    if rules_result.stops_chain() {
        return rules_result;
    }

    // Auto-allow read-only/safe commands
    if is_known_safe_command(command_args) {
        return ToolPermissionResult::allow(ToolPermissionReason::ReadOnlyCommand);
    }

    // Passthrough to existing approval flow
    ToolPermissionResult::passthrough()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::permission_rules::RuleBehavior;
    use codex_protocol::permission_rules::RuleDestination;

    fn vec_str(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_bash_permission_deny_rule() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule("Bash(rm *)", RuleBehavior::Deny, RuleDestination::Session);

        let result = evaluate_bash_permission("rm *", &vec_str(&["rm", "*"]), &ctx);
        assert!(result.is_deny());
    }

    #[test]
    fn test_bash_permission_allow_rule() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule(
            "Bash(npm test*)",
            RuleBehavior::Allow,
            RuleDestination::Session,
        );

        // "npm test" should match pattern "npm test*"
        let result = evaluate_bash_permission("npm test", &vec_str(&["npm", "test"]), &ctx);
        assert!(result.is_allow());
    }

    #[test]
    fn test_bash_permission_safe_command() {
        let ctx = PermissionContext::default();

        // ls is a known safe command
        let result = evaluate_bash_permission("ls", &vec_str(&["ls"]), &ctx);
        assert!(result.is_allow());
        assert!(matches!(
            result.reason,
            ToolPermissionReason::ReadOnlyCommand
        ));

        // git status is a known safe command
        let result = evaluate_bash_permission("git status", &vec_str(&["git", "status"]), &ctx);
        assert!(result.is_allow());
    }

    #[test]
    fn test_bash_permission_unsafe_command() {
        let ctx = PermissionContext::default();

        // rm is not a safe command
        let result = evaluate_bash_permission("rm -rf /", &vec_str(&["rm", "-rf", "/"]), &ctx);
        assert!(result.is_passthrough());

        // git push is not a safe command
        let result = evaluate_bash_permission("git push", &vec_str(&["git", "push"]), &ctx);
        assert!(result.is_passthrough());
    }

    #[test]
    fn test_shell_permission_deny_rule() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule(
            "Shell(sudo *)",
            RuleBehavior::Deny,
            RuleDestination::Session,
        );

        let result = evaluate_shell_permission("sudo rm", &vec_str(&["sudo", "rm"]), &ctx);
        assert!(result.is_deny());
    }

    #[test]
    fn test_shell_permission_allow_rule() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule(
            "Shell(make check)",
            RuleBehavior::Allow,
            RuleDestination::Session,
        );

        let result = evaluate_shell_permission("make check", &vec_str(&["make", "check"]), &ctx);
        assert!(result.is_allow());
    }

    #[test]
    fn test_deny_rule_takes_precedence_over_safe_command() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule("Bash(ls)", RuleBehavior::Deny, RuleDestination::Session);

        // Even though ls is safe, the deny rule takes precedence
        let result = evaluate_bash_permission("ls", &vec_str(&["ls"]), &ctx);
        assert!(result.is_deny());
    }
}
