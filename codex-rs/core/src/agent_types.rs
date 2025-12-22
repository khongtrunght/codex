//! Agent type configuration for sub-agent spawning via the Task tool.
//!
//! This module defines the configuration schema for different types of sub-agents
//! that can be spawned by the Task tool. Each agent type can have its own model,
//! tools, system prompt, and other settings.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a sub-agent type.
///
/// Agent types define specialized configurations for sub-agents spawned by the Task tool.
/// Each type can override the parent session's model, tools, and prompts.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct AgentTypeConfig {
    /// Human-readable name for this agent type.
    #[serde(default)]
    pub name: String,

    /// Description shown to the LLM for agent selection.
    /// This helps the model understand when to use this agent type.
    pub description: Option<String>,

    /// Model override in "provider:model" format (e.g., "anthropic:claude-sonnet-4-20250514").
    /// If None, inherits from parent session.
    pub model: Option<String>,

    /// System prompt additions for this agent type.
    /// This is appended to the parent session's instructions.
    pub system_prompt: Option<String>,

    /// Tools configuration for this agent type.
    /// Maps tool names to enabled/disabled state.
    /// If None, inherits parent's tools minus task/todowrite/todoread.
    pub tools: Option<HashMap<String, bool>>,

    /// Maximum steps (model turns) before forcing completion.
    pub max_steps: Option<u32>,

    /// Temperature override for model sampling.
    pub temperature: Option<f32>,

    /// Whether this agent type is hidden from LLM selection.
    /// Hidden agents can still be used explicitly but won't appear in descriptions.
    #[serde(default)]
    pub hidden: bool,
}

/// Registry of all available agent types.
///
/// The registry is initialized with built-in defaults and can be extended
/// with user-defined agent types from the config file.
#[derive(Debug, Clone, Default)]
pub struct AgentTypeRegistry {
    agents: HashMap<String, AgentTypeConfig>,
}

