//! Attachment collector implementations.
//!
//! Collectors are functions that gather contextual data during turns.
//! Each collector returns a Vec<AttachmentData> which may be empty if
//! no attachment should be generated for the current context.

use std::path::Path;

use codex_protocol::attachment::AttachmentData;
use codex_protocol::attachment::MentionAttachment;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::models::ResponseItem;
use codex_protocol::user_input::UserInput;
use futures::future::BoxFuture;

use crate::codex::Session;
use crate::codex::TurnContext;
use crate::mention_extraction::extract_mentions;
use crate::mention_loading::MentionContent;
use crate::mention_loading::load_mention_contents;
use crate::plan_file::resolve_plan_file_path_with_slug;

/// Minimum turns between plan mode reminder attachments.
const TURNS_BETWEEN_ATTACHMENTS: usize = 5;

/// Collects plan mode attachment data.
///
/// Returns attachment if in plan mode (CollaborationMode::Plan) and enough turns
/// have passed since the last plan mode attachment.
pub(crate) fn collect_plan_mode<'a>(
    session: &'a Session,
    _turn: &'a TurnContext,
) -> BoxFuture<'a, Vec<AttachmentData>> {
    Box::pin(async move {
        // Check if in plan mode via CollaborationMode
        let collab_mode = session.collaboration_mode().await;
        if !matches!(collab_mode, CollaborationMode::Plan) {
            return vec![];
        }

        // Single lock acquisition to get all needed state
        let (slug, history) = session
            .with_state(|state| (state.plan_slug.clone(), state.clone_history()))
            .await;

        // Need plan slug to be set
        let Some(slug) = slug else {
            return vec![];
        };

        // Check throttle: skip if recent attachment exists
        let turns_since = count_turns_since_last_plan_attachment(history.raw_items());
        if turns_since < TURNS_BETWEEN_ATTACHMENTS {
            return vec![];
        }

        // Resolve slug to path
        let plan_path = resolve_plan_file_path_with_slug(&slug);

        vec![AttachmentData::PlanMode {
            plan_file_path: plan_path.to_string_lossy().into_owned(),
            plan_exists: plan_path.exists(),
        }]
    })
}

/// Collects plan mode exit attachment data.
///
/// Returns attachment if exiting plan mode (needs_plan_exit_attachment flag is set
/// and we're no longer in CollaborationMode::Plan).
pub(crate) fn collect_plan_mode_exit<'a>(
    session: &'a Session,
    _turn: &'a TurnContext,
) -> BoxFuture<'a, Vec<AttachmentData>> {
    Box::pin(async move {
        // Check if exit attachment is needed
        let needs_exit = session
            .with_state(|state| state.needs_plan_exit_attachment)
            .await;

        if !needs_exit {
            return vec![];
        }

        // Double-check: only emit if NOT in plan mode anymore
        let collab_mode = session.collaboration_mode().await;
        if matches!(collab_mode, CollaborationMode::Plan) {
            return vec![];
        }

        // Get slug and clear the one-shot flag (mutable access)
        let plan_file_path = session
            .with_state_mut(|state| {
                state.clear_plan_exit_flag();
                state.plan_slug.as_ref().map(|slug| {
                    resolve_plan_file_path_with_slug(slug)
                        .to_string_lossy()
                        .into_owned()
                })
            })
            .await;

        // Include the next mode so transition instructions can be injected
        vec![AttachmentData::PlanModeExit {
            plan_file_path,
            next_mode: Some(collab_mode),
        }]
    })
}

/// Collects file mention attachments from user input.
///
/// Extracts @ mentions from user input, loads file/directory content,
/// and returns attachment data that will be expanded before API calls.
pub(crate) async fn collect_file_mentions(
    input: &[UserInput],
    cwd: &Path,
) -> (Vec<AttachmentData>, Vec<String>) {
    let extraction_result = extract_mentions(input, cwd);

    // Return warnings for logging
    let warnings = extraction_result.warnings;

    if extraction_result.mentions.is_empty() {
        return (vec![], warnings);
    }

    // Load content for all mentions
    let contents = load_mention_contents(extraction_result.mentions).await;

    if contents.is_empty() {
        return (vec![], warnings);
    }

    // Convert to MentionAttachment
    let attachments: Vec<MentionAttachment> = contents
        .into_iter()
        .map(|content| match content {
            MentionContent::File {
                path,
                content,
                line_range,
                truncated,
            } => MentionAttachment::File {
                path,
                content,
                line_start: line_range.as_ref().map(|r| r.start),
                line_end: line_range.as_ref().and_then(|r| r.end),
                truncated,
            },
            MentionContent::Directory { path, listing } => {
                MentionAttachment::Directory { path, listing }
            }
        })
        .collect();

    (
        vec![AttachmentData::FileMentions {
            contents: attachments,
        }],
        warnings,
    )
}

/// Count assistant turns since last PlanMode/PlanModeReentry attachment.
fn count_turns_since_last_plan_attachment(items: &[ResponseItem]) -> usize {
    let mut turns = 0;
    for item in items.iter().rev() {
        match item {
            ResponseItem::Message { role, .. } if role == "assistant" => turns += 1,
            ResponseItem::Attachment { data } => match data {
                AttachmentData::PlanMode { .. } | AttachmentData::PlanModeReentry { .. } => {
                    return turns;
                }
                AttachmentData::PlanModeExit { .. } => return usize::MAX, // Reset on exit
                _ => {}
            },
            _ => {}
        }
    }
    usize::MAX // No previous attachment found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_turns_no_history() {
        let items = vec![];
        assert_eq!(count_turns_since_last_plan_attachment(&items), usize::MAX);
    }

    #[test]
    fn test_count_turns_no_attachments() {
        use codex_protocol::models::ContentItem;

        let items = vec![
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "response 1".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "response 2".to_string(),
                }],
            },
        ];
        assert_eq!(count_turns_since_last_plan_attachment(&items), usize::MAX);
    }

    #[test]
    fn test_count_turns_with_plan_attachment() {
        use codex_protocol::models::ContentItem;

        let items = vec![
            ResponseItem::Attachment {
                data: AttachmentData::PlanMode {
                    plan_file_path: "/tmp/plan.md".to_string(),
                    plan_exists: true,
                },
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "response 1".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "response 2".to_string(),
                }],
            },
        ];
        // Counting from the end: 2 assistant messages before the attachment
        assert_eq!(count_turns_since_last_plan_attachment(&items), 2);
    }

    #[test]
    fn test_count_turns_reset_after_exit() {
        use codex_protocol::models::ContentItem;

        let items = vec![
            ResponseItem::Attachment {
                data: AttachmentData::PlanMode {
                    plan_file_path: "/tmp/plan.md".to_string(),
                    plan_exists: true,
                },
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "response 1".to_string(),
                }],
            },
            ResponseItem::Attachment {
                data: AttachmentData::PlanModeExit {
                    plan_file_path: Some("/tmp/plan.md".to_string()),
                    next_mode: None,
                },
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "response 2".to_string(),
                }],
            },
        ];
        // The exit attachment resets the counter, so we return MAX
        assert_eq!(count_turns_since_last_plan_attachment(&items), usize::MAX);
    }
}
