//! ExitPlanMode tool handler.
//!
//! Exits plan mode and presents the plan for user approval.
//! The plan is read from the plan file that was written during plan mode.

use async_trait::async_trait;
use codex_protocol::protocol::EventMsg;
use codex_protocol::session_mode::ExitedPlanModeEvent;

use crate::function_tool::FunctionCallError;
use crate::plan_file::{extract_plan_from_file_with_slug, resolve_plan_file_path_with_slug};
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::orchestrator::ToolOrchestrator;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::runtimes::plan_mode::ExitPlanModeRequest;
use crate::tools::runtimes::plan_mode::ExitPlanModeRuntime;
use crate::tools::sandboxing::ToolCtx;
use crate::tools::sandboxing::ToolError;
use crate::tools::spec::ApplyToolConfig;
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
            tool_name,
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
                let error_msg = format!(
                    "No plan file found at {path_str}. Please write your plan to this file before calling {{exit_plan_mode_tool}}."
                ).apply_tool_config(
                    turn.tools_config.edit_tool_type,
                    turn.tools_config.shell_type,
                );
                FunctionCallError::RespondToModel(error_msg)
            })?;

        // Create request with plan content for approval UI display
        let req = ExitPlanModeRequest {
            session_id: session_id.clone(),
            plan_content: Some(plan_content.clone()),
            plan_file_path: plan_file_path.clone(),
        };

        let mut orchestrator = ToolOrchestrator::new();
        let mut runtime = ExitPlanModeRuntime::new();
        let tool_ctx = ToolCtx {
            session: session.as_ref(),
            turn: turn.as_ref(),
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
        };

        // Run through orchestrator - this handles approval via Approvable trait
        let result = orchestrator
            .run(&mut runtime, &req, &tool_ctx, &turn, turn.approval_policy)
            .await;

        // Handle rejection
        match result {
            Ok(_) => {}
            Err(ToolError::Rejected(reason)) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "{EXIT_PLAN_MODE_TOOL_NAME} was rejected: {reason}"
                )));
            }
            Err(ToolError::Codex(e)) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "{EXIT_PLAN_MODE_TOOL_NAME} failed: {e}"
                )));
            }
        }

        // Approval granted - proceed with exiting plan mode

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
        let response = generate_approval_message(is_agent, &plan_content, &path_str);
        Ok(ToolOutput::Function {
            content: response,
            content_items: None,
            success: Some(true),
        })
    }
}

/// Generate the tool result message after user approval.
pub fn generate_approval_message(is_agent: bool, plan_content: &str, file_path: &str) -> String {
    if is_agent {
        r#"User has approved the plan. There is nothing else needed from you now. Please respond with "ok""#.to_string()
    } else {
        format!(
            r#"User has approved your plan. You can now start coding. Start with updating your todo list if applicable

Your plan has been saved to: {file_path}
You can refer back to it if needed during implementation.

## Approved Plan:
{plan_content}"#
        )
    }
}
