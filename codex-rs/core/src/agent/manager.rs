use std::collections::HashMap;

use codex_protocol::model_tier::ModelTier;
use serde::Deserialize;
use serde::Serialize;

use crate::config::Config;
use crate::features::Feature;
use crate::prompt_template::ExploreAgentPrompt;
use crate::prompt_template::GeneralMainPrompt;
use crate::prompt_template::PlanSubagentPrompt;
use crate::prompt_template::ToolNames;
use crate::protocol::SandboxPolicy;
use crate::tools::spec::APPLY_PATCH_TOOL_NAME;
use crate::tools::spec::EDIT_FILE_TOOL_NAME;
use crate::tools::spec::EXIT_PLAN_MODE_TOOL_NAME;
use crate::tools::spec::GLOB_TOOL_NAME;
use crate::tools::spec::GREP_FILES_TOOL_NAME;
use crate::tools::spec::LIST_DIR_TOOL_NAME;
use crate::tools::spec::READ_FILE_TOOL_NAME;
use crate::tools::spec::TASK_TOOL_NAME;
use crate::tools::spec::ToolsConfig;
use crate::tools::spec::WRITE_FILE_TOOL_NAME;

/// Tools that are globally banned for all subagents.
/// These tools are not available to any subagent regardless of configuration.
const SUBAGENT_BANNED_TOOLS: &[&str] = &[
    EXIT_PLAN_MODE_TOOL_NAME, // Only the main agent can exit plan mode
    TASK_TOOL_NAME,           // Prevent nested task spawning
];

pub struct AgentTypeManager {
    agents: HashMap<String, AgentTypeConfig>,
}

pub trait DeveloperPrompt {
    fn render<T: ToolNames>(&self, tools: &T) -> String;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum DeveloperPromptKind {
    Explore,
    Plan,
    General,
    Custom { prompt: String }, // We support raw prompt for now, not templated
}

impl DeveloperPrompt for DeveloperPromptKind {
    fn render<T: ToolNames>(&self, tools: &T) -> String {
        match self {
            DeveloperPromptKind::Explore => ExploreAgentPrompt { tools }.to_string(),
            DeveloperPromptKind::Plan => PlanSubagentPrompt { tools }.to_string(),
            DeveloperPromptKind::General => GeneralMainPrompt { tools }.to_string(),
            DeveloperPromptKind::Custom { prompt } => prompt.clone(),
        }
    }
}

impl AgentTypeManager {
    /// Create registry with built-in default agent types.
    pub fn with_defaults() -> Self {
        let mut agents = HashMap::new();

        // General-purpose sub-agent for complex multi-step tasks
        agents.insert(
            "general".to_string(),
            AgentTypeConfig {
                name: "general".to_string(),
                description:
                    r#"General-purpose agent for researching complex questions, searching for code, and executing multi-step tasks. When you are searching for a keyword or file and are not confident that you will find the right match in the first few tries use this agent to perform the search for you."#
                        .to_string()
                ,
                model_tier: ModelTier::Default,
                developer_instructions: None,
                tools: None,
                max_steps: Some(50),
                hidden: false,
                fork_context: false,
                disallowed_tools: None,
                is_built_in: true,
                read_only: false,
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
                model_tier: ModelTier::Small, // Use small/fast model for exploration
                developer_instructions: Some(
                    DeveloperPromptKind::Explore
                ),
                tools: None, // All tools (wildcard) - use disallowed_tools for restrictions
                max_steps: Some(30),
                hidden: false,
                fork_context: false,
                disallowed_tools: Some(vec![
                    EDIT_FILE_TOOL_NAME.to_string(),
                    WRITE_FILE_TOOL_NAME.to_string(),
                    APPLY_PATCH_TOOL_NAME.to_string(),
                ]),
                is_built_in: true,
                read_only: true,
            },
        );

        // Planning agent - for creating implementation plans
        agents.insert(
            "plan".to_string(),
            AgentTypeConfig {
                name: "plan".to_string(),
                description:
                    r#""Software architect agent for designing implementation plans. Use this when you need to plan the implementation strategy for a task. Returns step-by-step plans, identifies critical files, and considers architectural trade-offs."#.to_string()
                ,
                model_tier: ModelTier::Inherit,
                developer_instructions: Some(
                    DeveloperPromptKind::Plan
                ),
                tools: Some(vec![
                    READ_FILE_TOOL_NAME.to_string(),
                    GREP_FILES_TOOL_NAME.to_string(),
                    LIST_DIR_TOOL_NAME.to_string(),
                    GLOB_TOOL_NAME.to_string(),
                ]),
                max_steps: Some(40),
                hidden: false,
                fork_context: false,
                disallowed_tools: None, // Explicit tools list is already restrictive
                is_built_in: true,
                read_only: true,
            },
        );

        Self { agents }
    }

