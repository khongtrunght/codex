//! Agent spawn context for building sub-agent configurations.
//!
//! This module encapsulates the complexity of resolving model-dependent
//! configuration when spawning sub-agents via the Task tool.

use codex_protocol::model_tier::ModelTier;

use crate::codex::TurnContext;
use crate::config::Config;
use crate::function_tool::FunctionCallError;
use crate::models_manager::manager::ModelsManager;
use crate::tools::spec::ToolsConfig;
use crate::tools::spec::ToolsConfigParams;

use super::manager::AgentTypeConfig;

/// Context for spawning a sub-agent with fully resolved configuration.
///
/// This struct encapsulates all the complexity of:
/// - Resolving model from tier (default/small/inherit)
/// - Building model-aware ToolsConfig
/// - Rendering developer instructions with correct tool names
#[derive(Debug)]
pub struct AgentSpawnContext {
    /// The fully configured Config ready for agent spawn.
    pub config: Config,
    /// The resolved ToolsConfig (kept for potential future use).
    pub tools_config: ToolsConfig,
}

impl AgentSpawnContext {
    /// Build spawn context from agent type configuration.
    ///
    /// # Arguments
    /// * `models_manager` - For fetching model info
    /// * `turn` - Current turn context (provides base config)
    /// * `agent_type` - The agent type configuration to apply
    /// * `model_override` - Optional model tier override from Task tool params
    ///
    /// # Returns
    /// A fully configured `AgentSpawnContext` ready for spawning.
    pub async fn build(
        models_manager: &ModelsManager,
        turn: &TurnContext,
        agent_type: &AgentTypeConfig,
        model_override: Option<&str>,
    ) -> Result<Self, FunctionCallError> {
        let mut config = build_base_config(turn)?;

        // Resolve model from tier
        let resolved_model = resolve_model_for_agent(model_override, agent_type, &config)?;
        config.model = resolved_model.clone();

        // Build tools config with model info
        let tools_config = build_tools_config(models_manager, &config).await;

        // Render developer instructions with model-aware tool names
        config.developer_instructions = agent_type.render_developer_instructions(&tools_config);

        Ok(Self {
            config,
            tools_config,
        })
    }
}

/// Build the base agent config from turn context.
fn build_base_config(turn: &TurnContext) -> Result<Config, FunctionCallError> {
    let base_config = turn.client.config();
    let mut config = (*base_config).clone();
    config.model = Some(turn.client.get_model());
    config.model_provider = turn.client.get_provider();
    config.model_reasoning_effort = turn.client.get_reasoning_effort();
    config.model_reasoning_summary = turn.client.get_reasoning_summary();
    config.developer_instructions = turn.developer_instructions.clone();
    config.base_instructions = turn.base_instructions.clone();
    config.compact_prompt = turn.compact_prompt.clone();
    config.user_instructions = turn.user_instructions.clone();
    config.shell_environment_policy = turn.shell_environment_policy.clone();
    config.codex_linux_sandbox_exe = turn.codex_linux_sandbox_exe.clone();
    config.cwd = turn.cwd.clone();
    config
        .approval_policy
        .set(turn.approval_policy)
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("approval_policy is invalid: {err}"))
        })?;
    config
        .sandbox_policy
        .set(turn.sandbox_policy.clone())
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("sandbox_policy is invalid: {err}"))
        })?;
    Ok(config)
}

/// Build ToolsConfig from model info.
async fn build_tools_config(models_manager: &ModelsManager, config: &Config) -> ToolsConfig {
    if let Some(model_slug) = &config.model {
        let model_info = models_manager.get_model_info(model_slug, config).await;
        ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            features: &config.features,
            web_search_mode: config.web_search_mode,
        })
    } else {
        ToolsConfig::default()
    }
}

/// Resolve model from tier override or agent config.
fn resolve_model_for_agent(
    model_override: Option<&str>,
    agent_config: &AgentTypeConfig,
    parent_config: &Config,
) -> Result<Option<String>, FunctionCallError> {
    let small_model = parent_config
        .small_model
        .clone()
        .or_else(|| parent_config.model.clone());

    if let Some(model_str) = model_override {
        // Explicit model override from Task tool params
        if let Ok(tier) = model_str.parse::<ModelTier>() {
            match tier {
                ModelTier::Default => Ok(parent_config.model.clone()),
                ModelTier::Small => Ok(small_model),
                ModelTier::Inherit => Ok(parent_config.model.clone()),
            }
        } else {
            Err(FunctionCallError::RespondToModel(format!(
                "Invalid model tier override: '{model_str}'. Must be 'default', 'small', or 'inherit'."
            )))
        }
    } else {
        // No override - use agent's configured tier
        match agent_config.model_tier {
            ModelTier::Default => Ok(parent_config.model.clone()),
            ModelTier::Small => Ok(small_model),
            ModelTier::Inherit => Ok(parent_config.model.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_config;
    use codex_protocol::model_tier::ModelTier;

    fn make_config(model: Option<&str>, small_model: Option<&str>) -> Config {
        let mut config = test_config();
        config.model = model.map(String::from);
        config.small_model = small_model.map(String::from);
        config
    }

    fn make_agent_config(tier: ModelTier) -> AgentTypeConfig {
        AgentTypeConfig {
            model_tier: tier,
            ..Default::default()
        }
    }

    #[test]
    fn resolve_model_uses_default_tier() {
        let config = make_config(Some("gpt-4"), Some("gpt-3.5"));
        let agent = make_agent_config(ModelTier::Default);

        let result = resolve_model_for_agent(None, &agent, &config).unwrap();
        assert_eq!(result, Some("gpt-4".to_string()));
    }

    #[test]
    fn resolve_model_uses_small_tier() {
        let config = make_config(Some("gpt-4"), Some("gpt-3.5"));
        let agent = make_agent_config(ModelTier::Small);

        let result = resolve_model_for_agent(None, &agent, &config).unwrap();
        assert_eq!(result, Some("gpt-3.5".to_string()));
    }

    #[test]
    fn resolve_model_small_fallback_to_default() {
        let config = make_config(Some("gpt-4"), None);
        let agent = make_agent_config(ModelTier::Small);

        let result = resolve_model_for_agent(None, &agent, &config).unwrap();
        assert_eq!(result, Some("gpt-4".to_string()));
    }

    #[test]
    fn resolve_model_override_takes_precedence() {
        let config = make_config(Some("gpt-4"), Some("gpt-3.5"));
        let agent = make_agent_config(ModelTier::Default);

        let result = resolve_model_for_agent(Some("small"), &agent, &config).unwrap();
        assert_eq!(result, Some("gpt-3.5".to_string()));
    }

    #[test]
    fn resolve_model_invalid_override_errors() {
        let config = make_config(Some("gpt-4"), None);
        let agent = make_agent_config(ModelTier::Default);

        let result = resolve_model_for_agent(Some("invalid-tier"), &agent, &config);
        assert!(result.is_err());
    }
}
