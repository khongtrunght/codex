use std::sync::Arc;

use async_trait::async_trait;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::items::TurnItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::AgentMessageContentDeltaEvent;
use codex_protocol::protocol::AgentMessageDeltaEvent;
use codex_protocol::protocol::AgentReasoningDeltaEvent;
use codex_protocol::protocol::AgentReasoningRawContentDeltaEvent;
use codex_protocol::protocol::EnhancePromptCompletedEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::ReasoningContentDeltaEvent;
use codex_protocol::protocol::ReasoningRawContentDeltaEvent;
use codex_protocol::protocol::TurnCompleteEvent;
use codex_protocol::protocol::WarningEvent;
use codex_protocol::user_input::UserInput;
use tokio_util::sync::CancellationToken;

use crate::client_common::ENHANCE_PROMPT;
use crate::codex::TurnContext;
use crate::codex_delegate::run_codex_thread_one_shot;
use crate::state::TaskKind;

use super::SessionTask;
use super::SessionTaskContext;

#[derive(Clone, Copy)]
pub(crate) struct EnhancePromptTask;

impl EnhancePromptTask {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SessionTask for EnhancePromptTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Enhance
    }

    async fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        let _ = session
            .session
            .services
            .otel_manager
            .counter("codex.task.enhance_prompt", 1, &[]);

        let output = match start_enhance_conversation(
            session.clone(),
            ctx.clone(),
            input,
            cancellation_token.clone(),
        )
        .await
        {
            Some(receiver) => process_enhance_events(session.clone(), ctx.clone(), receiver).await,
            None => None,
        };

        if cancellation_token.is_cancelled() {
            return None;
        }

        let Some(prompt) = output.filter(|text| !text.trim().is_empty()) else {
            let message = "Prompt enhancement did not return any text.".to_string();
            session
                .clone_session()
                .send_event(ctx.as_ref(), EventMsg::Warning(WarningEvent { message }))
                .await;
            return None;
        };

        session
            .clone_session()
            .send_event(
                ctx.as_ref(),
                EventMsg::EnhancePromptCompleted(EnhancePromptCompletedEvent { prompt }),
            )
            .await;
        None
    }
}

async fn start_enhance_conversation(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    input: Vec<UserInput>,
    cancellation_token: CancellationToken,
) -> Option<async_channel::Receiver<Event>> {
    let config = ctx.client.config();
    let mut sub_agent_config = config.as_ref().clone();
    sub_agent_config.base_instructions = Some(ENHANCE_PROMPT.to_string());
    sub_agent_config.web_search_mode = Some(WebSearchMode::Disabled);
    sub_agent_config.model_reasoning_effort = Some(ReasoningEffort::Low);

    let model = config
        .small_model
        .clone()
        .unwrap_or_else(|| ctx.client.get_model());
    sub_agent_config.model = Some(model);

    run_codex_thread_one_shot(
        sub_agent_config,
        session.auth_manager(),
        session.models_manager(),
        input,
        session.clone_session(),
        ctx,
        cancellation_token,
        None,
    )
    .await
    .ok()
    .map(|io| io.rx_event)
}

async fn process_enhance_events(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    receiver: async_channel::Receiver<Event>,
) -> Option<String> {
    while let Ok(event) = receiver.recv().await {
        match event.clone().msg {
            EventMsg::AgentMessage(_)
            | EventMsg::UserMessage(_)
            | EventMsg::AgentReasoning(_)
            | EventMsg::AgentReasoningRawContent(_)
            | EventMsg::AgentMessageDelta(AgentMessageDeltaEvent { .. })
            | EventMsg::AgentMessageContentDelta(AgentMessageContentDeltaEvent { .. })
            | EventMsg::AgentReasoningDelta(AgentReasoningDeltaEvent { .. })
            | EventMsg::AgentReasoningRawContentDelta(AgentReasoningRawContentDeltaEvent {
                ..
            })
            | EventMsg::ReasoningContentDelta(ReasoningContentDeltaEvent { .. })
            | EventMsg::ReasoningRawContentDelta(ReasoningRawContentDeltaEvent { .. }) => {}
            EventMsg::ItemCompleted(ItemCompletedEvent {
                item: TurnItem::AgentMessage(_),
                ..
            }) => {}
            EventMsg::TurnComplete(TurnCompleteEvent { last_agent_message }) => {
                return last_agent_message;
            }
            EventMsg::TurnAborted(_) => {
                return None;
            }
            other => {
                session
                    .clone_session()
                    .send_event(ctx.as_ref(), other)
                    .await;
            }
        }
    }
    None
}
