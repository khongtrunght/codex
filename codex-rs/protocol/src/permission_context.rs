//! Permission Context types.
//!
//! This module defines the session-level permission context that stores
//! the current permission mode and active rules.

use std::collections::HashMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::permission_mode::PermissionMode;
use crate::permission_rules::{PermissionRule, RuleBehavior, RuleDestination};

/// Session-level permission context.
///
/// This stores the current permission mode, active rules, and related state.
/// Rules from session, project, and user are combined additively.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct PermissionContext {
    /// Current permission mode.
    pub mode: PermissionMode,

    /// Additional directories with write access beyond the cwd.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_working_directories: Vec<PathBuf>,

    /// Rules that always allow (keyed by destination).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub always_allow_rules: HashMap<RuleDestination, Vec<String>>,

    /// Rules that always deny (keyed by destination).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub always_deny_rules: HashMap<RuleDestination, Vec<String>>,

    /// Rules that always ask (keyed by destination).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub always_ask_rules: HashMap<RuleDestination, Vec<String>>,

    /// Whether bypass permissions mode is available.
    /// Controlled by CLI flag `--dangerously-skip-permissions`.
    #[serde(default)]
    pub is_bypass_available: bool,

    /// Track if plan mode has been exited (for reentry detection).
    #[serde(default)]
    pub has_exited_plan_mode: bool,
}

impl Default for PermissionContext {
    fn default() -> Self {
        Self {
            mode: PermissionMode::Default,
            additional_working_directories: Vec::new(),
            always_allow_rules: HashMap::new(),
            always_deny_rules: HashMap::new(),
            always_ask_rules: HashMap::new(),
            is_bypass_available: false,
            has_exited_plan_mode: false,
        }
    }
}

impl PermissionContext {
    /// Create a new permission context with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a permission context for non-interactive (subagent) mode.
    ///
    /// This inherits the rules from a parent context but sets the mode to DontAsk.
    pub fn for_subagent(parent: &Self) -> Self {
        Self {
            mode: PermissionMode::DontAsk,
            additional_working_directories: parent.additional_working_directories.clone(),
            always_allow_rules: parent.always_allow_rules.clone(),
            always_deny_rules: parent.always_deny_rules.clone(),
            always_ask_rules: parent.always_ask_rules.clone(),
            is_bypass_available: false, // Subagents can't use bypass
            has_exited_plan_mode: false,
        }
    }

    /// Set the permission mode.
    pub fn set_mode(&mut self, mode: PermissionMode) {
        self.mode = mode;
    }

    /// Add a rule with the specified behavior and destination.
    pub fn add_rule(&mut self, rule: &str, behavior: RuleBehavior, destination: RuleDestination) {
        let rules = match behavior {
            RuleBehavior::Allow => &mut self.always_allow_rules,
            RuleBehavior::Deny => &mut self.always_deny_rules,
            RuleBehavior::Ask => &mut self.always_ask_rules,
        };

        rules
            .entry(destination)
            .or_default()
            .push(rule.to_string());
    }

    /// Remove a rule from the specified behavior and destination.
    pub fn remove_rule(
        &mut self,
        rule: &str,
        behavior: RuleBehavior,
        destination: RuleDestination,
    ) {
        let rules = match behavior {
            RuleBehavior::Allow => &mut self.always_allow_rules,
            RuleBehavior::Deny => &mut self.always_deny_rules,
            RuleBehavior::Ask => &mut self.always_ask_rules,
        };

        if let Some(dest_rules) = rules.get_mut(&destination) {
            dest_rules.retain(|r| r != rule);
        }
    }

    /// Get all rules for a behavior, merged across all destinations.
    pub fn get_rules(&self, behavior: RuleBehavior) -> Vec<&str> {
        let rules = match behavior {
            RuleBehavior::Allow => &self.always_allow_rules,
            RuleBehavior::Deny => &self.always_deny_rules,
            RuleBehavior::Ask => &self.always_ask_rules,
        };

        rules
            .values()
            .flat_map(|v| v.iter().map(|s| s.as_str()))
            .collect()
    }

