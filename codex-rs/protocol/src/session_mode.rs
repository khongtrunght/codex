//! Session Mode types for plan mode transitions.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

/// Event emitted when plan mode is exited.
/// Session automatically transitions to Execute mode.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExitedPlanModeEvent {
    /// The plan content.
    pub plan: String,
    /// Path to the plan file.
    pub plan_file_path: String,
}
