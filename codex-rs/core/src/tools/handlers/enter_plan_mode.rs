//! EnterPlanMode tool handler.
//!
//! Enters plan mode for complex tasks requiring exploration and design.
//! Plan mode allows the model to explore the codebase and design an implementation
//! approach before making any changes.
//!
//! Note: Plan mode instructions are injected automatically at the start of each
//! task when in plan mode (see run_task in codex.rs). This handler only sets
//! the mode and emits events.

use async_trait::async_trait;
use codex_protocol::protocol::EventMsg;
use codex_protocol::session_mode::EnteredPlanModeEvent;

use crate::function_tool::FunctionCallError;
use crate::plan_file::{generate_unique_slug, resolve_plan_file_path_with_slug};
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::orchestrator::ToolOrchestrator;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::runtimes::plan_mode::EnterPlanModeRequest;
use crate::tools::runtimes::plan_mode::EnterPlanModeRuntime;
use crate::tools::sandboxing::ToolCtx;
use crate::tools::sandboxing::ToolError;

pub struct EnterPlanModeHandler;

#[async_trait]
impl ToolHandler for EnterPlanModeHandler {
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
                    "enter_plan_mode handler received unsupported payload".to_string(),
                ));
            }
        }

        // Check if this is being called from a sub-agent context
        // Sub-agents have a source_session_id that differs from their conversation_id
        if session.source_session_id().is_some() {
            return Err(FunctionCallError::RespondToModel(
                "EnterPlanMode tool cannot be used in agent contexts. Plan mode is only available for the main session.".to_string()
            ));
        }

        // Check if already in plan mode (using unified API)
        if session.is_in_plan_mode().await {
            return Err(FunctionCallError::RespondToModel(
                "Already in plan mode. Use ExitPlanMode when ready.".to_string(),
            ));
        }

        // Get or create slug from session state (persisted across plan mode entries)
        let slug = session
            .get_or_create_plan_slug(generate_unique_slug)
            .await;

        // Resolve plan file path using the slug
        let plan_file_path = resolve_plan_file_path_with_slug(&slug, None);
        let path_str = plan_file_path.to_string_lossy().to_string();
        let session_id = session.conversation_id().to_string();

        // Create request and run through orchestrator for approval
        let req = EnterPlanModeRequest {
            session_id: session_id.clone(),
            plan_file_path: plan_file_path.clone(),
        };

        let mut orchestrator = ToolOrchestrator::new();
        let mut runtime = EnterPlanModeRuntime::new();
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
                    "EnterPlanMode was rejected: {reason}"
                )));
            }
            Err(ToolError::Codex(e)) => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "EnterPlanMode failed: {e}"
                )));
            }
        }

        // Approval granted - proceed with entering plan mode
        // (path_str was already resolved before approval request)

        // Enter plan mode using unified API (also syncs with legacy fields)
        session.enter_plan_mode_unified(path_str.clone()).await;

        // Note: Plan mode instructions are NOT injected here.
        // They are injected automatically at the start of the next task
        // (see run_task in codex.rs) to avoid double injection.

        // Emit plan mode entered event for TUI (includes slug for resume)
        session
            .send_event(
                &turn,
                EventMsg::EnteredPlanMode(EnteredPlanModeEvent {
                    plan_file_path: path_str.clone(),
                    plan_slug: Some(slug),
                }),
            )
            .await;

        let message = format!(
            "Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach.\n\nYour plan file is at: {path_str}",
        );

        Ok(ToolOutput::Function {
            content: message,
            content_items: None,
            success: Some(true),
        })
    }
}
