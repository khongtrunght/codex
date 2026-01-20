pub(crate) mod control;
pub(crate) mod manager;
pub(crate) mod role;
pub(crate) mod spawn;
pub(crate) mod status;

pub(crate) use codex_protocol::protocol::AgentStatus;
pub(crate) use control::AgentControl;
pub(crate) use manager::AgentTypeManager;
pub(crate) use role::AgentRole;
pub(crate) use spawn::AgentSpawnContext;
pub(crate) use status::agent_status_from_event;
