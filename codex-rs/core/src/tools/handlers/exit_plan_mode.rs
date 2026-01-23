//! ExitPlanMode tool handler.
//!
//! Exits plan mode and presents the plan for user approval.
//! The plan is read from the plan file that was written during plan mode.
//! User will choose the target mode (PairProgramming or Execute) when approving.

use askama::Template;
use async_trait::async_trait;
use codex_protocol::config_types::CollaborationMode;

use crate::function_tool::FunctionCallError;
use crate::plan_file::extract_plan_from_file_with_slug;
use crate::plan_file::resolve_plan_file_path_with_slug;
use crate::prompt_template::ExitPlanModeNoFileError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::EXIT_PLAN_MODE_TOOL_NAME;

pub struct ExitPlanModeHandler;

#[async_trait]
impl ToolHandler for ExitPlanModeHandler {
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

        // Validate payload is Function type (even though args are empty)
        match payload {
            ToolPayload::Function { .. } => {}
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "exit_plan_mode handler received unsupported payload".to_string(),
                ));
            }
        }

        // Check if this is a subagent (affects response message)
        let is_subagent = session.is_plan_subagent().await;

        // Get slug from session state (was set when entering plan mode)
        let slug = session.get_plan_slug().await.ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "Cannot exit plan mode: no plan slug found. Was plan mode entered?".to_string(),
            )
        })?;

        // Resolve plan file path using the slug
        let plan_file_path = resolve_plan_file_path_with_slug(&slug, None);
        let path_str = plan_file_path.to_string_lossy().to_string();

        // Read plan from file
        let plan_content = extract_plan_from_file_with_slug(&slug, None)
            .await
            .ok_or_else(|| {
                let error_template = ExitPlanModeNoFileError {
                    tools: turn.tools_config.clone(),
                    path_str: path_str.clone(),
                };
                let error_msg = error_template
                    .render()
                    .unwrap_or_else(|_| format!("No plan file found at {path_str}"));
                FunctionCallError::RespondToModel(error_msg)
            })?;

        // Request approval from user (they will choose target mode)
        let response = session
            .request_exit_plan_mode_approval(&turn, call_id, plan_content.clone(), plan_file_path)
            .await;

        // Handle rejection
        if !response.approved {
            return Err(FunctionCallError::RespondToModel(format!(
                "{EXIT_PLAN_MODE_TOOL_NAME} was rejected by the user."
            )));
        }

        // Get target mode from the response (TUI should always provide this)
        let target_mode = response.target_mode.ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "Exit plan mode was approved but no target mode was specified.".to_string(),
            )
        })?;

        // Complete exit from plan mode
        session
            .complete_exit_plan_mode(
                &turn,
                target_mode.clone(),
                path_str.clone(),
                plan_content.clone(),
            )
            .await;

        // Build response with plan data
        let response_msg =
            generate_approval_message(is_subagent, &target_mode, &plan_content, &path_str);
        Ok(ToolOutput::Function {
            content: response_msg,
            content_items: None,
            success: Some(true),
        })
    }
}

/// Generate the tool result message after user approval.
pub fn generate_approval_message(
    is_subagent: bool,
    target_mode: &CollaborationMode,
    plan_content: &str,
    file_path: &str,
) -> String {
    if is_subagent {
        // Subagents should just acknowledge and let the parent handle things
        r#"User has approved the plan. There is nothing else needed from you now. Please respond with "ok""#.to_string()
    } else {
        let mode_name = match target_mode {
            CollaborationMode::PairProgramming => "pair programming",
            CollaborationMode::Execute => "execute",
            CollaborationMode::Plan => "plan",
        };
        format!(
            r#"User has approved your plan and selected {mode_name} mode. You can now start coding. Start with updating your todo list if applicable

Your plan has been saved to: {file_path}
You can refer back to it if needed during implementation.

## Approved Plan:
{plan_content}"#
        )
    }
}
