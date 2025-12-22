//! Sub-agent system prompt construction.
//!
//! This module provides functions to build augmented system prompts for sub-agents
//! spawned by the Task tool. The approach follows Claude Code's multi-layer prompt
//! structure.
//!
//! Note: Environment context (cwd, sandbox, etc.) is handled separately by
//! `EnvironmentContext` which injects XML as a user message. This module only
//! handles the system prompt layers.

/// Notes appended to all sub-agent system prompts.
///
/// These notes ensure sub-agents follow best practices for:
/// - Using absolute file paths (since cwd resets between bash calls)
/// - Returning file paths in responses
/// - Avoiding emojis for clear communication
pub const SUBAGENT_NOTES: &str = r#"
Notes:
- Agent threads always have their cwd reset between bash calls, as a result please only use absolute file paths.
- In your final response always share relevant file names and code snippets. Any file paths you return in your response MUST be absolute. Do NOT use relative paths.
- For clear communication with the user the assistant MUST avoid using emojis."#;

/// Default system prompt for sub-agents without a custom prompt.
pub const DEFAULT_SUBAGENT_PROMPT: &str = "You are a coding agent. \
     Given the user's message, you should use the tools available to complete the task. \
     Do what has been asked; nothing more, nothing less. \
     When you complete the task simply respond with a detailed writeup.";

/// Augment an agent's base system prompt with standard notes.
///
/// This creates the system prompt structure used by sub-agents:
/// 1. Agent-specific prompt (from AgentTypeConfig.system_prompt or default)
/// 2. Standard agent notes (absolute paths, no emojis, etc.)
///
/// Returns the complete system prompt as a single string.
pub fn augment_system_prompt(base_prompt: &str) -> String {
    format!("{base_prompt}\n{SUBAGENT_NOTES}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_augment_system_prompt() {
        let base = "You are a test agent.";
        let result = augment_system_prompt(base);

        assert!(result.starts_with("You are a test agent."));
        assert!(result.contains("Notes:"));
        assert!(result.contains("absolute file paths"));
        assert!(result.contains("avoid using emojis"));
    }

    #[test]
    fn test_augment_with_default_prompt() {
        let result = augment_system_prompt(DEFAULT_SUBAGENT_PROMPT);

        assert!(result.contains("Claude Code"));
        assert!(result.contains("Notes:"));
    }

    #[test]
    fn test_notes_contain_required_guidance() {
        // Verify all key guidance is present in SUBAGENT_NOTES
        assert!(SUBAGENT_NOTES.contains("absolute file paths"));
        assert!(SUBAGENT_NOTES.contains("cwd reset"));
        assert!(SUBAGENT_NOTES.contains("emojis"));
        assert!(SUBAGENT_NOTES.contains("final response"));
    }

    #[test]
    fn test_default_prompt_is_actionable() {
        // Verify default prompt provides clear direction
        assert!(DEFAULT_SUBAGENT_PROMPT.contains("Claude Code"));
        assert!(DEFAULT_SUBAGENT_PROMPT.contains("tools available"));
        assert!(DEFAULT_SUBAGENT_PROMPT.contains("complete the task"));
    }
}
