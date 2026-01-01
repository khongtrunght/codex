//! File-specific permission evaluation functions.
//!
//! This module provides helper functions for evaluating file read/write permissions
//! following the Claude Code pattern where each tool extracts its own input.

use std::path::Path;

use codex_protocol::permission_context::PermissionContext;
use codex_protocol::permission_mode::PermissionMode;

use super::tool_permission::evaluate_rules_permission;
use super::tool_permission::ToolPermissionReason;
use super::tool_permission::ToolPermissionResult;

/// Evaluate read permission for file operations.
///
/// Permission check order (first non-passthrough wins):
/// 1. Deny rules matching Read(path)
/// 2. Allow rules matching Read(path)
/// 3. Working directory auto-allow (if path is within cwd)
/// 4. Passthrough (defer to existing approval flow)
///
/// # Arguments
///
/// * `path` - The file path being accessed
/// * `ctx` - Permission context with rules
/// * `cwd` - Optional current working directory for relative path resolution
pub fn evaluate_file_read_permission(
    path: &str,
    ctx: &PermissionContext,
    cwd: Option<&Path>,
) -> ToolPermissionResult {
    // Check rules first (deny > ask > allow)
    let rules_result = evaluate_rules_permission(ctx, "Read", path);
    if rules_result.stops_chain() {
        return rules_result;
    }

    // Auto-allow reads within working directory
    if is_in_working_directory(path, ctx, cwd) {
        return ToolPermissionResult::allow(ToolPermissionReason::WorkingDirectory);
    }

    // Passthrough to existing approval flow
    ToolPermissionResult::passthrough()
}

/// Evaluate write permission for file operations.
///
/// Permission check order (first non-passthrough wins):
/// 1. Plan mode restriction (only plan file allowed)
/// 2. Deny rules matching Edit/Write(path)
/// 3. Allow rules matching Edit/Write(path)
/// 4. Passthrough (defer to existing approval flow)
///
/// Note: Write operations do NOT auto-allow for working directory.
/// This requires explicit user approval or rules.
pub fn evaluate_file_write_permission(
    path: &str,
    ctx: &PermissionContext,
    tool_name: &str,
) -> ToolPermissionResult {
    // Plan mode check - only plan file can be written
    if let PermissionMode::Plan { plan_file_path } = &ctx.mode {
        // Normalize both paths for comparison
        let normalized_path = normalize_path(path);
        let normalized_plan = normalize_path(plan_file_path);

        if normalized_path != normalized_plan {
            return ToolPermissionResult::deny(
                format!(
                    "In plan mode, only the plan file can be modified. Attempted to modify '{}' but only '{}' is allowed.",
                    path, plan_file_path
                ),
                ToolPermissionReason::Mode {
                    mode: "plan".to_string(),
                },
            );
        }
        // Writing to plan file - allow
        return ToolPermissionResult::allow(ToolPermissionReason::Mode {
            mode: "plan".to_string(),
        });
    }

    // Check rules (deny > ask > allow)
    let rules_result = evaluate_rules_permission(ctx, tool_name, path);
    if rules_result.stops_chain() {
        return rules_result;
    }

    // Passthrough to existing approval flow
    ToolPermissionResult::passthrough()
}

/// Check if a path is within any of the configured working directories.
///
/// # Arguments
///
/// * `path` - The file path to check
/// * `ctx` - Permission context with additional working directories
/// * `cwd` - The primary current working directory
pub fn is_in_working_directory(path: &str, ctx: &PermissionContext, cwd: Option<&Path>) -> bool {
    let path = Path::new(path);

    // Check relative paths (., .., relative/path)
    if !path.is_absolute() {
        // Relative paths are implicitly within the working directory
        // unless they escape with ..
        if !path_escapes_directory(path) {
            return true;
        }
    }

    // Check against primary cwd
    if let Some(cwd) = cwd {
        if path_is_within(path, cwd) {
            return true;
        }
    }

    // Check against additional working directories
    for working_dir in &ctx.additional_working_directories {
        if path_is_within(path, working_dir) {
            return true;
        }
    }

    false
}

/// Check if a path is within a directory.
fn path_is_within(path: &Path, dir: &Path) -> bool {
    // Try direct prefix match first
    if path.starts_with(dir) {
        return true;
    }

    // Try canonicalized comparison
    if let (Ok(canonical_path), Ok(canonical_dir)) = (path.canonicalize(), dir.canonicalize()) {
        if canonical_path.starts_with(&canonical_dir) {
            return true;
        }
    }

    false
}

/// Check if a relative path escapes the current directory using ..
fn path_escapes_directory(path: &Path) -> bool {
    use std::path::Component;

    let mut depth: i32 = 0;
    for component in path.components() {
        match component {
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            Component::Normal(_) => {
                depth += 1;
            }
            _ => {}
        }
    }
    false
}

