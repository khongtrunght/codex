//! Attachment collector functions.

use futures::future::BoxFuture;

use super::types::Attachment;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::plan_file::{plan_exists_with_slug, resolve_plan_file_path_with_slug};

/// Collect plan mode attachments.
///
/// Returns plan mode instructions if session is in plan mode.
/// Includes reentry attachment if previously exited plan mode.
pub fn collect_plan_mode<'a>(
    session: &'a Session,
    _turn: &'a TurnContext,
) -> BoxFuture<'a, Vec<Attachment>> {
    Box::pin(async move {
        // Only collect if in plan mode
        if !session.is_in_plan_mode().await {
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
            attachments.push(Attachment::PlanModeReentry {
                plan_file_path: plan_file_path.clone(),
            });
        }

        // Main plan mode attachment
        attachments.push(Attachment::PlanMode {
            plan_file_path,
            is_subagent,
            plan_exists: plan_file_exists,
        });

        attachments
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
