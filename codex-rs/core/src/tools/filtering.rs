//! Tool filtering for sub-agents.
//!
//! Implements blocking sets and agent-specific tool restrictions to prevent
//! infinite sub-agent nesting and enforce read-only agents.

use std::collections::HashMap;
use std::collections::HashSet;

/// Tools blocked from ALL sub-agents (prevents infinite nesting).
/// These tools are never available to sub-agents regardless of configuration.
pub static BLOCKED_FROM_SUBAGENTS: &[&str] = &[
    "task",        // Cannot spawn sub-sub-agents
    "update_plan", // Plan mode tools
];

/// Configuration for filtering tools in sub-agent sessions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubAgentToolFilter {
    /// Agent-specific tool enable/disable map.
    /// If a tool is mapped to `false`, it will be blocked.
    /// If a tool is mapped to `true`, it will be explicitly allowed.
    /// Tools not in this map follow default behavior.
    pub agent_tools: Option<HashMap<String, bool>>,
}

impl SubAgentToolFilter {
    /// Create a new filter with default settings (no agent-specific restrictions).
    pub fn new() -> Self {
        Self { agent_tools: None }
    }

    /// Create a new filter with agent-specific tool restrictions.
    pub fn with_agent_tools(tools: HashMap<String, bool>) -> Self {
        Self {
            agent_tools: Some(tools),
        }
    }

    /// Check if a tool is allowed for this sub-agent.
    ///
    /// Returns `false` if:
    /// 1. The tool is in BLOCKED_FROM_SUBAGENTS (always blocked)
    /// 2. The tool is explicitly disabled in agent_tools
    ///
    /// Returns `true` otherwise.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        // 1. Always block tools in BLOCKED_FROM_SUBAGENTS
        if BLOCKED_FROM_SUBAGENTS.contains(&tool_name) {
            return false;
        }

        // 2. Apply agent-specific tool config if present
        if let Some(config) = &self.agent_tools
            && let Some(&enabled) = config.get(tool_name)
        {
            return enabled;
        }
        // If config exists but tool not listed, allow by default

        true
    }

    /// Filter a list of tool names, returning only those allowed for this sub-agent.
    pub fn filter_tools(&self, available_tools: &[String]) -> Vec<String> {
        available_tools
            .iter()
            .filter(|tool_name| self.is_tool_allowed(tool_name))
            .cloned()
            .collect()
    }

    /// Get the set of tool names that are blocked for all sub-agents.
    pub fn blocked_tools() -> HashSet<&'static str> {
        BLOCKED_FROM_SUBAGENTS.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocks_task_tool() {
        let filter = SubAgentToolFilter::new();
        assert!(!filter.is_tool_allowed("task"));
        assert!(!filter.is_tool_allowed("update_plan"));
    }

    #[test]
    fn test_allows_regular_tools() {
        let filter = SubAgentToolFilter::new();
        assert!(filter.is_tool_allowed("shell"));
        assert!(filter.is_tool_allowed("read_file"));
        assert!(filter.is_tool_allowed("apply_patch"));
    }

    #[test]
    fn test_filter_tools_removes_blocked() {
        let filter = SubAgentToolFilter::new();
        let tools = vec![
            "shell".to_string(),
            "task".to_string(),
            "read_file".to_string(),
            "update_plan".to_string(),
        ];
        let filtered = filter.filter_tools(&tools);
        assert!(!filtered.contains(&"task".to_string()));
        assert!(!filtered.contains(&"update_plan".to_string()));
        assert!(filtered.contains(&"shell".to_string()));
        assert!(filtered.contains(&"read_file".to_string()));
    }

    #[test]
    fn test_applies_agent_config_disable() {
        let mut config = HashMap::new();
        config.insert("shell".to_string(), false);
        config.insert("read_file".to_string(), true);

        let filter = SubAgentToolFilter::with_agent_tools(config);

        assert!(!filter.is_tool_allowed("shell")); // Explicitly disabled
        assert!(filter.is_tool_allowed("read_file")); // Explicitly enabled
        assert!(filter.is_tool_allowed("apply_patch")); // Not in config, allowed
    }

    #[test]
    fn test_agent_config_cannot_override_blocked() {
        let mut config = HashMap::new();
        config.insert("task".to_string(), true); // Try to enable blocked tool

        let filter = SubAgentToolFilter::with_agent_tools(config);

        // Task is still blocked despite being enabled in agent config
        assert!(!filter.is_tool_allowed("task"));
    }

    #[test]
    fn test_filter_tools_with_agent_config() {
        let mut config = HashMap::new();
        config.insert("shell".to_string(), false);
        config.insert("apply_patch".to_string(), false);

        let filter = SubAgentToolFilter::with_agent_tools(config);
        let tools = vec![
            "shell".to_string(),
            "read_file".to_string(),
            "apply_patch".to_string(),
            "grep_files".to_string(),
        ];

        let filtered = filter.filter_tools(&tools);
        assert!(!filtered.contains(&"shell".to_string())); // Disabled
        assert!(!filtered.contains(&"apply_patch".to_string())); // Disabled
        assert!(filtered.contains(&"read_file".to_string())); // Not in config
        assert!(filtered.contains(&"grep_files".to_string())); // Not in config
    }

    #[test]
    fn test_blocked_tools_set() {
        let blocked = SubAgentToolFilter::blocked_tools();
        assert!(blocked.contains("task"));
        assert!(blocked.contains("update_plan"));
        assert!(!blocked.contains("shell"));
    }
}
