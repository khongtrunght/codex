//! Task tool handler for spawning specialized sub-agents.
//!
//! The Task tool allows the main agent to spawn sub-agents with specific capabilities
//! for handling complex, multi-step tasks autonomously.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing::warn;

use crate::agent_types::AgentTypeConfig;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::codex_delegate::run_codex_conversation_one_shot;
use crate::config::Config;
use crate::function_tool::FunctionCallError;
use crate::protocol::EventMsg;
use crate::protocol::SubAgentBeginEvent;
use crate::protocol::SubAgentEndEvent;
use crate::protocol::SubAgentTokenUsage;
use crate::protocol::SubAgentToolSummary;
use crate::subagent_prompt::DEFAULT_SUBAGENT_PROMPT;
use crate::subagent_prompt::augment_system_prompt;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::user_input::UserInput;

/// Handler for the `task` tool that spawns sub-agents.
pub struct TaskHandler;

#[derive(Debug, Deserialize)]
struct TaskParams {
    /// Short description of the task (3-5 words).
    description: String,
    /// Detailed prompt for the sub-agent.
    prompt: String,
    /// The type of specialized agent to use.
    subagent_type: String,
    /// Optional session ID to resume from a previous task.
    #[serde(default)]
    resume: Option<String>,
    /// Optional model override (e.g., "sonnet", "opus", "haiku").
    #[serde(default)]
    model: Option<String>,
}

#[async_trait]
impl ToolHandler for TaskHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "task handler received unsupported payload".to_string(),
                ));
            }
        };

        let params: TaskParams = serde_json::from_str(&arguments).map_err(|e| {
            FunctionCallError::RespondToModel(format!("Invalid task parameters: {e}"))
        })?;

        // Validate agent type exists
        let agent_config = session
            .services
            .agent_type_registry
            .get(&params.subagent_type)
            .cloned()
            .ok_or_else(|| {
                let available = session.services.agent_type_registry.agent_configs();
                let available_text = crate::tools::spec::render_agent_descriptions(&available);
                FunctionCallError::RespondToModel(format!(
                    "Unknown agent type: '{}'. Available types:\n{}",
                    params.subagent_type, available_text
                ))
            })?;

        let start_time = Instant::now();
        let resumed = params.resume.is_some();

        // Generate or use provided session ID
        let task_session_id = params
            .resume
            .clone()
            .unwrap_or_else(|| format!("task-{}", uuid::Uuid::new_v4()));

        info!(
            agent_type = %params.subagent_type,
            session_id = %task_session_id,
            description = %params.description,
            "Starting sub-agent task"
        );

        // Create subagent rollout file and register for event routing
        {
            let recorder = session.services.rollout.lock().await;
            if let Some(rec) = recorder.as_ref() {
                match rec
                    .create_subagent_file(
                        &task_session_id,
                        session.source_session_id().map(std::string::String::as_str),
                        &params.subagent_type,
                        &params.description,
                    )
                    .await
                {
                    Ok(_path) => {
                        info!(
                            session_id = %task_session_id,
                            "Created subagent rollout file"
                        );
                    }
                    Err(e) => {
                        warn!(
                            session_id = %task_session_id,
                            error = %e,
                            "Failed to create subagent rollout file"
                        );
                    }
                }
            }
        }
        // Register subagent for event routing
        session.register_subagent(&task_session_id).await;

        // Emit SubAgentBegin event
        session
            .send_event(
                turn.as_ref(),
                EventMsg::SubAgentBegin(SubAgentBeginEvent {
                    call_id: call_id.clone(),
                    agent_type: params.subagent_type.clone(),
                    description: params.description.clone(),
                    prompt: Some(params.prompt.clone()),
                    session_id: task_session_id.clone(),
                    resumed,
                }),
            )
            .await;

        // Get the base config from session
        let base_config = session.get_config().await;

        // Build config for sub-agent
        let sub_config = build_subagent_config(&base_config, &agent_config, &params);

        // Create a new cancellation token for this sub-agent
        let cancel_token = CancellationToken::new();

        // Track tool calls for progress
        let tool_summary = Arc::new(Mutex::new(Vec::<SubAgentToolSummary>::new()));
        // Track token usage
        let token_usage = Arc::new(Mutex::new(SubAgentTokenUsage::default()));

        // Run sub-agent
        let result = run_task_subagent(
            Arc::clone(&session),
            Arc::clone(&turn),
            sub_config,
            params.prompt.clone(),
            cancel_token,
            call_id.clone(),
            task_session_id.clone(),
            Arc::clone(&tool_summary),
            Arc::clone(&token_usage),
        )
        .await;

        let duration_ms = start_time.elapsed().as_millis() as u64;
        let final_summary = tool_summary.lock().await.clone();
        let final_token_usage = token_usage.lock().await.clone();

        // Emit SubAgentEnd event
        let (success, output) = match &result {
            Ok(text) => (true, text.clone()),
            Err(e) => (false, e.clone()),
        };

        session
            .send_event(
                turn.as_ref(),
                EventMsg::SubAgentEnd(SubAgentEndEvent {
                    call_id: call_id.clone(),
                    session_id: task_session_id.clone(),
                    success,
                    output: output.clone(),
                    duration_ms,
                    tool_summary: final_summary,
                    token_usage: if final_token_usage.total_tokens > 0 {
                        Some(final_token_usage)
                    } else {
                        None
                    },
                }),
            )
            .await;

        // Close subagent rollout file
        {
            let recorder = session.services.rollout.lock().await;
            if let Some(rec) = recorder.as_ref()
                && let Err(e) = rec.close_subagent_file(&task_session_id).await
            {
                warn!(
                    session_id = %task_session_id,
                    error = %e,
                    "Failed to close subagent rollout file"
                );
            }
        }

        info!(
            agent_type = %params.subagent_type,
            session_id = %task_session_id,
            success = %success,
            duration_ms = %duration_ms,
            "Sub-agent task completed"
        );

        // Format output with metadata
        let formatted_output =
            format!("{output}\n\n<task_metadata>\nsession_id: {task_session_id}\n</task_metadata>");

        Ok(ToolOutput::Function {
            content: formatted_output,
            content_items: None,
            success: Some(success),
        })
    }
}

