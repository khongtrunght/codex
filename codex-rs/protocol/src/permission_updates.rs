//! Permission Update types.
//!
//! This module defines actions for updating the permission context,
//! following a reducer pattern similar to Claude Code.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::permission_context::PermissionContext;
use crate::permission_mode::PermissionMode;
use crate::permission_rules::{RuleBehavior, RuleDestination};

/// Actions to update permission context.
///
/// These follow a reducer pattern where each action describes a mutation
/// to apply to the permission context.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type", rename_all = "camelCase")]
pub enum PermissionUpdate {
    /// Set the permission mode.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    SetMode {
        mode: PermissionMode,
        #[serde(default)]
        destination: RuleDestination,
    },

    /// Add permission rules.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    AddRules {
        rules: Vec<String>,
        behavior: RuleBehavior,
        destination: RuleDestination,
    },

    /// Remove permission rules.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    RemoveRules {
        rules: Vec<String>,
        behavior: RuleBehavior,
        destination: RuleDestination,
    },

    /// Replace all rules for a behavior/destination.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    ReplaceRules {
        rules: Vec<String>,
        behavior: RuleBehavior,
        destination: RuleDestination,
    },

    /// Add additional working directories.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    AddDirectories {
        directories: Vec<PathBuf>,
        #[serde(default)]
        destination: RuleDestination,
    },

    /// Set the bypass available flag.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    SetBypassAvailable { available: bool },

    /// Mark that plan mode has been exited.
    MarkPlanModeExited,
}

impl PermissionUpdate {
    /// Apply this update to a permission context.
    pub fn apply(&self, ctx: &mut PermissionContext) {
        match self {
            PermissionUpdate::SetMode { mode, .. } => {
                // Track if exiting plan mode
                if ctx.mode.is_planning() && !mode.is_planning() {
                    ctx.has_exited_plan_mode = true;
                }
                ctx.mode = mode.clone();
            }

            PermissionUpdate::AddRules {
                rules,
                behavior,
                destination,
            } => {
                for rule in rules {
                    ctx.add_rule(rule, *behavior, *destination);
                }
            }

            PermissionUpdate::RemoveRules {
                rules,
                behavior,
                destination,
            } => {
                for rule in rules {
                    ctx.remove_rule(rule, *behavior, *destination);
                }
            }

            PermissionUpdate::ReplaceRules {
                rules,
                behavior,
                destination,
            } => {
                // Clear existing rules for this behavior/destination
                let rule_map = match behavior {
                    RuleBehavior::Allow => &mut ctx.always_allow_rules,
                    RuleBehavior::Deny => &mut ctx.always_deny_rules,
                    RuleBehavior::Ask => &mut ctx.always_ask_rules,
                };
                rule_map.insert(*destination, rules.clone());
            }

            PermissionUpdate::AddDirectories { directories, .. } => {
                for dir in directories {
                    ctx.add_working_directory(dir.clone());
                }
            }

            PermissionUpdate::SetBypassAvailable { available } => {
                ctx.is_bypass_available = *available;
            }

            PermissionUpdate::MarkPlanModeExited => {
                ctx.has_exited_plan_mode = true;
            }
        }
    }

    /// Create a SetMode update.
    pub fn set_mode(mode: PermissionMode) -> Self {
        Self::SetMode {
            mode,
            destination: RuleDestination::Session,
        }
    }

    /// Create an AddRules update for session scope.
    pub fn add_session_rules(rules: Vec<String>, behavior: RuleBehavior) -> Self {
        Self::AddRules {
            rules,
            behavior,
            destination: RuleDestination::Session,
        }
    }

    /// Create an AddRules update with a single rule.
    pub fn add_rule(rule: &str, behavior: RuleBehavior, destination: RuleDestination) -> Self {
        Self::AddRules {
            rules: vec![rule.to_string()],
            behavior,
            destination,
        }
    }

    /// Create an update to enter plan mode.
    pub fn enter_plan_mode(plan_file_path: String) -> Self {
        Self::SetMode {
            mode: PermissionMode::Plan { plan_file_path },
            destination: RuleDestination::Session,
        }
    }

    /// Create an update to exit plan mode.
    pub fn exit_plan_mode() -> Self {
        Self::SetMode {
            mode: PermissionMode::Default,
            destination: RuleDestination::Session,
        }
    }
}

/// Result of a permission decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "decision", rename_all = "camelCase")]
#[ts(tag = "decision", rename_all = "camelCase")]
pub enum PermissionDecision {
    /// Allow the operation without prompting.
    Allow,
    /// Deny the operation with a reason.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Deny { reason: String },
    /// Need to prompt the user for approval.
    Ask,
}

