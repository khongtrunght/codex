use codex_core::protocol::AgentStatus;
use codex_core::protocol::EventMsg;
use codex_protocol::ThreadId;
use std::time::Duration;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubAgentStatus {
    Running,
    Completed(Option<String>),
    Error(String),
}

#[derive(Debug)]
pub(crate) struct SubAgentEntry {
    #[allow(dead_code)]
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

    pub(crate) fn set_thread_id(&mut self, id: ThreadId) {
        self.thread_id = Some(id);
    }

    pub(crate) fn add_event(&mut self, event: EventMsg) {
        self.raw_events.push(event);
    }

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

    pub(crate) fn is_completed(&self) -> bool {
        matches!(
            self.status,
            SubAgentStatus::Completed(_) | SubAgentStatus::Error(_)
        )
    }
}

#[derive(Debug)]
pub(crate) struct SubAgentCell {
    pub(super) agents: Vec<SubAgentEntry>,
    pub(super) animations_enabled: bool,
}

impl SubAgentCell {
    pub(crate) fn new(entry: SubAgentEntry, animations_enabled: bool) -> Self {
        Self {
            agents: vec![entry],
            animations_enabled,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_entries(entries: Vec<SubAgentEntry>, animations_enabled: bool) -> Self {
        Self {
            agents: entries,
            animations_enabled,
        }
    }

    pub(crate) fn is_completed(&self) -> bool {
        self.agents.iter().all(SubAgentEntry::is_completed)
    }

    pub(crate) fn is_active(&self) -> bool {
        self.agents.iter().any(|a| !a.is_completed())
    }

    pub(crate) fn running_thread_ids(&self) -> impl Iterator<Item = ThreadId> + '_ {
        self.agents
            .iter()
            .filter(|a| !a.is_completed())
            .filter_map(|a| a.thread_id)
    }

    pub(crate) fn active_start_time(&self) -> Option<Instant> {
        self.agents
            .iter()
            .find(|a| !a.is_completed())
            .and_then(|a| a.start_time)
    }

    pub(crate) fn set_thread_id(&mut self, id: ThreadId) {
        if let Some(agent) = self.agents.first_mut() {
            agent.set_thread_id(id);
        }
    }

    pub(crate) fn add_event(&mut self, event: EventMsg) {
        if let Some(agent) = self.agents.first_mut() {
            agent.add_event(event);
        }
    }

    pub(crate) fn complete_with_status(&mut self, status: &AgentStatus) {
        if let Some(agent) = self.agents.first_mut() {
            agent.complete_with_status(status);
        }
    }

    #[cfg(test)]
    pub(crate) fn find_agent_mut(&mut self, call_id: &str) -> Option<&mut SubAgentEntry> {
        self.agents.iter_mut().find(|a| a.call_id == call_id)
    }

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

    pub(crate) fn mark_interrupted(&mut self) {
        for agent in &mut self.agents {
            if !agent.is_completed() {
                agent.status = SubAgentStatus::Error("Session interrupted".to_string());
                agent.duration = agent.start_time.map(|st| st.elapsed());
            }
        }
    }
}
