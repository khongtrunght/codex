use async_trait::async_trait;
use codex_protocol::model_tier::ModelTier;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::SubAgentBeginEvent;
use codex_protocol::protocol::SubAgentCompleteEvent;
use codex_protocol::protocol::SubAgentEndEvent;
use serde::Deserialize;
use serde::Serialize;

use crate::agent::status::is_final;
use crate::error::CodexErr;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::collab::build_agent_spawn_config;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::ToolsConfig;
use crate::tools::spec::ToolsConfigParams;

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
    /// Optional model tier override ("default", "small", "inherit") or direct model slug.
    #[serde(default)]
    model: Option<ModelTier>,
}

#[derive(Debug, Serialize)]
struct TaskResult {
    agent_id: String,
    status: AgentStatus,
}

#[async_trait]
impl ToolHandler for TaskHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
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

        let params: TaskParams = parse_arguments(&arguments)?;

        // Validate agent type exists
        let agent_type = session
            .services
            .agent_type_manager
            .get(&params.subagent_type)
            .cloned()
            .ok_or_else(|| {
                let available = session.services.agent_type_manager.agent_configs();
                let available_text = crate::tools::spec::render_agent_descriptions(&available);
                FunctionCallError::RespondToModel(format!(
                    "Unknown agent type: '{}'. Available types:\n{}",
                    params.subagent_type, available_text
                ))
            })?;

        // Emit SubAgentSpawnBegin event
        session
            .send_event(
                &turn,
                SubAgentBeginEvent {
                    call_id: call_id.clone(),
                    agent_type: params.subagent_type.clone(),
                    description: params.description.clone(),
                    prompt: params.prompt.clone(),
                    sender_thread_id: session.conversation_id,
                    resumed: params.resume.is_some(),
                }
                .into(),
            )
            .await;

        let mut config =
            build_agent_spawn_config(&session.get_base_instructions().await, turn.as_ref())?;
        // model with get from the args first, then from the agent_type
        let requested_model_tier = params.model.unwrap_or(agent_type.model_tier);
        let model = match requested_model_tier {
            ModelTier::Small => config
                .small_model
                .clone()
                .unwrap_or_else(|| turn.client.get_model()),
            _ => turn.client.get_model(),
        };
        let task_model_info = session
            .services
            .models_manager
            .get_model_info(&model, &config)
            .await;

        // set config base on agent_type and the model_info recently get
        let mut tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &task_model_info,
            features: &config.features,
            web_search_mode: config.web_search_mode,
            agent_configs: Some(&session.services.agent_type_manager.agent_configs()),
        });

        agent_type
            .apply_to_config(&mut config, &mut tools_config)
            .map_err(FunctionCallError::RespondToModel)?;

        // Set the model on config so the spawned agent uses the correct model
        config.model = Some(model);

        // Spawn the agent
        let result = session
            .services
            .agent_control
            .spawn_agent(config, params.prompt.clone())
            .await
            .map_err(collab_spawn_error);

        // Get initial status for SubAgentSpawnEnd event
        let (new_thread_id, initial_status) = match &result {
            Ok(thread_id) => (
                Some(*thread_id),
                session.services.agent_control.get_status(*thread_id).await,
            ),
            Err(_) => (None, AgentStatus::NotFound),
        };

        // Emit SubAgentSpawnEnd event
        session
            .send_event(
                &turn,
                SubAgentEndEvent {
                    call_id: call_id.clone(),
                    sender_thread_id: session.conversation_id,
                    new_thread_id,
                    prompt: params.prompt,
                    status: initial_status,
                }
                .into(),
            )
            .await;

        // Propagate spawn error if any
        let new_thread_id = result?;

        // Wait for agent to complete
        let final_status = match session
            .services
            .agent_control
            .subscribe_status(new_thread_id)
            .await
        {
            Ok(mut status_rx) => loop {
                let status = status_rx.borrow().clone();
                if is_final(&status) {
                    break status;
                }
                if status_rx.changed().await.is_err() {
                    break AgentStatus::NotFound;
                }
            },
            Err(_) => {
                session
                    .services
                    .agent_control
                    .get_status(new_thread_id)
                    .await
            }
        };

        // Emit SubAgentComplete event
        session
            .send_event(
                &turn,
                SubAgentCompleteEvent {
                    call_id,
                    sender_thread_id: session.conversation_id,
                    agent_thread_id: new_thread_id,
                    status: final_status.clone(),
                }
                .into(),
            )
            .await;

        let content = serde_json::to_string(&TaskResult {
            agent_id: new_thread_id.to_string(),
            status: final_status,
        })
        .map_err(|e| FunctionCallError::Fatal(format!("Failed to serialize task result: {e}")))?;

        Ok(ToolOutput::Function {
            content,
            success: Some(true),
            content_items: None,
        })
    }
}

fn collab_spawn_error(err: CodexErr) -> FunctionCallError {
    match err {
        CodexErr::UnsupportedOperation(_) => {
            FunctionCallError::RespondToModel("collab manager unavailable".to_string())
        }
        err => FunctionCallError::RespondToModel(format!("collab spawn failed: {err}")),
    }
}
