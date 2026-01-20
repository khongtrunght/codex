// ============================================================================
// SubAgent cells for displaying Task tool invocations
// ============================================================================

use codex_core::protocol::AgentStatus;
use codex_core::protocol::EventMsg;
use codex_protocol::ThreadId;
use std::time::Duration;
use std::time::Instant;

/// Status of a sub-agent task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubAgentStatus {
    Running,
    /// Completed with optional final response.
    Completed(Option<String>),
    /// Error with error message.
    Error(String),
}

/// Cell displaying a sub-agent task with compact view.
#[derive(Debug)]
pub(crate) struct SubAgentCell {
    #[allow(dead_code)] // Used as HashMap key in chatwidget, retained for debugging
    pub(super) call_id: String,
    pub(super) thread_id: Option<ThreadId>,
    pub(super) agent_type: String,
    pub(super) description: String,
    pub(super) status: SubAgentStatus,
    pub(super) start_time: Option<Instant>,
    pub(super) duration: Option<Duration>,
    pub(super) raw_events: Vec<EventMsg>,
    pub(super) animations_enabled: bool,
}

impl SubAgentCell {
    /// Create a new SubAgentCell.
    pub(crate) fn new(
        call_id: String,
        agent_type: String,
        description: String,
        animations_enabled: bool,
    ) -> Self {
        SubAgentCell {
            call_id,
            thread_id: None,
            agent_type,
            description,
            status: SubAgentStatus::Running,
            start_time: Some(Instant::now()),
            duration: None,
            raw_events: Vec::new(),
            animations_enabled,
        }
    }

    /// Set the thread ID after spawn.
    pub(crate) fn set_thread_id(&mut self, id: ThreadId) {
        self.thread_id = Some(id);
    }

    /// Add a forwarded event from the subagent.
    pub(crate) fn add_event(&mut self, event: EventMsg) {
        self.raw_events.push(event);
    }

    /// Mark the cell as completed with the given status.
    pub(crate) fn complete_with_status(&mut self, status: &AgentStatus) {
        self.duration = self.start_time.map(|st| st.elapsed());
        self.status = match status {
            AgentStatus::Completed(response) => SubAgentStatus::Completed(response.clone()),
            AgentStatus::Errored(msg) => SubAgentStatus::Error(msg.clone()),
            AgentStatus::NotFound => SubAgentStatus::Error("Agent not found".to_string()),
            AgentStatus::Shutdown => SubAgentStatus::Error("Agent shutdown".to_string()),
            AgentStatus::PendingInit | AgentStatus::Running => SubAgentStatus::Running,
        };
    }

    /// Check if the subagent has completed (successfully or with error).
    pub(crate) fn is_completed(&self) -> bool {
        matches!(
            self.status,
            SubAgentStatus::Completed(_) | SubAgentStatus::Error(_)
        )
    }
}

/// Cell for grouping completed sub-agents.
#[derive(Debug)]
pub(crate) struct SubAgentGroupCell {
    pub(super) cells: Vec<SubAgentCell>,
}

impl SubAgentGroupCell {
    pub(crate) fn new(cells: Vec<SubAgentCell>) -> Self {
        Self { cells }
    }
}