impl PermissionDecision {
    /// Check if this is an allow decision.
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow)
    }

    /// Check if this is a deny decision.
    pub fn is_deny(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }

    /// Check if this needs user approval.
    pub fn needs_approval(&self) -> bool {
        matches!(self, Self::Ask)
    }

    /// Create an allow decision.
    pub fn allow() -> Self {
        Self::Allow
    }

    /// Create a deny decision with a reason.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }

    /// Create an ask decision.
    pub fn ask() -> Self {
        Self::Ask
    }
}

/// Reason for a permission decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type", rename_all = "camelCase")]
pub enum DecisionReason {
    /// Decision based on permission mode.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Mode { mode: String },
    /// Decision based on a matching rule.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Rule { rule: String, behavior: RuleBehavior },
    /// Decision based on tool being trusted.
    Trusted,
    /// Default decision (no specific reason).
    Default,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_mode_update() {
        let mut ctx = PermissionContext::default();
        assert!(matches!(ctx.mode, PermissionMode::Default));

        let update = PermissionUpdate::set_mode(PermissionMode::AcceptEdits);
        update.apply(&mut ctx);
        assert!(matches!(ctx.mode, PermissionMode::AcceptEdits));
    }

    #[test]
    fn test_add_rules_update() {
        let mut ctx = PermissionContext::default();

        let update =
            PermissionUpdate::add_session_rules(vec!["Read(*)".to_string()], RuleBehavior::Allow);
        update.apply(&mut ctx);

        assert!(ctx.has_allow_rule("Read", "any.file"));
    }

    #[test]
    fn test_remove_rules_update() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule("Read(*)", RuleBehavior::Allow, RuleDestination::Session);

        let update = PermissionUpdate::RemoveRules {
            rules: vec!["Read(*)".to_string()],
            behavior: RuleBehavior::Allow,
            destination: RuleDestination::Session,
        };
        update.apply(&mut ctx);

        assert!(!ctx.has_allow_rule("Read", "any.file"));
    }

    #[test]
    fn test_replace_rules_update() {
        let mut ctx = PermissionContext::default();
        ctx.add_rule("Read(*)", RuleBehavior::Allow, RuleDestination::Session);
        ctx.add_rule(
            "Edit(src/**)",
            RuleBehavior::Allow,
            RuleDestination::Session,
        );

        let update = PermissionUpdate::ReplaceRules {
            rules: vec!["Write(*)".to_string()],
            behavior: RuleBehavior::Allow,
            destination: RuleDestination::Session,
        };
        update.apply(&mut ctx);

        assert!(!ctx.has_allow_rule("Read", "any.file"));
        assert!(!ctx.has_allow_rule("Edit", "src/main.rs"));
        assert!(ctx.has_allow_rule("Write", "any.file"));
    }

    #[test]
    fn test_enter_exit_plan_mode() {
        let mut ctx = PermissionContext::default();

        let enter = PermissionUpdate::enter_plan_mode("/tmp/plan.md".to_string());
        enter.apply(&mut ctx);
        assert!(ctx.mode.is_planning());
        assert!(!ctx.has_exited_plan_mode);

        let exit = PermissionUpdate::exit_plan_mode();
        exit.apply(&mut ctx);
        assert!(!ctx.mode.is_planning());
        assert!(ctx.has_exited_plan_mode);
    }

    #[test]
    fn test_add_directories() {
        let mut ctx = PermissionContext::default();

        let update = PermissionUpdate::AddDirectories {
            directories: vec![PathBuf::from("/tmp/test"), PathBuf::from("/home/user")],
            destination: RuleDestination::Session,
        };
        update.apply(&mut ctx);

        assert_eq!(ctx.additional_working_directories.len(), 2);
    }

    #[test]
    fn test_permission_decision() {
        assert!(PermissionDecision::allow().is_allow());
        assert!(PermissionDecision::deny("test").is_deny());
        assert!(PermissionDecision::ask().needs_approval());
    }

    #[test]
    fn test_serialization() {
        let update = PermissionUpdate::enter_plan_mode("/tmp/plan.md".to_string());
        let json = serde_json::to_string(&update).unwrap();
        assert!(json.contains("setMode"));
        assert!(json.contains("plan"));

        let deserialized: PermissionUpdate = serde_json::from_str(&json).unwrap();
        match deserialized {
            PermissionUpdate::SetMode { mode, .. } => {
                assert!(mode.is_planning());
            }
            _ => panic!("Expected SetMode"),
        }
    }
}