    /// Check if a tool operation matches any rule with the given behavior.
    ///
    /// Returns the matching rule string if found.
    pub fn match_rule(&self, tool_name: &str, input: &str, behavior: RuleBehavior) -> Option<&str> {
        let rules = match behavior {
            RuleBehavior::Allow => &self.always_allow_rules,
            RuleBehavior::Deny => &self.always_deny_rules,
            RuleBehavior::Ask => &self.always_ask_rules,
        };

        for dest_rules in rules.values() {
            for rule_str in dest_rules {
                let rule = PermissionRule::parse(rule_str);
                if rule.matches(tool_name, input) {
                    return Some(rule_str);
                }
            }
        }
        None
    }

    /// Check if any allow rule matches the tool operation.
    pub fn has_allow_rule(&self, tool_name: &str, input: &str) -> bool {
        self.match_rule(tool_name, input, RuleBehavior::Allow)
            .is_some()
    }

    /// Check if any deny rule matches the tool operation.
    pub fn has_deny_rule(&self, tool_name: &str, input: &str) -> bool {
        self.match_rule(tool_name, input, RuleBehavior::Deny)
            .is_some()
    }

    /// Check if any ask rule matches the tool operation.
    pub fn has_ask_rule(&self, tool_name: &str, input: &str) -> bool {
        self.match_rule(tool_name, input, RuleBehavior::Ask)
            .is_some()
    }

    /// Add an additional working directory.
    pub fn add_working_directory(&mut self, path: PathBuf) {
        if !self.additional_working_directories.contains(&path) {
            self.additional_working_directories.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_context() {
        let ctx = PermissionContext::default();
        assert!(matches!(ctx.mode, PermissionMode::Default));
        assert!(!ctx.is_bypass_available);
        assert!(!ctx.has_exited_plan_mode);
        assert!(ctx.additional_working_directories.is_empty());
    }

    #[test]
    fn test_subagent_context() {
        let mut parent = PermissionContext::new();
        parent.is_bypass_available = true;
        parent.add_rule("Read(*)", RuleBehavior::Allow, RuleDestination::Session);

        let child = PermissionContext::for_subagent(&parent);
        assert!(matches!(child.mode, PermissionMode::DontAsk));
        assert!(!child.is_bypass_available); // Subagents can't use bypass
        assert!(child.has_allow_rule("Read", "anything"));
    }

    #[test]
    fn test_add_and_match_rules() {
        let mut ctx = PermissionContext::new();

        ctx.add_rule("Edit(src/**)", RuleBehavior::Allow, RuleDestination::Session);
        ctx.add_rule("Edit(.env)", RuleBehavior::Deny, RuleDestination::Project);

        assert!(ctx.has_allow_rule("Edit", "src/main.rs"));
        assert!(!ctx.has_allow_rule("Edit", "tests/test.rs"));
        assert!(ctx.has_deny_rule("Edit", ".env"));
        assert!(!ctx.has_deny_rule("Edit", ".gitignore"));
    }

    #[test]
    fn test_remove_rule() {
        let mut ctx = PermissionContext::new();

        ctx.add_rule("Read(*)", RuleBehavior::Allow, RuleDestination::Session);
        assert!(ctx.has_allow_rule("Read", "any.file"));

        ctx.remove_rule("Read(*)", RuleBehavior::Allow, RuleDestination::Session);
        assert!(!ctx.has_allow_rule("Read", "any.file"));
    }

    #[test]
    fn test_get_rules() {
        let mut ctx = PermissionContext::new();

        ctx.add_rule("Read(*)", RuleBehavior::Allow, RuleDestination::Session);
        ctx.add_rule("Edit(src/**)", RuleBehavior::Allow, RuleDestination::Project);

        let rules = ctx.get_rules(RuleBehavior::Allow);
        assert_eq!(rules.len(), 2);
        assert!(rules.contains(&"Read(*)"));
        assert!(rules.contains(&"Edit(src/**)"));
    }

    #[test]
    fn test_working_directories() {
        let mut ctx = PermissionContext::new();

        ctx.add_working_directory(PathBuf::from("/tmp/test"));
        ctx.add_working_directory(PathBuf::from("/tmp/test")); // Duplicate
        ctx.add_working_directory(PathBuf::from("/home/user"));

        assert_eq!(ctx.additional_working_directories.len(), 2);
    }
}
