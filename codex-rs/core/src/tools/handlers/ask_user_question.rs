//! AskUserQuestion tool handler.
//!
//! Asks the user multiple choice questions to gather information, clarify
//! ambiguity, understand preferences, or make decisions.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde::Serialize;

use codex_protocol::protocol::AskUserQuestion;
use codex_protocol::protocol::AskUserQuestionOption;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

/// Input schema for the AskUserQuestion tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AskUserQuestionInput {
    /// Questions to ask the user (1-4 questions).
    pub questions: Vec<QuestionInput>,
    /// User answers collected by the permission component (optional, populated on resolution).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<HashMap<String, String>>,
}

/// A single question in the input.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QuestionInput {
    /// The complete question to ask the user.
    pub question: String,
    /// Very short label displayed as a chip/tag (max 12 chars).
    pub header: String,
    /// Available choices for this question (2-4 options).
    pub options: Vec<OptionInput>,
    /// Whether multiple answers can be selected.
    #[serde(default)]
    pub multi_select: bool,
}

/// A single option for a question.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OptionInput {
    /// Display text for this option (1-5 words).
    pub label: String,
    /// Explanation of what this option means.
    pub description: String,
}

impl From<&QuestionInput> for AskUserQuestion {
    fn from(q: &QuestionInput) -> Self {
        AskUserQuestion {
            question: q.question.clone(),
            header: q.header.clone(),
            options: q
                .options
                .iter()
                .map(|o| AskUserQuestionOption {
                    label: o.label.clone(),
                    description: o.description.clone(),
                })
                .collect(),
            multi_select: q.multi_select,
        }
    }
}

pub struct AskUserQuestionHandler;

#[async_trait]
impl ToolHandler for AskUserQuestionHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;

        // Parse input from payload
        let input: AskUserQuestionInput = match payload {
            ToolPayload::Function { arguments } => {
                serde_json::from_str(&arguments).map_err(|e| {
                    FunctionCallError::RespondToModel(format!(
                        "Failed to parse AskUserQuestion input: {e}"
                    ))
                })?
            }
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "ask_user_question handler received unsupported payload".to_string(),
                ));
            }
        };

        // Validate input
        if input.questions.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "At least one question is required".to_string(),
            ));
        }
        if input.questions.len() > 4 {
            return Err(FunctionCallError::RespondToModel(
                "Maximum of 4 questions allowed".to_string(),
            ));
        }
        for q in &input.questions {
            if q.options.len() < 2 {
                return Err(FunctionCallError::RespondToModel(format!(
                    "Question '{}' must have at least 2 options",
                    q.question
                )));
            }
            if q.options.len() > 4 {
                return Err(FunctionCallError::RespondToModel(format!(
                    "Question '{}' can have at most 4 options",
                    q.question
                )));
            }
        }

        // Convert to protocol types
        let questions: Vec<AskUserQuestion> = input.questions.iter().map(Into::into).collect();

        // Request user answers via session
        let response = session
            .ask_user_question(&turn, call_id.clone(), questions.clone())
            .await;

        // Handle cancellation - send instructive message to model
        if response.cancelled {
            return Err(FunctionCallError::RespondToModel(
                "The user doesn't want to take this action right now. STOP what you are doing and wait for the user to tell you how to proceed.".to_string(),
            ));
        }

        let question_answers: String = response
            .answers
            .iter()
            .map(|(q, a)| format!(r#""{q}"="{a}""#))
            .collect::<Vec<String>>()
            .join(", ");
        let output = format!(
            "User has answered your questions: {question_answers}. You can now continue with the user's answers in mind."
        );

        Ok(ToolOutput::Function {
            content: output,
            content_items: None,
            success: Some(true),
        })
    }
}
