//! Types for the AskUserQuestion tool.
//!
//! Asks the user multiple choice questions to gather information, clarify
//! ambiguity, understand preferences, or make decisions.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

/// A single option for a question in the AskUserQuestion tool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct AskUserQuestionOption {
    /// Display text for this option (1-5 words).
    pub label: String,
    /// Explanation of what this option means or what will happen if chosen.
    pub description: String,
}

/// A single question in the AskUserQuestion tool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct AskUserQuestion {
    /// The complete question to ask the user.
    pub question: String,
    /// Very short label displayed as a chip/tag (max 12 chars).
    pub header: String,
    /// Available choices for this question (2-4 options).
    pub options: Vec<AskUserQuestionOption>,
    /// Whether multiple answers can be selected.
    #[serde(default)]
    pub multi_select: bool,
}

/// Optional metadata for tracking purposes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct AskUserQuestionMetadata {
    /// Source identifier (e.g., "remember" for /remember command).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Input arguments for the AskUserQuestion tool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct AskUserQuestionArgs {
    /// Questions to ask the user (1-4 questions).
    pub questions: Vec<AskUserQuestion>,
    /// User answers collected by the permission component (optional, populated on resolution).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<HashMap<String, String>>,
    /// Optional metadata for tracking purposes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AskUserQuestionMetadata>,
}

/// Request event for asking the user questions.
/// Sent when the agent needs user input via the AskUserQuestion tool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
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
pub struct AskUserQuestionResponse {
    /// Map of question text to answer string.
    /// For multi-select questions, answers are comma-separated.
    pub answers: HashMap<String, String>,
    /// Whether the user cancelled the question dialog.
    #[serde(default)]
    pub cancelled: bool,
}
