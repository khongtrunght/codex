//! Permission system module.
//!
//! This module provides the permission checking logic that determines
//! whether tool operations should be allowed, denied, or require user approval.

mod rule_matcher;

pub use rule_matcher::check_permission;
pub use rule_matcher::PermissionCheckResult;
