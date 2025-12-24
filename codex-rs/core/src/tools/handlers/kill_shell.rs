//! KillShell tool handler - terminates background shells.

use async_trait::async_trait;
use serde::Deserialize;

use crate::function_tool::FunctionCallError;
use crate::tools::context::{ToolInvocation, ToolOutput, ToolPayload};
use crate::tools::registry::{ToolHandler, ToolKind};

pub struct KillShellHandler;

#[derive(Debug, Deserialize)]
struct KillShellArgs {
    shell_id: String,
}

#[async_trait]
impl ToolHandler for KillShellHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation { session, payload, .. } = invocation;

        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "kill_shell handler received unsupported payload".to_string(),
            ));
        };

        let args: KillShellArgs = serde_json::from_str(&arguments).map_err(|e| {
            FunctionCallError::RespondToModel(format!("failed to parse arguments: {e:?}"))
        })?;

        let manager = &session.services.unified_exec_manager;
        let result = manager.terminate_session(&args.shell_id).await.map_err(|e| {
            FunctionCallError::RespondToModel(format!(
                "No shell found with ID: {}. {e:?}",
                args.shell_id
            ))
        })?;

        let message = if result.was_running {
            format!("Successfully killed shell: {}", args.shell_id)
        } else {
            format!("Shell {} was already terminated", args.shell_id)
        };

        let response = serde_json::json!({
            "message": message,
            "shell_id": result.process_id,
            "command": result.command.join(" "),
            "was_running": result.was_running,
        });

        Ok(ToolOutput::Function {
            content: serde_json::to_string_pretty(&response).unwrap_or_default(),
            content_items: None,
            success: Some(true),
        })
    }
}
