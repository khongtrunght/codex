use crate::protocol::EventMsg;
use crate::protocol::RolloutItem;
use codex_protocol::models::ResponseItem;

/// Whether a rollout `item` should be persisted in rollout files.
#[inline]
pub(crate) fn is_persisted_response_item(item: &RolloutItem) -> bool {
    match item {
        RolloutItem::ResponseItem(item) => should_persist_response_item(item),
        RolloutItem::EventMsg(ev) => should_persist_event_msg(ev),
        // Persist Codex executive markers so we can analyze flows (e.g., compaction, API turns).
        RolloutItem::Compacted(_) | RolloutItem::TurnContext(_) | RolloutItem::SessionMeta(_) => {
            true
        }
    }
}

/// Whether a `ResponseItem` should be persisted in rollout files.
#[inline]
pub(crate) fn should_persist_response_item(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::Message { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::GhostSnapshot { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::Attachment { .. } => true,
        ResponseItem::Other => false,
    }
}

/// Whether an `EventMsg` should be persisted in rollout files.
#[inline]
pub(crate) fn should_persist_event_msg(ev: &EventMsg) -> bool {
    match ev {
        EventMsg::UserMessage(_)
        | EventMsg::AgentMessage(_)
        | EventMsg::AgentReasoning(_)
        | EventMsg::AgentReasoningRawContent(_)
        | EventMsg::TokenCount(_)
        | EventMsg::ContextCompacted(_)
        | EventMsg::EnteredReviewMode(_)
        | EventMsg::ExitedReviewMode(_)
        | EventMsg::EnhancePromptStarted(_)
        | EventMsg::EnhancePromptCompleted(_)
        | EventMsg::ThreadRolledBack(_)
        | EventMsg::UndoCompleted(_)
        | EventMsg::TurnAborted(_)
        // SubAgent events are persisted for resume support
        | EventMsg::SubAgentSpawnBegin(_)
        | EventMsg::SubAgentSpawnEnd(_)
        | EventMsg::SubAgentComplete(_)
        // Visual content events are persisted so they appear on resume
        | EventMsg::ViewImageToolCall(_)
        | EventMsg::MermaidToolCall(_) => true,
        EventMsg::Error(_)
        | EventMsg::Warning(_)
        | EventMsg::TurnStarted(_)
        | EventMsg::TurnComplete(_)
        | EventMsg::AgentMessageDelta(_)
        | EventMsg::AgentReasoningDelta(_)
        | EventMsg::AgentReasoningRawContentDelta(_)
        | EventMsg::AgentReasoningSectionBreak(_)
        | EventMsg::RawResponseItem(_)
        | EventMsg::SessionConfigured(_)
        | EventMsg::McpToolCallBegin(_)
        | EventMsg::McpToolCallEnd(_)
        | EventMsg::WebSearchBegin(_)
        | EventMsg::WebSearchEnd(_)
        | EventMsg::ExecCommandBegin(_)
        | EventMsg::TerminalInteraction(_)
        | EventMsg::ExecCommandOutputDelta(_)
        | EventMsg::ExecCommandEnd(_)
        | EventMsg::ExecApprovalRequest(_)
        | EventMsg::AskUserQuestionRequest(_)
        | EventMsg::ElicitationRequest(_)
        | EventMsg::ApplyPatchApprovalRequest(_)
        | EventMsg::BackgroundEvent(_)
        | EventMsg::StreamError(_)
        | EventMsg::PatchApplyBegin(_)
        | EventMsg::PatchApplyEnd(_)
        | EventMsg::TurnDiff(_)
        | EventMsg::GetHistoryEntryResponse(_)
        | EventMsg::UndoStarted(_)
        | EventMsg::McpListToolsResponse(_)
        | EventMsg::McpStartupUpdate(_)
        | EventMsg::McpStartupComplete(_)
        | EventMsg::ListCustomPromptsResponse(_)
        | EventMsg::ListSkillsResponse(_)
        | EventMsg::PlanUpdate(_)
        | EventMsg::ShutdownComplete
        | EventMsg::DeprecationNotice(_)
        | EventMsg::ItemStarted(_)
        | EventMsg::ItemCompleted(_)
        | EventMsg::AgentMessageContentDelta(_)
        | EventMsg::ReasoningContentDelta(_)
        | EventMsg::ReasoningRawContentDelta(_)
        | EventMsg::SkillsUpdateAvailable
        | EventMsg::CollabAgentSpawnBegin(_)
        | EventMsg::CollabAgentSpawnEnd(_)
        | EventMsg::CollabAgentInteractionBegin(_)
        | EventMsg::CollabAgentInteractionEnd(_)
        | EventMsg::CollabWaitingBegin(_)
        | EventMsg::CollabWaitingEnd(_)
        | EventMsg::CollabCloseBegin(_)
        | EventMsg::CollabCloseEnd(_)
        | EventMsg::ExitPlanModeApprovalRequest(_)
        | EventMsg::ExitedPlanMode(_)
        | EventMsg::AttachmentLoaded(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::ThreadId;
    use codex_protocol::protocol::AgentStatus;
    use codex_protocol::protocol::SubAgentBeginEvent;
    use codex_protocol::protocol::SubAgentCompleteEvent;
    use codex_protocol::protocol::SubAgentEndEvent;

    #[test]
    fn subagent_spawn_begin_is_persisted() {
        let event = EventMsg::SubAgentSpawnBegin(SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            agent_type: "explore".to_string(),
            description: "Test agent".to_string(),
            prompt: "Do something".to_string(),
            sender_thread_id: ThreadId::new(),
            resumed: false,
        });
        assert!(
            should_persist_event_msg(&event),
            "SubAgentSpawnBegin should be persisted for resume support"
        );
    }

    #[test]
    fn subagent_spawn_end_is_persisted() {
        let event = EventMsg::SubAgentSpawnEnd(SubAgentEndEvent {
            call_id: "call-1".to_string(),
            sender_thread_id: ThreadId::new(),
            new_thread_id: Some(ThreadId::new()),
            prompt: "Do something".to_string(),
            status: AgentStatus::Running,
        });
        assert!(
            should_persist_event_msg(&event),
            "SubAgentSpawnEnd should be persisted for resume support"
        );
    }

    #[test]
    fn subagent_complete_is_persisted() {
        let event = EventMsg::SubAgentComplete(SubAgentCompleteEvent {
            call_id: "call-1".to_string(),
            sender_thread_id: ThreadId::new(),
            agent_thread_id: ThreadId::new(),
            status: AgentStatus::Completed(Some("Done".to_string())),
        });
        assert!(
            should_persist_event_msg(&event),
            "SubAgentComplete should be persisted for resume support"
        );
    }

    #[test]
    fn subagent_events_in_rollout_item_are_persisted() {
        let begin_event = EventMsg::SubAgentSpawnBegin(SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            agent_type: "explore".to_string(),
            description: "Test".to_string(),
            prompt: "prompt".to_string(),
            sender_thread_id: ThreadId::new(),
            resumed: false,
        });
        let item = RolloutItem::EventMsg(begin_event);
        assert!(
            is_persisted_response_item(&item),
            "RolloutItem containing SubAgent event should be persisted"
        );
    }
}
