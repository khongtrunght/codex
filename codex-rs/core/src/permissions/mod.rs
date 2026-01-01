//! Permission system module.
//!
//! This module provides the permission checking logic that determines
//! whether tool operations should be allowed, denied, or require user approval.
//!
//! ## Architecture
//!
//! The permission system follows the Claude Code pattern with support for
//! chained permission checks:
//!
//! 1. **Tool-specific checks** (`check_permissions` on ToolHandler)
//!    - Each tool can implement custom permission logic
//!    - Returns `ToolPermissionResult` with `Passthrough` support
//!
//! 2. **Rule-based checks** (`check_permission` function)
//!    - Evaluates against allow/deny/ask rules
//!    - Handles permission modes (BypassPermissions, DontAsk, etc.)
//!
//! 3. **DontAsk mode handling**
//!    - Converts `Ask` to `Deny` for subagents and non-interactive contexts

mod bash_permission;
mod file_permission;
mod rule_matcher;
mod tool_permission;

// Legacy API for rule-based permission checking
pub use rule_matcher::check_permission;
pub use rule_matcher::PermissionCheckReason;
pub use rule_matcher::PermissionCheckResult;

// Tool-specific permission types (Claude Code pattern)
pub use tool_permission::evaluate_mode_permission;
pub use tool_permission::evaluate_rules_permission;
pub use tool_permission::handle_dont_ask_mode;
pub use tool_permission::is_safe_tool;
pub use tool_permission::is_write_tool;
pub use tool_permission::PermissionBehavior;
pub use tool_permission::ToolPermissionReason;
pub use tool_permission::ToolPermissionResult;

// File permission helpers
pub use file_permission::evaluate_file_read_permission;
pub use file_permission::evaluate_file_write_permission;
pub use file_permission::is_in_working_directory;

// Bash permission helpers
pub use bash_permission::evaluate_bash_permission;
pub use bash_permission::evaluate_shell_permission;