/// Build configuration for a sub-agent with augmented system prompt.
///
/// Unlike the parent session, sub-agents receive a fresh system prompt that includes:
/// 1. Agent-specific instructions (from AgentTypeConfig.system_prompt or default)
/// 2. Standard agent notes (absolute paths, no emojis)
///
/// Environment context (cwd, sandbox, etc.) is injected separately via EnvironmentContext.
/// The parent's user_instructions and developer_instructions are NOT inherited.
fn build_subagent_config(
    parent_config: &Config,
    agent_config: &AgentTypeConfig,
    params: &TaskParams,
) -> Config {
    let mut config = parent_config.clone();

    // Apply model override if specified in agent config or params
    if let Some(model_str) = params.model.as_ref().or(agent_config.model.as_ref()) {
        // Parse "provider:model" format or just model name
        if let Some((_provider, model)) = model_str.split_once(':') {
            // Full format with provider - just use the model part for now
            config.model = Some(model.to_string());
        } else {
            // Just model name - map common aliases
            let mapped_model = match model_str.as_str() {
                "sonnet" => "claude-sonnet-4-5-20250929",
                "opus" => "claude-opus-4-5-20251101",
                "haiku" => "claude-haiku-4-5-20251001",
                other => other,
            };
            config.model = Some(mapped_model.to_string());
        }
    }

    // Build augmented system prompt for sub-agent
    // Use agent's custom prompt or fall back to default
    let agent_prompt = agent_config
        .system_prompt
        .as_deref()
        .unwrap_or(DEFAULT_SUBAGENT_PROMPT);

    // Augment with standard notes (absolute paths, no emojis, etc.)
    let augmented = augment_system_prompt(agent_prompt);

    // Set as base_instructions (system prompt), not user_instructions
    // This REPLACES the parent's instructions, following Claude Code's approach
    config.base_instructions = Some(augmented);

    // Clear parent's instructions - sub-agents should not inherit these
    config.user_instructions = None;
    config.developer_instructions = None;

    // Set up tool filtering for this sub-agent.
    // This blocks the task tool (preventing infinite nesting) and applies
    // any agent-specific tool restrictions.
    config.subagent_tool_filter = Some(if let Some(tools) = &agent_config.tools {
        crate::tools::filtering::SubAgentToolFilter::with_allowed_tools(tools.clone())
    } else {
        crate::tools::filtering::SubAgentToolFilter::new()
    });

    config
}

