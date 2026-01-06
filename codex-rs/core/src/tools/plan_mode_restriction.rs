//! Plan mode write restriction checking.
//!
//! In plan mode, only the plan file can be written to. All other write
//! operations should be blocked with an error message.

use crate::codex::Session;
use crate::tools::spec::EXIT_PLAN_MODE_TOOL_NAME;
use std::path::Path;

/// Check if a write operation is allowed in plan mode.
///
/// Returns `Ok(())` if the write is allowed, or `Err(message)` if blocked.
///
/// # Rules
/// - If not in plan mode: always allow
/// - If in plan mode and writing to plan file: allow
/// - If in plan mode and writing to other file: block
pub async fn check_plan_mode_write(session: &Session, file_path: &Path) -> Result<(), String> {
    // Not in plan mode - allow all writes
    if !session.is_in_plan_mode().await {
        return Ok(());
    }

    // In plan mode - check if this is the plan file
    if session.is_plan_file_path(file_path).await {
        return Ok(());
    }

    // Get plan file path for error message
    let plan_file = session
        .get_plan_file_path_unified()
        .await
        .unwrap_or_else(|| "<unknown>".to_string());

    Err(format!(
        "In Plan Mode, only the plan file ({}) can be modified. Attempted to modify: {}. \
        Use {EXIT_PLAN_MODE_TOOL_NAME} when ready to make code changes.",
        plan_file,
        file_path.display()
    ))
}
