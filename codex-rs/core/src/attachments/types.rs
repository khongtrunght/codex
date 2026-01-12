//! Attachment type definitions.
//!
//! All attachment variants are defined here with their data and conversion logic.

use codex_protocol::models::AttachmentData;
use codex_protocol::models::CompactRestoredFile;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;

use crate::prompt_template::PlanModeEnhancedPrompt;
use crate::prompt_template::PlanModeLegacyPrompt;
use crate::prompt_template::PlanModeReentryPrompt;
use crate::prompt_template::PlanModeSubagentPrompt;
use crate::prompt_template::ToolConfig;

/// Number of parallel Plan agents to use for multi-perspective planning.
const PLAN_AGENT_COUNT: usize = 3;

/// Number of parallel Explore agents to use for codebase research.
const EXPLORE_AGENT_COUNT: usize = 3;

/// Turns between attachment injections (matching JS TURNS_BETWEEN_ATTACHMENTS).
pub const TURNS_BETWEEN_ATTACHMENTS: usize = 5;

/// Convert AttachmentData to ResponseItem::Message items for API call.
/// This follows the GhostSnapshot pattern: Attachment is stored in history,
/// then expanded to Message items during get_history_for_prompt().
pub fn attachment_data_to_messages(
    data: &AttachmentData,
    tools: &ToolConfig,
) -> Vec<ResponseItem> {
    match data {
        AttachmentData::PlanMode {
            plan_file_path,
            is_subagent,
            plan_exists,
        } => {
            #[cfg(feature = "exit_plan_mode")]
            {
                if *is_subagent {
                    generate_subagent_items(plan_file_path, *plan_exists, tools)
                } else {
                    generate_main_session_items(plan_file_path, *plan_exists, tools)
                }
            }
            #[cfg(not(feature = "exit_plan_mode"))]
            {
                generate_legacy_plan_mode_items(tools, *is_subagent)
            }
        }
        AttachmentData::PlanModeReentry { plan_file_path } => {
            generate_reentry_items(plan_file_path, tools)
        }
        AttachmentData::PlanModeExit { plan_file_path } => generate_exit_items(plan_file_path),
        AttachmentData::CompactFileRestore { files } => {
            generate_compact_file_restore_items(files)
        }
    }
}

/// Expand Attachment items in history to Message items (GhostSnapshot pattern).
/// Called during get_history_for_prompt() to convert Attachment markers to actual messages.
pub fn expand_attachments(items: Vec<ResponseItem>, tools: &ToolConfig) -> Vec<ResponseItem> {
    items
        .into_iter()
        .flat_map(|item| match item {
            ResponseItem::Attachment { ref data, .. } => {
                // Expand to Message items
                attachment_data_to_messages(data, tools)
            }
            other => vec![other],
        })
        .collect()
}

// =============================================================================
// Plan Mode Templates
// =============================================================================

fn plan_agent_count_from_env_or_defaults() -> usize {
    std::env::var("CODEX_PLAN_V2_AGENT_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| (1..=10).contains(&v))
        .unwrap_or(PLAN_AGENT_COUNT)
}

fn explore_agent_count_from_env_or_default() -> usize {
    std::env::var("CLAUDE_CODE_PLAN_V2_EXPLORE_AGENT_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| (1..=10).contains(&v))
        .unwrap_or(EXPLORE_AGENT_COUNT)
}

// =============================================================================
// Generation Functions
// =============================================================================

fn generate_main_session_items(
    plan_file_path: &str,
    plan_exists: bool,
    tools: &ToolConfig,
) -> Vec<ResponseItem> {
    let plan_file_info = if plan_exists {
        format!(
            "A plan file already exists at {plan_file_path}. You can read it and make incremental edits using the {} tool.",
            tools.edit_tool_name()
        )
    } else {
        format!(
            "No plan file exists yet. You should create your plan at {plan_file_path} using the {} tool.",
            tools.write_tool_name()
        )
    };

    let num_planing_agent = plan_agent_count_from_env_or_defaults();

    let contents = PlanModeEnhancedPrompt {
        tools: tools.clone(),
        plan_file_info,
        plan_agent_count: num_planing_agent,
        explore_agent_count: explore_agent_count_from_env_or_default(),
        multi_agent_mode: num_planing_agent > 1,
    }
    .to_string();

    vec![wrap_in_system_reminder(&contents)]
}

#[allow(dead_code)]
fn generate_legacy_plan_mode_items(tools: &ToolConfig, is_subagent: bool) -> Vec<ResponseItem> {
    let contents = PlanModeLegacyPrompt {
        tools: tools.clone(),
        is_subagent,
    }
    .to_string();

    vec![wrap_in_system_reminder(&contents)]
}

fn generate_subagent_items(
    plan_file_path: &str,
    plan_exists: bool,
    tools: &ToolConfig,
) -> Vec<ResponseItem> {
    let plan_file_info = if plan_exists {
        format!(
            "A plan file already exists at {plan_file_path}. You can read it and make incremental edits using the {} tool if you need to.",
            tools.edit_tool_name()
        )
    } else {
        format!(
            "No plan file exists yet. You should create your plan at {plan_file_path} using the {} tool if you need to.",
            tools.write_tool_name()
        )
    };

    let contents = PlanModeSubagentPrompt {
        tools: tools.clone(),
        plan_file_info,
    }
    .to_string();

    vec![wrap_in_system_reminder(&contents)]
}

fn generate_reentry_items(plan_file_path: &str, tools: &ToolConfig) -> Vec<ResponseItem> {
    let contents = PlanModeReentryPrompt {
        tools: tools.clone(),
        plan_file_path: plan_file_path.to_string(),
    }
    .to_string();

    vec![wrap_in_system_reminder(&contents)]
}

/// Template for plan mode exit notification.
/// This is injected ONCE when the user exits plan mode via UI (shift+tab).
const PLAN_MODE_EXIT_TEMPLATE: &str = r#"## Exited Plan Mode

You have exited plan mode. You can now make edits, run tools, and take actions. {plan_file_info}
"#;

fn generate_exit_items(plan_file_path: &Option<String>) -> Vec<ResponseItem> {
    let plan_file_info = match plan_file_path {
        Some(path) => format!(" The plan file is located at {path} if you need to reference it."),
        None => "No plan file was created during the planning session.".to_string(),
    };

    let contents = PLAN_MODE_EXIT_TEMPLATE.replace("{plan_file_info}", &plan_file_info);

    vec![wrap_in_system_reminder(&contents)]
}

/// Wrap content in <system-reminder> tags as a user message.
fn wrap_in_system_reminder(contents: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: format!("<system-reminder>\n{contents}\n</system-reminder>"),
        }],
    }
}