#[allow(clippy::too_many_arguments)]
async fn run_task_subagent(
    parent_session: Arc<Session>,
    parent_turn: Arc<TurnContext>,
    config: Config,
    prompt: String,
    cancel_token: CancellationToken,
    _call_id: String,
    session_id: String,
    tool_summary: Arc<Mutex<Vec<SubAgentToolSummary>>>,
    token_usage: Arc<Mutex<SubAgentTokenUsage>>,
) -> Result<String, String> {
    // Build user input
    let input = vec![UserInput::Text { text: prompt }];

    // Spawn sub-agent using the existing infrastructure
    // Pass the session_id so the sub-agent Session knows its own identity
    let codex = run_codex_conversation_one_shot(
        config,
        Arc::clone(&parent_session.services.auth_manager),
        Arc::clone(&parent_session.services.models_manager),
        input,
        Arc::clone(&parent_session),
        Arc::clone(&parent_turn),
        cancel_token.clone(),
        None, // For now, don't support resume - would need rollout path lookup
        Some(session_id.clone()), // Sub-agent's own session ID
    )
    .await
    .map_err(|e| format!("Failed to spawn sub-agent: {e}"))?;

    // Process events from the sub-agent
    let mut final_output = String::new();

    loop {
        tokio::select! {
            biased;

            _ = cancel_token.cancelled() => {
                return Err("Task cancelled".to_string());
            }

            event = codex.next_event() => {
                let event = match event {
                    Ok(event) => event,
                    Err(_) => break,
                };

                // Forward events to parent session for TUI visibility
                // Events are tagged with source_session_id so TUI can route them
                // to the appropriate SubAgentCell
                let should_forward = !matches!(
                    &event.msg,
                    EventMsg::SessionConfigured(_)
                    | EventMsg::TaskComplete(_)
                    | EventMsg::TurnAborted(_)
                    | EventMsg::SubAgentBegin(_)
                    | EventMsg::SubAgentProgress(_)
                    | EventMsg::SubAgentEnd(_)
                );

                if should_forward {
                    // For nested sub-agents, parent_session_id is the parent's own session ID
                    // For first-level sub-agents spawned by root, parent's source_session_id() is None
                    parent_session
                        .send_event_with_source(
                            parent_turn.as_ref(),
                            event.msg.clone(),
                            Some(session_id.clone()),
                            parent_session.source_session_id().cloned(),
                        )
                        .await;
                }

                match &event.msg {
                    // Track command executions for final summary
                    EventMsg::ExecCommandBegin(cmd) => {
                        let mut summary = tool_summary.lock().await;
                        summary.push(SubAgentToolSummary {
                            tool_name: "shell".to_string(),
                            title: Some(cmd.command.join(" ")),
                            status: "running".to_string(),
                        });
                    }

                    EventMsg::ExecCommandEnd(cmd) => {
                        let mut summary = tool_summary.lock().await;
                        if let Some(last) = summary.last_mut() {
                            last.status = if cmd.exit_code == 0 {
                                "completed".to_string()
                            } else {
                                "error".to_string()
                            };
                        }
                    }

                    // Track patch applications
                    EventMsg::PatchApplyBegin(_) => {
                        let mut summary = tool_summary.lock().await;
                        summary.push(SubAgentToolSummary {
                            tool_name: "apply_patch".to_string(),
                            title: Some("Applying code changes".to_string()),
                            status: "running".to_string(),
                        });
                    }

                    EventMsg::PatchApplyEnd(patch) => {
                        let mut summary = tool_summary.lock().await;
                        if let Some(last) = summary.last_mut() {
                            last.status = if patch.success {
                                "completed".to_string()
                            } else {
                                "error".to_string()
                            };
                        }
                    }

                    // Track MCP tool calls
                    EventMsg::McpToolCallBegin(mcp) => {
                        let mut summary = tool_summary.lock().await;
                        summary.push(SubAgentToolSummary {
                            tool_name: mcp.invocation.tool.clone(),
                            title: Some(format!("MCP: {}", mcp.invocation.tool)),
                            status: "running".to_string(),
                        });
                    }

                    EventMsg::McpToolCallEnd(_) => {
                        let mut summary = tool_summary.lock().await;
                        if let Some(last) = summary.last_mut() {
                            last.status = "completed".to_string();
                        }
                    }

                    // Track token usage
                    EventMsg::TokenCount(tc) => {
                        if let Some(info) = &tc.info {
                            let mut usage = token_usage.lock().await;
                            // Accumulate the last turn's usage into our totals
                            let last = &info.last_token_usage;
                            // Use absolute values since tokens are i64 in protocol
                            usage.input_tokens = usage
                                .input_tokens
                                .saturating_add(last.input_tokens.max(0) as u64);
                            usage.output_tokens = usage
                                .output_tokens
                                .saturating_add(last.output_tokens.max(0) as u64);
                            usage.total_tokens = usage.input_tokens + usage.output_tokens;
                        }
                    }

                    // Capture final agent response
                    EventMsg::AgentMessage(msg) => {
                        final_output = msg.message.clone();
                    }

                    // Task complete
                    EventMsg::TaskComplete(_) => {
                        break;
                    }

                    EventMsg::TurnAborted(aborted) => {
                        warn!(reason = ?aborted.reason, "Sub-agent turn aborted");
                        if final_output.is_empty() {
                            final_output = format!("Task aborted: {:?}", aborted.reason);
                        }
                        break;
                    }

                    _ => {}
                }
            }
        }
    }

    if final_output.is_empty() {
        final_output = "Sub-agent completed without producing output.".to_string();
    }

    Ok(final_output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_config;
    use crate::subagent_prompt::SUBAGENT_NOTES;

    #[test]
    fn test_subagent_token_usage_default() {
        let usage = SubAgentTokenUsage::default();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.total_tokens, 0);
    }

    #[test]
    fn test_subagent_token_usage_accumulation() {
        let mut usage = SubAgentTokenUsage::default();

        // Simulate accumulating tokens like we do in the event loop
        let input1: i64 = 100;
        let output1: i64 = 50;
        usage.input_tokens = usage.input_tokens.saturating_add(input1.max(0) as u64);
        usage.output_tokens = usage.output_tokens.saturating_add(output1.max(0) as u64);
        usage.total_tokens = usage.input_tokens + usage.output_tokens;

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, 150);

        // Accumulate more
        let input2: i64 = 200;
        let output2: i64 = 75;
        usage.input_tokens = usage.input_tokens.saturating_add(input2.max(0) as u64);
        usage.output_tokens = usage.output_tokens.saturating_add(output2.max(0) as u64);
        usage.total_tokens = usage.input_tokens + usage.output_tokens;

        assert_eq!(usage.input_tokens, 300);
        assert_eq!(usage.output_tokens, 125);
        assert_eq!(usage.total_tokens, 425);
    }

    #[test]
    fn test_subagent_token_usage_handles_negative_gracefully() {
        let mut usage = SubAgentTokenUsage::default();

        // Negative values should be treated as 0
        let input: i64 = -100;
        let output: i64 = -50;
        usage.input_tokens = usage.input_tokens.saturating_add(input.max(0) as u64);
        usage.output_tokens = usage.output_tokens.saturating_add(output.max(0) as u64);
        usage.total_tokens = usage.input_tokens + usage.output_tokens;

        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.total_tokens, 0);
    }

    #[test]
    fn test_parse_task_params() {
        let json = r#"{
            "description": "Explore codebase",
            "prompt": "Find all rust files",
            "subagent_type": "explore"
        }"#;

        let params: TaskParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.description, "Explore codebase");
        assert_eq!(params.prompt, "Find all rust files");
        assert_eq!(params.subagent_type, "explore");
        assert!(params.resume.is_none());
        assert!(params.model.is_none());
    }

    #[test]
    fn test_parse_task_params_with_optional_fields() {
        let json = r#"{
            "description": "Test task",
            "prompt": "Do something",
            "subagent_type": "general",
            "resume": "task-123",
            "model": "sonnet"
        }"#;

        let params: TaskParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.resume, Some("task-123".to_string()));
        assert_eq!(params.model, Some("sonnet".to_string()));
    }

    #[test]
    fn test_build_subagent_config_sets_base_instructions() {
        let parent_config = test_config();
        let agent_config = AgentTypeConfig {
            name: "test".to_string(),
            system_prompt: Some("You are a test agent.".to_string()),
            ..Default::default()
        };
        let params = TaskParams {
            description: "Test".to_string(),
            prompt: "Do something".to_string(),
            subagent_type: "test".to_string(),
            resume: None,
            model: None,
        };

        let sub_config = build_subagent_config(&parent_config, &agent_config, &params);

        // Should have base_instructions set with augmented prompt
        assert!(sub_config.base_instructions.is_some());
        let base = sub_config.base_instructions.unwrap();
        assert!(base.contains("You are a test agent."));
        assert!(base.contains(SUBAGENT_NOTES));
    }

    #[test]
    fn test_build_subagent_config_clears_parent_instructions() {
        let mut parent_config = test_config();
        parent_config.user_instructions = Some("Parent user instructions".to_string());
        parent_config.developer_instructions = Some("Parent dev instructions".to_string());

        let agent_config = AgentTypeConfig {
            name: "test".to_string(),
            system_prompt: Some("Agent prompt.".to_string()),
            ..Default::default()
        };
        let params = TaskParams {
            description: "Test".to_string(),
            prompt: "Do something".to_string(),
            subagent_type: "test".to_string(),
            resume: None,
            model: None,
        };

        let sub_config = build_subagent_config(&parent_config, &agent_config, &params);

        // Parent's instructions should NOT be inherited
        assert!(sub_config.user_instructions.is_none());
        assert!(sub_config.developer_instructions.is_none());
    }

    #[test]
    fn test_build_subagent_config_uses_default_prompt_when_none() {
        let parent_config = test_config();
        let agent_config = AgentTypeConfig {
            name: "general".to_string(),
            system_prompt: None, // No custom prompt
            ..Default::default()
        };
        let params = TaskParams {
            description: "Test".to_string(),
            prompt: "Do something".to_string(),
            subagent_type: "general".to_string(),
            resume: None,
            model: None,
        };

        let sub_config = build_subagent_config(&parent_config, &agent_config, &params);

        // Should use default fallback prompt
        assert!(sub_config.base_instructions.is_some());
        let base = sub_config.base_instructions.unwrap();
        assert!(base.contains(DEFAULT_SUBAGENT_PROMPT));
        assert!(base.contains(SUBAGENT_NOTES));
    }

    #[test]
    fn test_build_subagent_config_model_resolution() {
        let parent_config = test_config();
        let agent_config = AgentTypeConfig::default();

        // Test alias resolution
        let params = TaskParams {
            description: "Test".to_string(),
            prompt: "Do something".to_string(),
            subagent_type: "test".to_string(),
            resume: None,
            model: Some("sonnet".to_string()),
        };

        let sub_config = build_subagent_config(&parent_config, &agent_config, &params);
        assert_eq!(
            sub_config.model,
            Some("claude-sonnet-4-5-20250929".to_string())
        );

        // Test provider:model format
        let params2 = TaskParams {
            description: "Test".to_string(),
            prompt: "Do something".to_string(),
            subagent_type: "test".to_string(),
            resume: None,
            model: Some("anthropic:claude-custom".to_string()),
        };

        let sub_config2 = build_subagent_config(&parent_config, &agent_config, &params2);
        assert_eq!(sub_config2.model, Some("claude-custom".to_string()));
    }
}
