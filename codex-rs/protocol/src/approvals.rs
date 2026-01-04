use std::collections::HashMap;
use std::path::PathBuf;

use crate::parse_command::ParsedCommand;
use crate::protocol::FileChange;
use mcp_types::RequestId;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

/// Proposed execpolicy change to allow commands starting with this prefix.
///
/// The `command` tokens form the prefix that would be added as an execpolicy
/// `prefix_rule(..., decision="allow")`, letting the agent bypass approval for
/// commands that start with this token sequence.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(transparent)]
#[ts(type = "Array<string>")]
pub struct ExecPolicyAmendment {
    pub command: Vec<String>,
}

impl ExecPolicyAmendment {
    pub fn new(command: Vec<String>) -> Self {
        Self { command }
    }

    pub fn command(&self) -> &[String] {
        &self.command
    }
}

impl From<Vec<String>> for ExecPolicyAmendment {
    fn from(command: Vec<String>) -> Self {
        Self { command }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ExecApprovalRequestEvent {
    /// Identifier for the associated exec call, if available.
    pub call_id: String,
    /// Turn ID that this command belongs to.
    /// Uses `#[serde(default)]` for backwards compatibility.
    #[serde(default)]
    pub turn_id: String,
    /// The command to be executed.
    pub command: Vec<String>,
    /// The command's working directory.
    pub cwd: PathBuf,
    /// Optional human-readable reason for the approval (e.g. retry without sandbox).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Proposed execpolicy amendment that can be applied to allow future runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    pub parsed_cmd: Vec<ParsedCommand>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ElicitationRequestEvent {
    pub server_name: String,
    pub id: RequestId,
    pub message: String,
    // TODO: MCP servers can request we fill out a schema for the elicitation. We don't support
    // this yet.
    // pub requested_schema: ElicitRequestParamsRequestedSchema,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
pub enum ElicitationAction {
    Accept,
    Decline,
    Cancel,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApplyPatchApprovalRequestEvent {
    /// Responses API call id for the associated patch apply call, if available.
    pub call_id: String,
    /// Turn ID that this patch belongs to.
    /// Uses `#[serde(default)]` for backwards compatibility with older senders.
    #[serde(default)]
    pub turn_id: String,
    pub changes: HashMap<PathBuf, FileChange>,
    /// Optional explanatory reason (e.g. request for extra write access).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When set, the agent is asking the user to allow writes under this root for the remainder of the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_root: Option<PathBuf>,
}

/// Approval request event for entering plan mode.
/// Sent before transitioning to plan mode to get user confirmation.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct EnterPlanModeApprovalRequestEvent {
    /// Identifier for the associated tool call.
    pub call_id: String,
    /// Turn ID that this tool call belongs to.
    #[serde(default)]
    pub turn_id: String,
    /// Path where the plan file will be created.
    pub plan_file_path: PathBuf,
}

/// Approval request event for exiting plan mode.
/// Sent before transitioning out of plan mode to get user approval of the plan.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExitPlanModeApprovalRequestEvent {
    /// Identifier for the associated tool call.
    pub call_id: String,
    /// Turn ID that this tool call belongs to.
    #[serde(default)]
    pub turn_id: String,
    /// The plan content for user review.
    pub plan: String,
    /// Path to the plan file.
    pub plan_file_path: PathBuf,
}

// ============================================================================
// AskUserQuestion types
// ============================================================================

/// A single option for a question in the AskUserQuestion tool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AskUserQuestionOption {
    /// Display text for this option (1-5 words).
    pub label: String,
    /// Explanation of what this option means or what will happen if chosen.
    pub description: String,
}

/// A single question in the AskUserQuestion tool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AskUserQuestion {
    /// The complete question to ask the user.
    pub question: String,
    /// Very short label displayed as a chip/tag (max 12 chars).
    pub header: String,
    /// Available choices for this question (2-4 options).
    pub options: Vec<AskUserQuestionOption>,
    /// Whether multiple answers can be selected.
    pub multi_select: bool,
}

/// Request event for asking the user questions.
/// Sent when the agent needs user input via the AskUserQuestion tool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AskUserQuestionRequestEvent {
    /// Identifier for the associated tool call.
    pub call_id: String,
    /// Turn ID that this tool call belongs to.
    #[serde(default)]
    pub turn_id: String,
    /// The questions to ask the user (1-4 questions).
    pub questions: Vec<AskUserQuestion>,
}

/// User's response to an AskUserQuestion request.
/// Maps question text to the selected answer(s).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AskUserQuestionResponse {
    /// Map of question text to answer string.
    /// For multi-select questions, answers are comma-separated.
    pub answers: HashMap<String, String>,
    /// Whether the user cancelled the question dialog.
    #[serde(default)]
    pub cancelled: bool,
}
