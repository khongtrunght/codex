//! Agent type configuration for sub-agent spawning via the Task tool.
//!
//! This module defines the configuration schema for different types of sub-agents
//! that can be spawned by the Task tool. Each agent type can have its own model,
//! tools, system prompt, and other settings.

use crate::tools::spec::ApplyToolConfig;
use crate::tools::spec::EXEC_COMMAND_TOOL_NAME;
use crate::tools::spec::GLOB_TOOL_NAME;
use crate::tools::spec::GREP_FILES_TOOL_NAME;
use crate::tools::spec::LIST_DIR_TOOL_NAME;
use crate::tools::spec::READ_FILE_TOOL_NAME;
use crate::tools::spec::SHELL_COMMAND_TOOL_NAME;
use crate::tools::spec::SHELL_TOOL_NAME;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::EditToolType;
use serde::Deserialize;
use serde::Serialize;
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
    pub description: String,

    /// Model override in "provider:model" format (e.g., "anthropic:claude-sonnet-4-20250514").
    /// If None, inherits from parent session.
    pub model: Option<String>,

    /// System prompt additions for this agent type.
    /// This is appended to the parent session's instructions.
    pub system_prompt: Option<String>,

    /// Tools configuration for this agent type.
    /// Maps tool names to enabled/disabled state.
    /// If None, inherits parent's tools minus task.
    pub tools: Option<Vec<String>>,

    /// Maximum steps (model turns) before forcing completion.
    pub max_steps: Option<u32>,

    /// Temperature override for model sampling.
    pub temperature: Option<f32>,

    /// Whether this agent type is hidden from LLM selection.
    /// Hidden agents can still be used explicitly but won't appear in descriptions.
    #[serde(default)]
    pub hidden: bool,

    /// Whether fork context or not.
    /// If true, the agent will inherit the context from the parent agent.
    #[serde(default)]
    pub fork_context: bool,
}

/// Registry of all available agent types.
///
/// The registry is initialized with built-in defaults and can be extended
/// with user-defined agent types from the config file.
#[derive(Debug, Clone, Default)]
pub struct AgentTypeRegistry {
    agents: HashMap<String, AgentTypeConfig>,
}