/// Normalize a path string for comparison.
fn normalize_path(path: &str) -> String {
    // Remove trailing slashes and normalize separators
    let normalized = path.trim_end_matches('/').trim_end_matches('\\');
    normalized.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use codex_protocol::permission_rules::RuleBehavior;
    use codex_protocol::permission_rules::RuleDestination;

    use super::*;

    #[test]
    fn test_read_permission_deny_rule() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule("Read(.env)", RuleBehavior::Deny, RuleDestination::Session);

        let result = evaluate_file_read_permission(".env", &ctx, None);
        assert!(result.is_deny());
    }

    #[test]
    fn test_read_permission_allow_rule() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule(
            "Read(*.rs)",
            RuleBehavior::Allow,
            RuleDestination::Session,
        );

        let result = evaluate_file_read_permission("main.rs", &ctx, None);
        assert!(result.is_allow());
    }

    #[test]
    fn test_read_permission_working_directory() {
        let mut ctx = PermissionContext::default();
        let cwd = PathBuf::from("/home/user/project");
        ctx.additional_working_directories.push(cwd.clone());

        let result =
            evaluate_file_read_permission("/home/user/project/src/main.rs", &ctx, Some(&cwd));
        assert!(result.is_allow());
        assert!(matches!(
            result.reason,
            ToolPermissionReason::WorkingDirectory
        ));
    }

    #[test]
    fn test_read_permission_outside_working_directory() {
        let mut ctx = PermissionContext::default();
        let cwd = PathBuf::from("/home/user/project");
        ctx.additional_working_directories.push(cwd.clone());

        let result = evaluate_file_read_permission("/etc/passwd", &ctx, Some(&cwd));
        assert!(result.is_passthrough());
    }

    #[test]
    fn test_read_permission_relative_path() {
        let ctx = PermissionContext::default();

        // Relative paths within cwd should be allowed
        let result = evaluate_file_read_permission("src/main.rs", &ctx, None);
        assert!(result.is_allow());

        // Path that escapes should passthrough
        let result = evaluate_file_read_permission("../../etc/passwd", &ctx, None);
        assert!(result.is_passthrough());
    }

    #[test]
    fn test_write_permission_plan_mode() {
        let mut ctx = PermissionContext::default();
        ctx.mode = PermissionMode::Plan {
            plan_file_path: "/home/user/plan.md".to_string(),
        };

        // Writing to plan file should be allowed
        let result = evaluate_file_write_permission("/home/user/plan.md", &ctx, "Write");
        assert!(result.is_allow());

        // Writing to other files should be denied
        let result = evaluate_file_write_permission("/home/user/src/main.rs", &ctx, "Write");
        assert!(result.is_deny());
        assert!(result.message.unwrap().contains("plan mode"));
    }

    #[test]
    fn test_write_permission_deny_rule() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule(
            "Edit(.env)",
            RuleBehavior::Deny,
            RuleDestination::Session,
        );

        let result = evaluate_file_write_permission(".env", &ctx, "Edit");
        assert!(result.is_deny());
    }

    #[test]
    fn test_write_permission_allow_rule() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule(
            "Write(*.txt)",
            RuleBehavior::Allow,
            RuleDestination::Session,
        );

        let result = evaluate_file_write_permission("notes.txt", &ctx, "Write");
        assert!(result.is_allow());
    }

    #[test]
    fn test_is_in_working_directory_cwd() {
        let ctx = PermissionContext::default();
        let cwd = PathBuf::from("/home/user/project");

        assert!(is_in_working_directory(
            "/home/user/project/src/main.rs",
            &ctx,
            Some(&cwd)
        ));
        assert!(is_in_working_directory("/home/user/project", &ctx, Some(&cwd)));
        assert!(!is_in_working_directory("/etc/passwd", &ctx, Some(&cwd)));
        assert!(!is_in_working_directory("/home/user/other", &ctx, Some(&cwd)));
    }

    #[test]
    fn test_is_in_working_directory_additional() {
        let mut ctx = PermissionContext::default();
        ctx.additional_working_directories
            .push(PathBuf::from("/tmp/workspace"));

        assert!(is_in_working_directory(
            "/tmp/workspace/file.txt",
            &ctx,
            None
        ));
    }

    #[test]
    fn test_path_escapes_directory() {
        assert!(!path_escapes_directory(Path::new("src/main.rs")));
        assert!(!path_escapes_directory(Path::new("./src/main.rs")));
        assert!(!path_escapes_directory(Path::new("src/../src/main.rs")));
        assert!(path_escapes_directory(Path::new("../src/main.rs")));
        assert!(path_escapes_directory(Path::new("../../etc/passwd")));
        assert!(path_escapes_directory(Path::new("src/../../etc/passwd")));
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("/home/user/"), "/home/user");
        assert_eq!(normalize_path("/home/user"), "/home/user");
        assert_eq!(normalize_path("C:\\Users\\test\\"), "C:/Users/test");
    }
}
