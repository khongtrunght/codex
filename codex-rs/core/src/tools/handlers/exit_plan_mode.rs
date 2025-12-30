//! ExitPlanMode tool handler.
//!
//! Exits plan mode and presents the plan for user approval.
//! The plan is read from the plan file that was written during plan mode.

use async_trait::async_trait;
use codex_protocol::permission_mode::ExitedPlanModeEvent;
use codex_protocol::protocol::EventMsg;

use crate::function_tool::FunctionCallError;
use crate::plan_file::extract_plan_from_file;
use crate::plan_file::resolve_plan_file_path;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

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

        let session_id = session.conversation_id().to_string();
        let is_agent = session.source_session_id().is_some();

        // Get plan file path
        let plan_file_path = resolve_plan_file_path(&session_id, None);
        let path_str = plan_file_path.to_string_lossy().to_string();

        // Read plan from file (matches Claude Code's extractPlanFromContext)
        let plan_content = extract_plan_from_file(&session_id, None)
            .await
            .ok_or_else(|| {
                FunctionCallError::RespondToModel(format!(
                    "No plan file found at {}. Please write your plan to this file before calling ExitPlanMode.",
                    path_str
                ))
            })?;

        // Exit plan mode using unified API (also syncs legacy fields and sets has_exited flag)
        session.exit_plan_mode_unified().await;

        // Emit plan mode exited event for TUI
        session
            .send_event(
                &turn,
                EventMsg::ExitedPlanMode(ExitedPlanModeEvent {
                    plan: plan_content.clone(),
                    plan_file_path: path_str.clone(),
                }),
            )
            .await;

        // Build response with plan data
        // The actual approval message is generated after user approval
        let response = serde_json::json!({
            "plan": plan_content,
            "isAgent": is_agent,
            "filePath": path_str,
        });

        Ok(ToolOutput::Function {
            content: response.to_string(),
            content_items: None,
            success: Some(true),
        })
    }
}

/// Generate the tool result message after user approval.
/// Matches Claude Code's mapToolResultToToolResultBlockParam.
#[allow(dead_code)]
pub fn generate_approval_message(is_agent: bool, plan_content: &str, file_path: &str) -> String {
    if is_agent {
        r#"User has approved the plan. There is nothing else needed from you now. Please respond with "ok". Very important: remember, if you modify your plan along the way, you MUST edit the plan file to reflect the changes."#.to_string()
    } else {
        format!(
            "User has approved your plan. You can now start coding. Start with updating your todo list if applicable. Very important: remember, if you modify your plan along the way, you MUST edit the plan file to reflect the changes.\n\nYour plan has been saved to: {}\n\n## Approved Plan:\n{}",
            file_path, plan_content
        )
    }
}
