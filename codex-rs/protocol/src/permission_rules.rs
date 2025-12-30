//! Permission Rules types.
//!
//! This module defines types for permission rules that control tool access.
//! Rules are stored as strings in format `ToolName(content)` following Claude Code's pattern.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use ts_rs::TS;

/// A permission rule in format "ToolName(content)".
///
/// Rules define patterns for allowing, denying, or prompting for tool operations.
/// The content field uses glob patterns for matching.
///
/// # Examples
///
/// - `Read(*)` - Allow reading any file
/// - `Edit(src/**)` - Allow editing in src/ directory
/// - `Bash(npm:*)` - Allow npm commands
/// - `Bash(cargo test:*)` - Allow cargo test commands
/// - `WebFetch(domain:docs.rs)` - Allow fetching from docs.rs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRule {
    /// Tool name (e.g., "Read", "Edit", "Bash", "WebFetch").
    pub tool_name: String,
    /// Rule content/pattern (e.g., "src/*", "npm:*", "domain:docs.rs").
    /// None means the rule applies to all uses of the tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

impl PermissionRule {
    /// Create a new permission rule.
    pub fn new(tool_name: impl Into<String>, content: Option<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            content,
        }
    }

    /// Create a rule that matches all uses of a tool.
    pub fn all(tool_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            content: Some("*".to_string()),
        }
    }

    /// Parse from string format "ToolName(content)" or just "ToolName".
    ///
    /// # Examples
    ///
    /// ```
    /// use codex_protocol::permission_rules::PermissionRule;
    ///
    /// let rule = PermissionRule::parse("Edit(src/*)");
    /// assert_eq!(rule.tool_name, "Edit");
    /// assert_eq!(rule.content, Some("src/*".to_string()));
    ///
    /// let rule = PermissionRule::parse("Read");
    /// assert_eq!(rule.tool_name, "Read");
    /// assert_eq!(rule.content, None);
    /// ```
    pub fn parse(s: &str) -> Self {
        // Match pattern: ToolName(content)
        if let Some(paren_start) = s.find('(') {
            if let Some(paren_end) = s.rfind(')') {
                if paren_end > paren_start {
                    let tool_name = s[..paren_start].to_string();
                    let content = s[paren_start + 1..paren_end].to_string();
                    return Self {
                        tool_name,
                        content: Some(content),
                    };
                }
            }
        }

        // No parentheses, just tool name
        Self {
            tool_name: s.to_string(),
            content: None,
        }
    }

    /// Check if this rule matches the given tool name and input.
    ///
    /// Uses glob pattern matching for the content field.
    pub fn matches(&self, tool_name: &str, input: &str) -> bool {
        if self.tool_name != tool_name {
            return false;
        }

        match &self.content {
            None => true, // No content means match all
            Some(pattern) => {
                if pattern == "*" {
                    return true;
                }

                // Try glob pattern matching
                match glob::Pattern::new(pattern) {
                    Ok(glob_pattern) => glob_pattern.matches(input),
                    Err(_) => {
                        // If glob parsing fails, fall back to exact match
                        pattern == input
                    }
                }
            }
        }
    }
}

impl fmt::Display for PermissionRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.content {
            Some(c) => write!(f, "{}({})", self.tool_name, c),
            None => write!(f, "{}", self.tool_name),
        }
    }
}

/// Rule behavior - what to do when rule matches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase")]
pub enum RuleBehavior {
    /// Allow the operation without prompting.
    Allow,
    /// Deny the operation without prompting.
    Deny,
    /// Always ask for confirmation, even if would otherwise be auto-approved.
    Ask,
}

impl fmt::Display for RuleBehavior {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allow => write!(f, "allow"),
            Self::Deny => write!(f, "deny"),
            Self::Ask => write!(f, "ask"),
        }
    }
}

/// Where rules are stored/applied.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase")]
pub enum RuleDestination {
    /// In-memory only, valid for current session.
    Session,
    /// Project-level (.codex/settings.toml).
    Project,
    /// User-level (~/.codex/settings.toml).
    User,
}

impl Default for RuleDestination {
    fn default() -> Self {
        Self::Session
    }
}

impl fmt::Display for RuleDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session => write!(f, "session"),
            Self::Project => write!(f, "project"),
            Self::User => write!(f, "user"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rule_with_content() {
        let rule = PermissionRule::parse("Edit(src/**)");
        assert_eq!(rule.tool_name, "Edit");
        assert_eq!(rule.content, Some("src/**".to_string()));
        assert_eq!(rule.to_string(), "Edit(src/**)");
    }

    #[test]
    fn test_parse_rule_without_content() {
        let rule = PermissionRule::parse("Read");
        assert_eq!(rule.tool_name, "Read");
        assert_eq!(rule.content, None);
        assert_eq!(rule.to_string(), "Read");
    }

    #[test]
    fn test_parse_bash_command() {
        let rule = PermissionRule::parse("Bash(npm:*)");
        assert_eq!(rule.tool_name, "Bash");
        assert_eq!(rule.content, Some("npm:*".to_string()));
    }

    #[test]
    fn test_parse_webfetch_domain() {
        let rule = PermissionRule::parse("WebFetch(domain:docs.rs)");
        assert_eq!(rule.tool_name, "WebFetch");
        assert_eq!(rule.content, Some("domain:docs.rs".to_string()));
    }

    #[test]
    fn test_rule_matches_exact() {
        let rule = PermissionRule::parse("Edit(src/main.rs)");
        assert!(rule.matches("Edit", "src/main.rs"));
        assert!(!rule.matches("Edit", "src/lib.rs"));
        assert!(!rule.matches("Read", "src/main.rs"));
    }

    #[test]
    fn test_rule_matches_glob() {
        let rule = PermissionRule::parse("Edit(src/**)");
        assert!(rule.matches("Edit", "src/main.rs"));
        assert!(rule.matches("Edit", "src/lib/mod.rs"));
        assert!(!rule.matches("Edit", "tests/test.rs"));
    }

    #[test]
    fn test_rule_matches_wildcard() {
        let rule = PermissionRule::parse("Read(*)");
        assert!(rule.matches("Read", "any/path/here.rs"));
        assert!(rule.matches("Read", ""));
    }

    #[test]
    fn test_rule_matches_no_content() {
        let rule = PermissionRule::new("Bash", None);
        assert!(rule.matches("Bash", "ls -la"));
        assert!(rule.matches("Bash", "cargo test"));
    }

    #[test]
    fn test_rule_serialization() {
        let rule = PermissionRule::parse("Edit(src/**)");
        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("Edit"));
        assert!(json.contains("src/**"));

        let deserialized: PermissionRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_behavior_display() {
        assert_eq!(RuleBehavior::Allow.to_string(), "allow");
        assert_eq!(RuleBehavior::Deny.to_string(), "deny");
        assert_eq!(RuleBehavior::Ask.to_string(), "ask");
    }

    #[test]
    fn test_destination_default() {
        assert_eq!(RuleDestination::default(), RuleDestination::Session);
    }
}
