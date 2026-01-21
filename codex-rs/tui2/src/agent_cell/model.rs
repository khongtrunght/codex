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

/// A single sub-agent invocation (analogous to ExecCall).
#[derive(Debug)]
pub(crate) struct SubAgentEntry {
    #[allow(dead_code)] // Stored for identification, will be used for agent lookup
    pub(super) call_id: String,
    pub(super) thread_id: Option<ThreadId>,
    pub(super) agent_type: String,
    pub(super) description: String,
    pub(super) prompt: Option<String>,
    pub(super) status: SubAgentStatus,
    pub(super) start_time: Option<Instant>,
    pub(super) duration: Option<Duration>,
    pub(super) raw_events: Vec<EventMsg>,
}

impl SubAgentEntry {
    /// Create a new SubAgentEntry.
    pub(crate) fn new(
        call_id: String,
        agent_type: String,
        description: String,
        prompt: Option<String>,
    ) -> Self {
        Self {
            call_id,
            thread_id: None,
            agent_type,
            description,
            prompt,
            status: SubAgentStatus::Running,
            start_time: Some(Instant::now()),
            duration: None,
            raw_events: Vec::new(),
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

    /// Mark the entry as completed with the given status.
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

/// Cell displaying one or more sub-agent tasks (analogous to ExecCell).
#[derive(Debug)]
pub(crate) struct SubAgentCell {
    pub(super) agents: Vec<SubAgentEntry>,
    pub(super) animations_enabled: bool,
}

impl SubAgentCell {
    /// Create a new SubAgentCell with a single agent entry.
    pub(crate) fn new(entry: SubAgentEntry, animations_enabled: bool) -> Self {
        Self {
            agents: vec![entry],
            animations_enabled,
        }
    }

    /// Create a SubAgentCell from multiple completed entries (for grouping).
    #[cfg(test)] // Used by render tests for creating multi-agent cells
    pub(crate) fn from_entries(entries: Vec<SubAgentEntry>, animations_enabled: bool) -> Self {
        Self {
            agents: entries,
            animations_enabled,
        }
    }

    /// Check if all agents have completed.
    pub(crate) fn is_completed(&self) -> bool {
        self.agents.iter().all(SubAgentEntry::is_completed)
    }

    /// Check if any agent is still running.
    pub(crate) fn is_active(&self) -> bool {
        self.agents.iter().any(|a| !a.is_completed())
    }

    /// Get the start time of the first active agent (for spinner).
    pub(crate) fn active_start_time(&self) -> Option<Instant> {
        self.agents
            .iter()
            .find(|a| !a.is_completed())
            .and_then(|a| a.start_time)
    }

    // -------------------------------------------------------------------------
    // Delegate methods for single-agent cells (compatibility layer)
    // -------------------------------------------------------------------------

    /// Set the thread ID on the first agent entry.
    /// For single-agent cells created via new_subagent_cell.
    pub(crate) fn set_thread_id(&mut self, id: ThreadId) {
        if let Some(agent) = self.agents.first_mut() {
            agent.set_thread_id(id);
        }
    }

    /// Add an event to the first agent entry.
    /// For single-agent cells created via new_subagent_cell.
    pub(crate) fn add_event(&mut self, event: EventMsg) {
        if let Some(agent) = self.agents.first_mut() {
            agent.add_event(event);
        }
    }

    /// Complete the first agent entry with the given status.
    /// For single-agent cells created via new_subagent_cell.
    pub(crate) fn complete_with_status(&mut self, status: &AgentStatus) {
        if let Some(agent) = self.agents.first_mut() {
            agent.complete_with_status(status);
        }
    }

    /// Find an agent entry by call_id.
    #[cfg(test)]
    pub(crate) fn find_agent_mut(&mut self, call_id: &str) -> Option<&mut SubAgentEntry> {
        self.agents.iter_mut().find(|a| a.call_id == call_id)
    }

    /// Merge multiple cells into one (for grouping completed subagents).
    pub(crate) fn merge(cells: Vec<SubAgentCell>) -> Option<Self> {
        if cells.is_empty() {
            return None;
        }
        let animations_enabled = cells.first().map(|c| c.animations_enabled).unwrap_or(false);
        let entries: Vec<SubAgentEntry> = cells.into_iter().flat_map(|c| c.agents).collect();
        Some(Self {
            agents: entries,
            animations_enabled,
        })
    }
}