const EXPLORE_AGENT_PROMPT_TEMPLATE: &str = include_str!("../explore_agent_prompt.md");
const PLAN_SUB_AGENT_PROMPT_TEMPLATE: &str = include_str!("../plan_subagent_prompt.md");

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
                description:
                    r#"General-purpose agent for researching complex questions and executing multi-step tasks. Use this agent to execute multiple units of work in parallel."#
                        .to_string()
                ,
                model: None,
                system_prompt: None,
                tools: None,
                max_steps: Some(50),
                temperature: None,
                hidden: false,
                fork_context: false,
            },
        );

        // Exploration agent - optimized for codebase exploration (read-only)
        agents.insert(
            "explore".to_string(),
            AgentTypeConfig {
                name: "explore".to_string(),
                description:
                    r#"Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns (eg. "src/components/**/*.tsx"), search code for keywords (eg. "API endpoints"), or answer questions about the codebase (eg. "how do API endpoints work?"). When calling this agent, specify the desired thoroughness level: "quick" for basic searches, "medium" for moderate exploration, or "very thorough" for comprehensive analysis across multiple locations and naming conventions."#.to_string()
                ,
                model: None,
                system_prompt: Some(
                    EXPLORE_AGENT_PROMPT_TEMPLATE
                    .apply_tool_config(
                    Some(EditToolType::FileEdit),
                    ConfigShellToolType::Bash
                    ) // TODO: make tool config customizable in the args
                ),
                tools: Some(vec![
                    READ_FILE_TOOL_NAME.to_string(),
                    GREP_FILES_TOOL_NAME.to_string(),
                    LIST_DIR_TOOL_NAME.to_string(),
                    GLOB_TOOL_NAME.to_string(),
                    SHELL_TOOL_NAME.to_string(),
                    EXEC_COMMAND_TOOL_NAME.to_string(),
                    SHELL_COMMAND_TOOL_NAME.to_string(),
                ]),
                max_steps: Some(30),
                temperature: None,
                hidden: false,
                fork_context: false,
            },
        );

        // Planning agent - for creating implementation plans
        agents.insert(
            "plan".to_string(),
            AgentTypeConfig {
                name: "plan".to_string(),
                description:
                    r#"Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns (eg. "src/components/**/*.tsx"), search code for keywords (eg. "API endpoints"), or answer questions about the codebase (eg. "how do API endpoints work?"). When calling this agent, specify the desired thoroughness level: "quick" for basic searches, "medium" for moderate exploration, or "very thorough" for comprehensive analysis across multiple locations and naming conventions."#.to_string()
                ,
                model: None,
                system_prompt: Some(
                    PLAN_SUB_AGENT_PROMPT_TEMPLATE
                    .apply_tool_config(
                        Some(EditToolType::FileEdit),
                        ConfigShellToolType::Bash
                    )
                ),
                tools: Some(vec![
                    READ_FILE_TOOL_NAME.to_string(),
                    GREP_FILES_TOOL_NAME.to_string(),
                    LIST_DIR_TOOL_NAME.to_string(),
                    GLOB_TOOL_NAME.to_string(),
                ]),
                max_steps: Some(40),
                temperature: None,
                hidden: false,
                fork_context: false,
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

    /// Generate agent configs for the task tool schema.
    ///
    /// This produces a list of available agent types and their descriptions
    /// that can be embedded in the tool's description for LLM consumption.
    pub fn agent_configs(&self) -> Vec<AgentTypeConfig> {
        let mut agents = self.visible_agents();

        agents.sort_by(|(a, _), (b, _)| a.cmp(b));

        agents
            .into_iter()
            .map(|(_, config)| config.clone())
            .collect()
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
                description: "Custom agent".to_string(),
                ..Default::default()
            },
        );

        registry.merge_from_config(user_agents);

        assert!(registry.get("custom").is_some());
        assert_eq!(registry.get("custom").unwrap().description, "Custom agent");
    }

    #[test]
    fn test_merge_from_config_overrides_existing() {
        let mut registry = AgentTypeRegistry::with_defaults();
        let mut user_agents = HashMap::new();
        user_agents.insert(
            "explore".to_string(),
            AgentTypeConfig {
                name: "explore".to_string(),
                description: "Overridden description".to_string(),
                max_steps: Some(100),
                ..Default::default()
            },
        );

        registry.merge_from_config(user_agents);

        let explore = registry.get("explore").unwrap();
        assert_eq!(explore.description, "Overridden description");
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
                description: "Description A".to_string(),
                hidden: false,
                ..Default::default()
            },
        );
        registry.agents.insert(
            "agent_b".to_string(),
            AgentTypeConfig {
                name: "agent_b".to_string(),
                description: "Description B".to_string(),
                hidden: false,
                ..Default::default()
            },
        );

        let descriptions = registry.agent_configs();
        assert_eq!(descriptions.len(), 2);
        assert_eq!(descriptions[0].name, "agent_a");
        assert_eq!(descriptions[0].description, "Description A");
        assert_eq!(descriptions[1].name, "agent_b");
        assert_eq!(descriptions[1].description, "Description B");
    }

    #[test]
    fn test_empty_name_gets_key() {
        let mut registry = AgentTypeRegistry::new();
        let mut user_agents = HashMap::new();
        user_agents.insert(
            "my_agent".to_string(),
            AgentTypeConfig {
                name: String::new(), // Empty name
                description: "Test".to_string(),
                ..Default::default()
            },
        );

        registry.merge_from_config(user_agents);

        assert_eq!(registry.get("my_agent").unwrap().name, "my_agent");
    }
}
