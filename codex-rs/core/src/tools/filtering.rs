//! Tool filtering for sub-agents.
//!
//! Implements blocking sets and agent-specific tool restrictions to prevent
//! infinite sub-agent nesting and enforce read-only agents.

use std::collections::HashSet;

/// Tools blocked from ALL sub-agents (prevents infinite nesting).
/// These tools are never available to sub-agents regardless of configuration.
pub static BLOCKED_FROM_SUBAGENTS: &[&str] = &[
    "task",            // Cannot spawn sub-sub-agents
    "todo_write",      // TODO tracking tools
    "enter_plan_mode", // Plan mode is for main session only
    "exit_plan_mode",  // Plan mode is for main session only
];

/// Configuration for filtering tools in sub-agent sessions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubAgentToolFilter {
    /// List of allowed tools for this agent.
    /// If None, all tools are allowed (except blocked ones).
    /// If Some, only listed tools are allowed.
    pub allowed_tools: Option<Vec<String>>,
}

impl SubAgentToolFilter {
    /// Create a new filter with default settings (no agent-specific restrictions).
    pub fn new() -> Self {
        Self {
            allowed_tools: None,
        }
    }

    /// Create a new filter with a list of allowed tools.
    pub fn with_allowed_tools(tools: Vec<String>) -> Self {
        Self {
            allowed_tools: Some(tools),
        }
    }

    /// Check if a tool is allowed for this sub-agent.
    ///
    /// Returns `false` if:
    /// 1. The tool is in BLOCKED_FROM_SUBAGENTS (always blocked)
    /// 2. allowed_tools is Some and the tool is not in the list
    ///
    /// Returns `true` otherwise.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        // 1. Always block tools in BLOCKED_FROM_SUBAGENTS
        if BLOCKED_FROM_SUBAGENTS.contains(&tool_name) {
            return false;
        }

        // 2. If allowed_tools is set, only allow tools in that list
        if let Some(allowed) = &self.allowed_tools {
            return allowed.iter().any(|t| t == tool_name);
        }

        // No restrictions - allow by default
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
        assert!(!filter.is_tool_allowed("todo_write"));
        assert!(!filter.is_tool_allowed("enter_plan_mode"));
        assert!(!filter.is_tool_allowed("exit_plan_mode"));
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
            "todo_write".to_string(),
        ];
        let filtered = filter.filter_tools(&tools);
        assert!(!filtered.contains(&"task".to_string()));
        assert!(!filtered.contains(&"todo_write".to_string()));
        assert!(filtered.contains(&"shell".to_string()));
        assert!(filtered.contains(&"read_file".to_string()));
    }

    #[test]
    fn test_allowed_tools_restricts() {
        let filter = SubAgentToolFilter::with_allowed_tools(vec![
            "read_file".to_string(),
            "glob".to_string(),
        ]);

        assert!(filter.is_tool_allowed("read_file")); // In allowed list
        assert!(filter.is_tool_allowed("glob")); // In allowed list
        assert!(!filter.is_tool_allowed("shell")); // Not in allowed list
        assert!(!filter.is_tool_allowed("apply_patch")); // Not in allowed list
    }

    #[test]
    fn test_allowed_tools_cannot_override_blocked() {
        let filter = SubAgentToolFilter::with_allowed_tools(vec![
            "task".to_string(), // Try to allow blocked tool
            "read_file".to_string(),
        ]);

        // Task is still blocked despite being in allowed list
        assert!(!filter.is_tool_allowed("task"));
        assert!(filter.is_tool_allowed("read_file"));
    }

    #[test]
    fn test_filter_tools_with_allowed_list() {
        let filter = SubAgentToolFilter::with_allowed_tools(vec![
            "read_file".to_string(),
            "grep_files".to_string(),
        ]);
        let tools = vec![
            "shell".to_string(),
            "read_file".to_string(),
            "apply_patch".to_string(),
            "grep_files".to_string(),
        ];

        let filtered = filter.filter_tools(&tools);
        assert!(!filtered.contains(&"shell".to_string())); // Not allowed
        assert!(!filtered.contains(&"apply_patch".to_string())); // Not allowed
        assert!(filtered.contains(&"read_file".to_string())); // Allowed
        assert!(filtered.contains(&"grep_files".to_string())); // Allowed
    }

    #[test]
    fn test_blocked_tools_set() {
        let blocked = SubAgentToolFilter::blocked_tools();
        assert!(blocked.contains("task"));
        assert!(blocked.contains("todo_write"));
        assert!(blocked.contains("enter_plan_mode"));
        assert!(blocked.contains("exit_plan_mode"));
        assert!(!blocked.contains("shell"));
    }
}
