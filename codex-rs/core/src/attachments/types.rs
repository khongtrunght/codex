//! Attachment type definitions and expansion logic.
//!
//! All attachment variants are defined in codex_protocol::models::AttachmentData.
//! This module provides the conversion logic to expand attachments into messages.

use codex_protocol::models::AttachmentData;
use codex_protocol::models::CompactRestoredFile;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;

/// Convert AttachmentData to ResponseItem::Message items for API call.
pub(crate) fn expand_attachment(data: &AttachmentData) -> Vec<ResponseItem> {
    match data {
        AttachmentData::PlanMode {
            plan_file_path,
            is_subagent,
            plan_exists,
        } => generate_plan_mode_items(plan_file_path, *is_subagent, *plan_exists),
        AttachmentData::PlanModeReentry { plan_file_path } => {
            generate_reentry_items(plan_file_path)
        }
        AttachmentData::PlanModeExit { plan_file_path } => generate_exit_items(plan_file_path),
        AttachmentData::CompactFileRestore { files } => generate_compact_file_restore_items(files),
    }
}

/// Generate items for plan mode attachment.
fn generate_plan_mode_items(
    plan_file_path: &str,
    is_subagent: bool,
    plan_exists: bool,
) -> Vec<ResponseItem> {
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
        "<system-reminder>\nPlan mode is active.{subagent_note}\n\n## Plan File Info:\n{plan_file_info}\n</system-reminder>"
    );

    vec![wrap_in_user_message(&contents)]
}

/// Generate items for plan mode reentry.
fn generate_reentry_items(plan_file_path: &str) -> Vec<ResponseItem> {
    let contents = format!(
        "<system-reminder>\n## Re-entering Plan Mode\n\nYou are returning to plan mode. The plan file is at {plan_file_path}. Update it as needed.\n</system-reminder>"
    );
    vec![wrap_in_user_message(&contents)]
}

/// Generate items for plan mode exit.
fn generate_exit_items(plan_file_path: &Option<String>) -> Vec<ResponseItem> {
    let plan_file_info = match plan_file_path {
        Some(path) => {
            format!(" The plan file is located at {path} if you need to reference it.")
        }
        None => "No plan file was created during the planning session.".to_string(),
    };

    let contents = format!(
        "<system-reminder>\n## Exited Plan Mode\n\nYou have exited plan mode. You can now make edits, run tools, and take actions.{plan_file_info}\n</system-reminder>"
    );

    vec![wrap_in_user_message(&contents)]
}

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
                let truncate_note = if *truncated {
                    " truncated=\"true\""
                } else {
                    ""
                };
                let msg = format!(
                    "<file_context path=\"{path}\"{truncate_note}>\n{content}\n</file_context>"
                );
                items.push(wrap_in_user_message(&msg));
            }
            CompactRestoredFile::ReferenceOnly { path } => {
                let msg = format!(
                    "Note: {path} was read before the last conversation was summarized, \
                     but the contents are too large to include. Use read_file tool if you need to access it."
                );
                items.push(wrap_in_user_message(&msg));
            }
        }
    }

    items
}

/// Wrap content in a user message.
fn wrap_in_user_message(contents: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
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

        let expanded = expand_attachment(&data);
        assert_eq!(expanded.len(), 1);

        if let ResponseItem::Message { content, .. } = &expanded[0] {
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

        let expanded = expand_attachment(&data);

        if let ResponseItem::Message { content, .. } = &expanded[0] {
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

        let expanded = expand_attachment(&data);

        if let ResponseItem::Message { content, .. } = &expanded[0] {
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
        };

        let expanded = expand_attachment(&data);

        if let ResponseItem::Message { content, .. } = &expanded[0] {
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
    fn test_plan_mode_reentry() {
        let data = AttachmentData::PlanModeReentry {
            plan_file_path: "/tmp/plan.md".to_string(),
        };

        let expanded = expand_attachment(&data);

        if let ResponseItem::Message { content, .. } = &expanded[0] {
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