    /// Merge user-defined agent types from config.
    ///
    /// User-defined agents override built-in agents with the same key.
    // TODO: Wire up to config loading for user-defined agent types in config.toml
    #[allow(dead_code)]
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
    // TODO: Useful for debugging/introspection
    #[allow(dead_code)]
    pub fn all_names(&self) -> Vec<&str> {
        self.agents.keys().map(String::as_str).collect()
    }
}

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

    /// Which model tier this agent uses (Default or Small).
    /// Default = use config.model, Small = use config.small_model.
    #[serde(default)]
    pub model_tier: ModelTier,

    /// developer_instructions additions for this agent type.
    /// This is appended to the parent session's instructions.
    pub developer_instructions: Option<DeveloperPromptKind>,

    /// Tools configuration for this agent type.
    /// Maps tool names to enabled/disabled state.
    /// If None, inherits parent's tools minus task.
    pub tools: Option<Vec<String>>,

    /// Maximum steps (model turns) before forcing completion.
    pub max_steps: Option<u32>,

    /// Whether this agent type is hidden from LLM selection.
    /// Hidden agents can still be used explicitly but won't appear in descriptions.
    #[serde(default)]
    pub hidden: bool,

    /// Whether fork context or not.
    /// If true, the agent will inherit the context from the parent agent.
    #[serde(default)]
    pub fork_context: bool,

    /// Tools to block for this agent (negative list).
    /// Applied after global blocks and before allowed_tools filter.
    /// Use this when you want "all tools except these".
    #[serde(default)]
    pub disallowed_tools: Option<Vec<String>>,

    /// Whether this is a built-in agent (affects filtering rules).
    /// Built-in agents have fewer restrictions than user-defined agents.
    #[serde(default)]
    pub is_built_in: bool,

    /// Whether to force a read-only sandbox policy.
    #[serde(default)]
    pub read_only: bool,
}

impl AgentTypeConfig {
    pub fn apply_to_config(
        self,
        config: &mut Config,
        tools_config: &mut ToolsConfig,
    ) -> Result<(), String> {
        // Apply developer instructions
        if let Some(developer_instructions) = self.developer_instructions {
            let prompt = developer_instructions.render(tools_config);

            // Check feature flag to determine where to apply instructions
            if config.features.enabled(Feature::UniformBaseInstructions) {
                // Replace base_instructions when feature enabled
                config.base_instructions = Some(prompt);
            } else {
                // Original: append to developer_instructions
                config
                    .developer_instructions
                    .get_or_insert_with(String::new)
                    .push_str(&format!("\n\n{prompt}"));
            }
        }

        // Apply read-only sandbox policy if configured
        if self.read_only {
            config
                .sandbox_policy
                .set(SandboxPolicy::new_read_only_policy())
                .map_err(|err| format!("sandbox_policy is invalid: {err}"))?;
        }

        // Apply tool filtering for subagents
        // 1. Add globally banned tools
        for tool in SUBAGENT_BANNED_TOOLS {
            tools_config.disabled_tools.push((*tool).to_string());
        }

        // 2. Add agent-specific disallowed tools
        if let Some(disallowed) = &self.disallowed_tools {
            tools_config.disabled_tools.extend(disallowed.clone());
        }

        Ok(())
    }
}