// =============================================================================
// Compact File Restore
// =============================================================================

/// Generate items for restored files after compaction.
fn generate_compact_file_restore_items(files: &[CompactRestoredFile]) -> Vec<ResponseItem> {
    if files.is_empty() {
        return vec![];
    }

    let mut items = Vec::new();

    for file in files {
        match file {
            CompactRestoredFile::WithContent {
                path,
                content,
                truncated,
                ..
            } => {
                // Full content restoration
                let truncate_note = if *truncated {
                    " truncated=\"true\""
                } else {
                    ""
                };
                let msg =
                    format!("<file_context path=\"{path}\"{truncate_note}>\\n{content}\\n</file_context>");
                items.push(wrap_in_system_reminder(&msg));
            }
            CompactRestoredFile::ReferenceOnly { path } => {
                // Reference only - tell model to re-read if needed
                let msg = format!(
                    "Note: {path} was read before the last conversation was summarized, \
                     but the contents are too large to include. Use read_file tool if you need to access it."
                );
                items.push(wrap_in_system_reminder(&msg));
            }
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::openai_models::ConfigShellToolType;
    use codex_protocol::openai_models::EditToolType;

    #[test]
    fn test_plan_mode_main_session() {
        let data = AttachmentData::PlanMode {
            plan_file_path: "/tmp/plan.md".to_string(),
            is_subagent: false,
            plan_exists: false,
        };

        let tools = ToolConfig::new(Some(EditToolType::FileEdit), ConfigShellToolType::Bash);

        let items = attachment_data_to_messages(&data, &tools);
        assert_eq!(items.len(), 1);

        if let ResponseItem::Message { content, .. } = &items[0] {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("<system-reminder>"));
                assert!(text.contains("Plan mode is active"));
                assert!(text.contains("Phase 1: Initial Understanding"));
                assert!(text.contains("/tmp/plan.md"));
            }
        }
    }

    #[test]
    fn test_plan_mode_subagent() {
        let data = AttachmentData::PlanMode {
            plan_file_path: "/tmp/plan.md".to_string(),
            is_subagent: true,
            plan_exists: true,
        };

        let tools = ToolConfig::new(Some(EditToolType::FileEdit), ConfigShellToolType::Bash);

        let items = attachment_data_to_messages(&data, &tools);
        assert_eq!(items.len(), 1);

        if let ResponseItem::Message { content, .. } = &items[0] {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("Answer the user's query comprehensively"));
                assert!(!text.contains("Phase 1")); // Subagent doesn't get phases
            }
        }
    }

    #[test]
    fn test_plan_mode_reentry() {
        let data = AttachmentData::PlanModeReentry {
            plan_file_path: "/tmp/plan.md".to_string(),
        };

        let tools = ToolConfig::new(Some(EditToolType::FileEdit), ConfigShellToolType::Bash);

        let items = attachment_data_to_messages(&data, &tools);
        assert_eq!(items.len(), 1);

        if let ResponseItem::Message { content, .. } = &items[0] {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("Re-entering Plan Mode"));
                assert!(text.contains("/tmp/plan.md"));
            }
        }
    }

    #[test]
    fn test_expand_attachments() {
        let tools = ToolConfig::new(Some(EditToolType::FileEdit), ConfigShellToolType::Bash);

        // Create history with Attachment item
        let history = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "test".to_string(),
                }],
            },
            ResponseItem::Attachment {
                data: AttachmentData::PlanMode {
                    plan_file_path: "/tmp/plan.md".to_string(),
                    is_subagent: false,
                    plan_exists: false,
                },
                timestamp: "2024-01-01T00:00:00Z".to_string(),
            },
        ];

        // Expand attachments
        let expanded = expand_attachments(history, &tools);

        // Should have 2 items: original user message + expanded plan mode message
        assert_eq!(expanded.len(), 2);

        // First should be original user message
        assert!(matches!(&expanded[0], ResponseItem::Message { role, .. } if role == "user"));

        // Second should be expanded plan mode (NOT Attachment)
        assert!(matches!(&expanded[1], ResponseItem::Message { .. }));
        assert!(!matches!(&expanded[1], ResponseItem::Attachment { .. }));
    }
}