impl AgentTypeRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// Create registry with built-in default agent types.
    pub fn with_defaults() -> Self {
        let mut agents = HashMap::new();

        // General-purpose sub-agent for complex multi-step tasks
        agents.insert(
            "general".to_string(),
            AgentTypeConfig {
                name: "general".to_string(),
                description: Some(
                    "General-purpose agent for complex multi-step tasks. Use for research, \
                     code exploration, and tasks requiring multiple tool calls."
                        .to_string(),
                ),
                model: None,
                system_prompt: None,
                tools: None,
                max_steps: Some(50),
                temperature: None,
                hidden: false,
            },
        );

        // Exploration agent - optimized for codebase exploration (read-only)
        agents.insert(
            "explore".to_string(),
            AgentTypeConfig {
                name: "explore".to_string(),
                description: Some(
                    "Fast agent for exploring codebases, finding files, and searching code. \
                     Read-only - cannot modify files or run commands."
                        .to_string(),
                ),
                model: None,
                system_prompt: Some(
                    "Focus on exploration and information gathering. Be thorough but efficient. \
                     Return specific file paths and line numbers when possible."
                        .to_string(),
                ),
                tools: Some(HashMap::from([
                    ("read_file".to_string(), true),
                    ("grep_files".to_string(), true),
                    ("list_dir".to_string(), true),
                    ("shell".to_string(), false),
                    ("apply_patch".to_string(), false),
                ])),
                max_steps: Some(30),
                temperature: None,
                hidden: false,
            },
        );

        // Planning agent - for creating implementation plans
        agents.insert(
            "plan".to_string(),
            AgentTypeConfig {
                name: "plan".to_string(),
                description: Some(
                    "Agent for creating detailed implementation plans. Explores the codebase \
                     and produces structured plans with specific file references."
                        .to_string(),
                ),
                model: None,
                system_prompt: Some(
                    "Create thorough, actionable implementation plans. Include specific file paths, \
                     function names, and step-by-step instructions. Be explicit about changes needed."
                        .to_string(),
                ),
                tools: Some(HashMap::from([
                    ("read_file".to_string(), true),
                    ("grep_files".to_string(), true),
                    ("list_dir".to_string(), true),
                    ("shell".to_string(), false),
                    ("apply_patch".to_string(), false),
                ])),
                max_steps: Some(40),
                temperature: None,
                hidden: false,
            },
        );

        Self { agents }
    }

    /// Merge user-defined agent types from config.
    ///
    /// User-defined agents override built-in agents with the same key.
    pub fn merge_from_config(&mut self, user_agents: HashMap<String, AgentTypeConfig>) {
        for (key, mut config) in user_agents {
            // Ensure name is set if not provided
            if config.name.is_empty() {
                config.name = key.clone();
            }
            self.agents.insert(key, config);
        }
    }

    /// Get agent type configuration by name.
    pub fn get(&self, name: &str) -> Option<&AgentTypeConfig> {
        self.agents.get(name)
    }

    /// List all visible agent types for LLM description.
    pub fn visible_agents(&self) -> Vec<(&String, &AgentTypeConfig)> {
        self.agents
            .iter()
            .filter(|(_, config)| !config.hidden)
            .collect()
    }

    /// Generate description string for the task tool schema.
    ///
    /// This produces a formatted list of available agent types and their descriptions
    /// that can be embedded in the tool's description for LLM consumption.
    pub fn generate_agent_descriptions(&self) -> String {
        let mut agents: Vec<_> = self.visible_agents();
        // Sort alphabetically for consistent output
        agents.sort_by(|(a, _), (b, _)| a.cmp(b));

        agents
            .iter()
            .map(|(key, config)| {
                format!(
                    "- {}: {}",
                    key,
                    config
                        .description
                        .as_deref()
                        .unwrap_or("No description available")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Get all agent type names (including hidden ones).
    pub fn all_names(&self) -> Vec<&str> {
        self.agents.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_registry_has_expected_agents() {
        let registry = AgentTypeRegistry::with_defaults();

        assert!(registry.get("general").is_some());
        assert!(registry.get("explore").is_some());
        assert!(registry.get("plan").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_merge_from_config_adds_new_agent() {
        let mut registry = AgentTypeRegistry::with_defaults();
        let mut user_agents = HashMap::new();
        user_agents.insert(
            "custom".to_string(),
            AgentTypeConfig {
                name: "custom".to_string(),
                description: Some("Custom agent".to_string()),
                ..Default::default()
            },
        );

        registry.merge_from_config(user_agents);

        assert!(registry.get("custom").is_some());
        assert_eq!(
            registry.get("custom").unwrap().description,
            Some("Custom agent".to_string())
        );
    }

    #[test]
    fn test_merge_from_config_overrides_existing() {
        let mut registry = AgentTypeRegistry::with_defaults();
        let mut user_agents = HashMap::new();
        user_agents.insert(
            "explore".to_string(),
            AgentTypeConfig {
                name: "explore".to_string(),
                description: Some("Overridden description".to_string()),
                max_steps: Some(100),
                ..Default::default()
            },
        );

        registry.merge_from_config(user_agents);

        let explore = registry.get("explore").unwrap();
        assert_eq!(
            explore.description,
            Some("Overridden description".to_string())
        );
        assert_eq!(explore.max_steps, Some(100));
    }

    #[test]
    fn test_hidden_agents_not_in_visible() {
        let mut registry = AgentTypeRegistry::new();
        registry.agents.insert(
            "visible".to_string(),
            AgentTypeConfig {
                name: "visible".to_string(),
                hidden: false,
                ..Default::default()
            },
        );
        registry.agents.insert(
            "hidden".to_string(),
            AgentTypeConfig {
                name: "hidden".to_string(),
                hidden: true,
                ..Default::default()
            },
        );

        let visible = registry.visible_agents();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].0, "visible");
    }

    #[test]
    fn test_generate_agent_descriptions() {
        let mut registry = AgentTypeRegistry::new();
        registry.agents.insert(
            "agent_a".to_string(),
            AgentTypeConfig {
                name: "agent_a".to_string(),
                description: Some("Description A".to_string()),
                hidden: false,
                ..Default::default()
            },
        );
        registry.agents.insert(
            "agent_b".to_string(),
            AgentTypeConfig {
                name: "agent_b".to_string(),
                description: Some("Description B".to_string()),
                hidden: false,
                ..Default::default()
            },
        );

        let descriptions = registry.generate_agent_descriptions();
        assert!(descriptions.contains("- agent_a: Description A"));
        assert!(descriptions.contains("- agent_b: Description B"));
    }

    #[test]
    fn test_empty_name_gets_key() {
        let mut registry = AgentTypeRegistry::new();
        let mut user_agents = HashMap::new();
        user_agents.insert(
            "my_agent".to_string(),
            AgentTypeConfig {
                name: String::new(), // Empty name
                description: Some("Test".to_string()),
                ..Default::default()
            },
        );

        registry.merge_from_config(user_agents);

        assert_eq!(registry.get("my_agent").unwrap().name, "my_agent");
    }
}
