//! Permission configuration loading and management.
//!
//! This module handles loading permission rules from TOML config files:
//! - `.codex/settings.toml` (project level)
//! - `~/.codex/settings.toml` (user level)

use std::path::Path;
use std::path::PathBuf;

use codex_protocol::permission_context::PermissionContext;
use codex_protocol::permission_rules::RuleBehavior;
use codex_protocol::permission_rules::RuleDestination;
use serde::Deserialize;
use serde::Serialize;

/// Permission settings as stored in TOML config files.
///
/// Example config:
/// ```toml
/// [permissions]
/// allow = [
///     "Read(*)",
///     "Bash(cargo test:*)",
///     "WebFetch(domain:docs.rs)",
/// ]
/// deny = [
///     "Edit(.env)",
///     "Edit(.codex/*)",
/// ]
/// ask = []
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionConfig {
    /// Rules that always allow operations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,

    /// Rules that always deny operations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,

    /// Rules that always ask for confirmation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ask: Vec<String>,
}

impl PermissionConfig {
    /// Create an empty permission config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if this config has any rules.
    pub fn has_rules(&self) -> bool {
        !self.allow.is_empty() || !self.deny.is_empty() || !self.ask.is_empty()
    }

    /// Merge another config into this one (additive).
    pub fn merge(&mut self, other: &PermissionConfig) {
        self.allow.extend(other.allow.iter().cloned());
        self.deny.extend(other.deny.iter().cloned());
        self.ask.extend(other.ask.iter().cloned());
    }

    /// Apply these rules to a permission context with the given destination.
    pub fn apply_to_context(&self, ctx: &mut PermissionContext, destination: RuleDestination) {
        for rule in &self.allow {
            ctx.add_rule(rule, RuleBehavior::Allow, destination);
        }
        for rule in &self.deny {
            ctx.add_rule(rule, RuleBehavior::Deny, destination);
        }
        for rule in &self.ask {
            ctx.add_rule(rule, RuleBehavior::Ask, destination);
        }
    }
}

/// Wrapper for the full settings TOML that contains permissions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SettingsToml {
    #[serde(default)]
    permissions: PermissionConfig,
}

/// Load permission rules from project config (.codex/settings.toml).
///
/// Returns None if the file doesn't exist or can't be parsed.
pub fn load_project_permissions(cwd: &Path) -> Option<PermissionConfig> {
    let config_path = cwd.join(".codex").join("settings.toml");
    load_permissions_from_file(&config_path)
}

/// Load permission rules from user config (~/.codex/settings.toml).
///
/// Returns None if the file doesn't exist or can't be parsed.
pub fn load_user_permissions() -> Option<PermissionConfig> {
    let home = dirs::home_dir()?;
    let config_path = home.join(".codex").join("settings.toml");
    load_permissions_from_file(&config_path)
}

/// Load permission rules from a specific TOML file.
fn load_permissions_from_file(path: &Path) -> Option<PermissionConfig> {
    if !path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    let settings: SettingsToml = toml::from_str(&content).ok()?;

    if settings.permissions.has_rules() {
        Some(settings.permissions)
    } else {
        None
    }
}

/// Load all permission rules and apply them to a permission context.
///
/// Loads from both user and project configs, applying them with appropriate destinations.
pub fn load_and_apply_permissions(ctx: &mut PermissionContext, cwd: &Path) {
    // Load user-level permissions
    if let Some(user_config) = load_user_permissions() {
        user_config.apply_to_context(ctx, RuleDestination::User);
    }

    // Load project-level permissions
    if let Some(project_config) = load_project_permissions(cwd) {
        project_config.apply_to_context(ctx, RuleDestination::Project);
    }
}

/// Get the path to the project permissions config file.
pub fn project_permissions_path(cwd: &Path) -> PathBuf {
    cwd.join(".codex").join("settings.toml")
}

/// Get the path to the user permissions config file.
pub fn user_permissions_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex").join("settings.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_config_default() {
        let config = PermissionConfig::default();
        assert!(config.allow.is_empty());
        assert!(config.deny.is_empty());
        assert!(config.ask.is_empty());
        assert!(!config.has_rules());
    }

    #[test]
    fn test_permission_config_merge() {
        let mut config1 = PermissionConfig {
            allow: vec!["Read(*)".to_string()],
            deny: vec!["Edit(.env)".to_string()],
            ask: vec![],
        };

        let config2 = PermissionConfig {
            allow: vec!["Bash(cargo test:*)".to_string()],
            deny: vec![],
            ask: vec!["Bash(rm:*)".to_string()],
        };

        config1.merge(&config2);

        assert_eq!(config1.allow.len(), 2);
        assert_eq!(config1.deny.len(), 1);
        assert_eq!(config1.ask.len(), 1);
    }

    #[test]
    fn test_parse_toml() {
        let toml_content = r#"
[permissions]
allow = [
    "Read(*)",
    "Bash(cargo test:*)",
]
deny = [
    "Edit(.env)",
]
ask = []
"#;

        let settings: SettingsToml = toml::from_str(toml_content).unwrap();
        assert_eq!(settings.permissions.allow.len(), 2);
        assert_eq!(settings.permissions.deny.len(), 1);
        assert!(settings.permissions.ask.is_empty());
    }

    #[test]
    fn test_apply_to_context() {
        let config = PermissionConfig {
            allow: vec!["Read(*)".to_string()],
            deny: vec!["Edit(.env)".to_string()],
            ask: vec![],
        };

        let mut ctx = PermissionContext::default();
        config.apply_to_context(&mut ctx, RuleDestination::Project);

        assert!(ctx.has_allow_rule("Read", "any.file"));
        assert!(ctx.has_deny_rule("Edit", ".env"));
    }
}
