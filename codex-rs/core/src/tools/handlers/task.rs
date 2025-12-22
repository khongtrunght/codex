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
use tracing::{info, warn};

use crate::agent_types::AgentTypeConfig;
use crate::codex::{Session, TurnContext};
use crate::codex_delegate::run_codex_conversation_one_shot;
use crate::config::Config;
use crate::function_tool::FunctionCallError;
use crate::protocol::{
    EventMsg, SubAgentBeginEvent, SubAgentEndEvent, SubAgentProgressEvent, SubAgentToolSummary,
};
use crate::tools::context::{ToolInvocation, ToolOutput, ToolPayload};
use crate::tools::registry::{ToolHandler, ToolKind};
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
                let available = session
                    .services
                    .agent_type_registry
                    .generate_agent_descriptions();
                FunctionCallError::RespondToModel(format!(
                    "Unknown agent type: '{}'. Available types:\n{}",
                    params.subagent_type, available
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

        // Emit SubAgentBegin event
        session
            .send_event(
                turn.as_ref(),
                EventMsg::SubAgentBegin(SubAgentBeginEvent {
                    call_id: call_id.clone(),
                    agent_type: params.subagent_type.clone(),
                    description: params.description.clone(),
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
        )
        .await;

        let duration_ms = start_time.elapsed().as_millis() as u64;
        let final_summary = tool_summary.lock().await.clone();

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
                }),
            )
            .await;

        info!(
            agent_type = %params.subagent_type,
            session_id = %task_session_id,
            success = %success,
            duration_ms = %duration_ms,
            "Sub-agent task completed"
        );

        // Format output with metadata
        let formatted_output = format!(
            "{output}\n\n<task_metadata>\nsession_id: {task_session_id}\n</task_metadata>"
        );

        Ok(ToolOutput::Function {
            content: formatted_output,
            content_items: None,
            success: Some(success),
        })
    }
}

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
                "sonnet" => "claude-sonnet-4-20250514",
                "opus" => "claude-opus-4-20250514",
                "haiku" => "claude-3-5-haiku-20241022",
                other => other,
            };
            config.model = Some(mapped_model.to_string());
        }
    }

    // Apply system prompt addition
    if let Some(system_prompt) = &agent_config.system_prompt {
        let base = config.user_instructions.clone().unwrap_or_default();
        config.user_instructions = Some(format!("{base}\n\n{system_prompt}"));
    }

    // Set up tool filtering for this sub-agent.
    // This blocks the task tool (preventing infinite nesting) and applies
    // any agent-specific tool restrictions.
    config.subagent_tool_filter = Some(
        crate::tools::filtering::SubAgentToolFilter::with_agent_tools(
            agent_config.tools.clone().unwrap_or_default(),
        ),
    );

    config
}

async fn run_task_subagent(
    parent_session: Arc<Session>,
    parent_turn: Arc<TurnContext>,
    config: Config,
    prompt: String,
    cancel_token: CancellationToken,
    call_id: String,
    session_id: String,
    tool_summary: Arc<Mutex<Vec<SubAgentToolSummary>>>,
) -> Result<String, String> {
    // Build user input
    let input = vec![UserInput::Text { text: prompt }];

    // Spawn sub-agent using the existing infrastructure
    let codex = run_codex_conversation_one_shot(
        config,
        Arc::clone(&parent_session.services.auth_manager),
        Arc::clone(&parent_session.services.models_manager),
        input,
        Arc::clone(&parent_session),
        Arc::clone(&parent_turn),
        cancel_token.clone(),
        None, // For now, don't support resume - would need rollout path lookup
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

                match &event.msg {
                    // Track command executions for progress
                    EventMsg::ExecCommandBegin(cmd) => {
                        let mut summary = tool_summary.lock().await;
                        summary.push(SubAgentToolSummary {
                            tool_name: "shell".to_string(),
                            title: Some(cmd.command.join(" ")),
                            status: "running".to_string(),
                        });

                        // Emit progress event
                        parent_session
                            .send_event(
                                parent_turn.as_ref(),
                                EventMsg::SubAgentProgress(SubAgentProgressEvent {
                                    call_id: call_id.clone(),
                                    session_id: session_id.clone(),
                                    completed_tools: summary.clone(),
                                    status: Some(format!("Running: {}", cmd.command.join(" "))),
                                }),
                            )
                            .await;
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
}
