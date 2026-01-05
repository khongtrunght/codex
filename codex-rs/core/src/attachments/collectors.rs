//! Attachment collector functions.

use codex_protocol::models::AttachmentData;
use codex_protocol::models::ResponseItem;
use futures::future::BoxFuture;

use super::types::TURNS_BETWEEN_ATTACHMENTS;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::plan_file::{plan_exists_with_slug, resolve_plan_file_path_with_slug};

/// Analyzes history to find turns since last attachment of given types.
/// Returns (turns_since_attachment, found_previous_attachment).
///
/// Walks backwards through history counting assistant turns until finding
/// a plan_mode or plan_mode_reentry attachment.
///
/// Reset on exit: If a plan_mode_exit is encountered before plan_mode/plan_mode_reentry,
/// the throttle is reset (returns found=false) so re-entry always gets fresh plan_mode.
pub fn analyze_history_for_throttle(items: &[ResponseItem]) -> (usize, bool) {
    let mut turn_count = 0;
    let mut found = false;

    // Walk backwards through history (newest to oldest)
    for item in items.iter().rev() {
        match item {
            // Count assistant messages as turns
            ResponseItem::Message { role, .. } if role == "assistant" => {
                turn_count += 1;
            }
            // Check for previous plan_mode or plan_mode_reentry attachment
            ResponseItem::Attachment { data, .. } => match data {
                AttachmentData::PlanMode { .. } | AttachmentData::PlanModeReentry { .. } => {
                    found = true;
                    break;
                }
                AttachmentData::PlanModeExit { .. } => {
                    // Exit found before any plan_mode - treat as no previous attachment
                    found = false;
                    break;
                }
            }
            _ => {}
        }
    }

    (turn_count, found)
}

/// Collect plan mode attachments with throttling.
///
/// Returns plan mode instructions if session is in plan mode AND:
/// - No previous attachment exists, OR
/// - Enough turns have passed since last attachment (TURNS_BETWEEN_ATTACHMENTS)
///
/// Includes reentry attachment if previously exited plan mode.
pub fn collect_plan_mode<'a>(
    session: &'a Session,
    _turn: &'a TurnContext,
) -> BoxFuture<'a, Vec<AttachmentData>> {
    Box::pin(async move {
        // Only collect if in plan mode
        if !session.is_in_plan_mode().await {
            return vec![];
        }

        // Get history for throttle analysis (in-memory, fast)
        let history = session.clone_history().await;
        let items = history.contents();

        // Check throttle: skip if found previous AND not enough turns passed
        let (turns_since, found_previous) = analyze_history_for_throttle(&items);
        if found_previous && turns_since < TURNS_BETWEEN_ATTACHMENTS {
            return vec![];
        }

        // Get slug from session state
        let slug = match session.get_plan_slug().await {
            Some(s) => s,
            None => {
                // Fallback: session is in plan mode but no slug set
                // This shouldn't happen in normal flow
                return vec![];
            }
        };

        // Resolve plan file path using slug
        let plan_file_path = session
            .get_plan_file_path_unified()
            .await
            .unwrap_or_else(|| {
                resolve_plan_file_path_with_slug(&slug, None)
                    .to_string_lossy()
                    .into_owned()
            });

        let plan_file_exists = plan_exists_with_slug(&slug, None);
        let is_subagent = session.source_session_id().is_some();
        let has_exited = session.has_exited_plan_mode().await;

        let mut attachments = vec![];

        // Reentry detection: previously exited AND plan file exists
        if has_exited && plan_file_exists {
            attachments.push(AttachmentData::PlanModeReentry {
                plan_file_path: plan_file_path.clone(),
            });
        }

        // Main plan mode attachment
        attachments.push(AttachmentData::PlanMode {
            plan_file_path,
            is_subagent,
            plan_exists: plan_file_exists,
        });

        attachments
    })
}

/// Collect plan mode exit attachment.
///
/// Returns a plan_mode_exit attachment ONCE when the user exits plan mode via UI (shift+tab).
/// This notifies Claude that it is no longer in plan mode.
///
/// Note: The flag is cleared by the caller (codex.rs) after collecting.
pub fn collect_plan_mode_exit<'a>(
    session: &'a Session,
    _turn: &'a TurnContext,
) -> BoxFuture<'a, Vec<AttachmentData>> {
    Box::pin(async move {
        // Check if exit attachment is needed
        if !session.needs_plan_mode_exit_attachment().await {
            return vec![];
        }

        // Double-check: if we're back in plan mode, don't send exit notification
        if session.is_in_plan_mode().await {
            return vec![];
        }

        // Get slug from session state
        let slug = match session.get_plan_slug().await {
            Some(s) => s,
            None => return vec![],
        };

        // Resolve plan file path using slug
        let plan_file_path = session
            .get_plan_file_path_unified()
            .await
            .unwrap_or_else(|| {
                resolve_plan_file_path_with_slug(&slug, None)
                    .to_string_lossy()
                    .into_owned()
            });

        let plan_file_exists = plan_exists_with_slug(&slug, None);

        vec![AttachmentData::PlanModeExit {
            plan_file_path,
            plan_exists: plan_file_exists,
        }]
    })
}

// Future collectors can be added here:
//
// pub fn collect_todo<'a>(
//     session: &'a Session,
//     turn: &'a TurnContext,
// ) -> BoxFuture<'a, Vec<Attachment>> {
//     Box::pin(async move {
//         // ...
//     })
// }
