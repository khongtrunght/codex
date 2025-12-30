//! EnterPlanMode tool handler.
//!
//! Enters plan mode for complex tasks requiring exploration and design.
//! Plan mode allows the model to explore the codebase and design an implementation
//! approach before making any changes.

use async_trait::async_trait;
use codex_protocol::permission_mode::EnteredPlanModeEvent;
use codex_protocol::protocol::EventMsg;

use crate::function_tool::FunctionCallError;
use crate::plan_file::resolve_plan_file_path;
use crate::plan_mode_attachment::generate_plan_mode_attachment;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

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

        let session_id = session.conversation_id().to_string();

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

        // Resolve plan file path
        let plan_file_path = resolve_plan_file_path(&session_id, None);
        let path_str = plan_file_path.to_string_lossy().to_string();

        // Enter plan mode using unified API (also syncs with legacy fields)
        session.enter_plan_mode_unified(path_str.clone()).await;

        // Generate and record plan mode instructions
        let plan_mode_instructions = generate_plan_mode_attachment(
            &session_id,
            &path_str,
            turn.tools_config.edit_tool_type.clone(),
            turn.tools_config.shell_type.clone(),
        );

        // Record the plan mode instructions as a conversation item
        // This will be included in subsequent turns for the model
        let plan_mode_item = codex_protocol::models::ResponseItem::from(plan_mode_instructions);
        session
            .record_conversation_items(&turn, &[plan_mode_item])
            .await;

        // Emit plan mode entered event for TUI
        session
            .send_event(
                &turn,
                EventMsg::EnteredPlanMode(EnteredPlanModeEvent {
                    plan_file_path: path_str.clone(),
                }),
            )
            .await;

        // Match Claude Code's exact message
        let message = format!(
            "Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach.\n\nYour plan file is at: {}",
            path_str
        );

        Ok(ToolOutput::Function {
            content: message,
            content_items: None,
            success: Some(true),
        })
    }
}
