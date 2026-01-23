use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

use crate::config_types::CollaborationMode;
use crate::models::CompactRestoredFile;
use crate::models::ContentItem;
use crate::models::ResponseItem;
use crate::protocol::SYSTEM_REMINDER_CLOSE_TAG;
use crate::protocol::SYSTEM_REMINDER_OPEN_TAG;

// Collaboration mode templates
const COLLABORATION_MODE_PLAN: &str = include_str!("prompts/collaboration_mode/plan.md");
const COLLABORATION_MODE_PAIR_PROGRAMMING: &str =
    include_str!("prompts/collaboration_mode/pair_programming.md");
const COLLABORATION_MODE_EXECUTE: &str = include_str!("prompts/collaboration_mode/execute.md");

/// Data payload for attachment types.
/// Attachments are contextual markers stored in history and expanded to messages before API calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AttachmentData {
    /// Collected when entering/re-entering plan mode.
    PlanMode {
        plan_file_path: String,
        is_subagent: bool,
        plan_exists: bool,
    },
    /// Collected when re-entering plan mode after being in a different mode.
    PlanModeReentry { plan_file_path: String },
    /// Collected when exiting plan mode.
    PlanModeExit {
        plan_file_path: Option<String>,
        /// The collaboration mode being transitioned to after exiting plan mode.
        next_mode: Option<CollaborationMode>,
    },
    /// Collected after compaction to restore file context.
    CompactFileRestore { files: Vec<CompactRestoredFile> },
}

impl From<AttachmentData> for ResponseItem {
    fn from(data: AttachmentData) -> Self {
        match data {
            AttachmentData::PlanMode {
                plan_file_path,
                is_subagent,
                plan_exists,
            } => generate_plan_mode_items(&plan_file_path, is_subagent, plan_exists),
            AttachmentData::PlanModeReentry { plan_file_path } => {
                generate_reentry_items(&plan_file_path)
            }
            AttachmentData::PlanModeExit {
                plan_file_path,
                next_mode,
            } => generate_exit_items(&plan_file_path, &next_mode),
            AttachmentData::CompactFileRestore { files } => {
                generate_compact_file_restore_items(&files)
            }
        }
    }
}

fn generate_plan_mode_items(
    plan_file_path: &str,
    is_subagent: bool,
    plan_exists: bool,
) -> ResponseItem {
    let plan_file_info = if plan_exists {
        format!(
            "A plan file already exists at {plan_file_path}. You can read it and make incremental edits to it."
        )
    } else {
        format!("No plan file exists yet. You should create your plan at {plan_file_path}.")
    };

    let subagent_note = if is_subagent {
        "\nNote: You are a subagent. Focus on your specific task."
    } else {
        ""
    };

    let contents = format!(
        "{SYSTEM_REMINDER_OPEN_TAG}\nPlan mode is active.{subagent_note}\n\n## Plan File Info:\n{plan_file_info}\n\n{COLLABORATION_MODE_PLAN}\n{SYSTEM_REMINDER_CLOSE_TAG}"
    );

    wrap_in_developer_message(&contents)
}

/// Generate items for plan mode reentry.
fn generate_reentry_items(plan_file_path: &str) -> ResponseItem {
    let contents = format!(
        "{SYSTEM_REMINDER_OPEN_TAG}\n## Re-entering Plan Mode\n\nYou are returning to plan mode. The plan file is at {plan_file_path}. Update it as needed.\n{SYSTEM_REMINDER_CLOSE_TAG}"
    );
    wrap_in_developer_message(&contents)
}

/// Generate items for plan mode exit.
fn generate_exit_items(
    plan_file_path: &Option<String>,
    next_mode: &Option<CollaborationMode>,
) -> ResponseItem {
    let plan_file_info = match plan_file_path {
        Some(path) => {
            format!(" The plan file is located at {path} if you need to reference it.")
        }
        None => "No plan file was created during the planning session.".to_string(),
    };

    let next_mode_instructions = match next_mode {
        Some(CollaborationMode::PairProgramming) => {
            format!("\n\n{COLLABORATION_MODE_PAIR_PROGRAMMING}")
        }
        Some(CollaborationMode::Execute) => {
            format!("\n\n{COLLABORATION_MODE_EXECUTE}")
        }
        _ => String::new(),
    };

    let contents = format!(
        "{SYSTEM_REMINDER_OPEN_TAG}\n## Exited Plan Mode\n\nYou have exited plan mode. You can now make edits, run tools, and take actions.{plan_file_info}{next_mode_instructions}\n{SYSTEM_REMINDER_CLOSE_TAG}"
    );

    wrap_in_developer_message(&contents)
}

/// Generate items for restored files after compaction.
fn generate_compact_file_restore_items(files: &[CompactRestoredFile]) -> ResponseItem {
    if files.is_empty() {
        return wrap_in_developer_message("No files were restored.");
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
                let truncate_note = if *truncated {
                    " truncated=\"true\""
                } else {
                    ""
                };
                let msg = format!(
                    "<file_context path=\"{path}\"{truncate_note}>\n{content}\n</file_context>"
                );
                items.push(ContentItem::InputText { text: msg });
            }
            CompactRestoredFile::ReferenceOnly { path } => {
                let msg = format!(
                    "Note: {path} was read before the last conversation was summarized, \
                     but the contents are too large to include. Use read_file tool if you need to access it."
                );
                items.push(ContentItem::InputText { text: msg });
            }
        }
    }

    ResponseItem::Message {
        id: None,
        role: "developer".to_string(),
        content: items,
    }
}

/// Wrap content in a user message.
fn wrap_in_developer_message(contents: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "developer".to_string(),
        content: vec![ContentItem::InputText {
            text: contents.to_string(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_mode_new_plan() {
        let data = AttachmentData::PlanMode {
            plan_file_path: "/tmp/plan.md".to_string(),
            is_subagent: false,
            plan_exists: false,
        };

        if let ResponseItem::Message { content, .. } = data.into() {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("Plan mode is active"));
                assert!(text.contains("/tmp/plan.md"));
                assert!(text.contains("create your plan"));
            } else {
                panic!("Expected InputText");
            }
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_plan_mode_existing_plan() {
        let data = AttachmentData::PlanMode {
            plan_file_path: "/tmp/plan.md".to_string(),
            is_subagent: false,
            plan_exists: true,
        };

        if let ResponseItem::Message { content, .. } = data.into() {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("already exists"));
                assert!(text.contains("incremental edits"));
            } else {
                panic!("Expected InputText");
            }
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_plan_mode_subagent() {
        let data = AttachmentData::PlanMode {
            plan_file_path: "/tmp/plan.md".to_string(),
            is_subagent: true,
            plan_exists: false,
        };

        if let ResponseItem::Message { content, .. } = data.into() {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("You are a subagent"));
            } else {
                panic!("Expected InputText");
            }
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_plan_mode_exit() {
        let data = AttachmentData::PlanModeExit {
            plan_file_path: Some("/tmp/plan.md".to_string()),
            next_mode: None,
        };

        if let ResponseItem::Message { content, .. } = data.into() {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("Exited Plan Mode"));
                assert!(text.contains("/tmp/plan.md"));
            } else {
                panic!("Expected InputText");
            }
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_plan_mode_exit_to_pair_programming() {
        let data = AttachmentData::PlanModeExit {
            plan_file_path: Some("/tmp/plan.md".to_string()),
            next_mode: Some(CollaborationMode::PairProgramming),
        };

        if let ResponseItem::Message { content, .. } = data.into() {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("Exited Plan Mode"));
                assert!(text.contains("/tmp/plan.md"));
                // Should contain pair programming instructions
                assert!(text.contains("Collaboration Style: Pair Programming"));
            } else {
                panic!("Expected InputText");
            }
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_plan_mode_exit_to_execute() {
        let data = AttachmentData::PlanModeExit {
            plan_file_path: Some("/tmp/plan.md".to_string()),
            next_mode: Some(CollaborationMode::Execute),
        };

        if let ResponseItem::Message { content, .. } = data.into() {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("Exited Plan Mode"));
                assert!(text.contains("/tmp/plan.md"));
                // Should contain execute instructions
                assert!(text.contains("Collaboration Style: Execute"));
            } else {
                panic!("Expected InputText");
            }
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_plan_mode_exit_to_plan_mode() {
        // Transitioning back to plan mode should not include extra instructions
        let data = AttachmentData::PlanModeExit {
            plan_file_path: Some("/tmp/plan.md".to_string()),
            next_mode: Some(CollaborationMode::Plan),
        };

        if let ResponseItem::Message { content, .. } = data.into() {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("Exited Plan Mode"));
                // Should NOT contain collaboration style instructions
                assert!(!text.contains("Collaboration Style:"));
            } else {
                panic!("Expected InputText");
            }
        } else {
            panic!("Expected Message");
        }
    }

    #[test]
    fn test_plan_mode_reentry() {
        let data = AttachmentData::PlanModeReentry {
            plan_file_path: "/tmp/plan.md".to_string(),
        };

        if let ResponseItem::Message { content, .. } = data.into() {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("Re-entering Plan Mode"));
                assert!(text.contains("/tmp/plan.md"));
            } else {
                panic!("Expected InputText");
            }
        } else {
            panic!("Expected Message");
        }
    }
}
