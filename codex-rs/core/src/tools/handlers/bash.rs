//! Bash tool handler - executes shell commands with optional background mode.
//!
//! Foreground mode: Uses ShellHandler::run_exec_like() for synchronous execution
//! Background mode: Uses UnifiedExecSessionManager for PTY session with process ID

use async_trait::async_trait;
use codex_protocol::models::BashToolCallParams;
use std::sync::Arc;

use crate::codex::TurnContext;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::function_tool::FunctionCallError;
use crate::is_safe_command::is_known_safe_command;
use crate::sandboxing::SandboxPermissions;
use crate::tools::context::{ToolInvocation, ToolOutput, ToolPayload};
use crate::tools::registry::{ToolHandler, ToolKind};
use crate::unified_exec::{ExecCommandRequest, UnifiedExecContext};

use super::ShellHandler;

pub struct BashHandler;

impl BashHandler {
    /// Convert Bash params to ExecParams for foreground execution
    fn to_exec_params(
        params: &BashToolCallParams,
        session: &crate::codex::Session,
        turn_context: &TurnContext,
    ) -> ExecParams {
        let shell = session.user_shell();
        // Convert string command to shell args (e.g., ["bash", "-lc", "command"])
        let command = shell.derive_exec_args(&params.command, true);

        let sandbox_permissions = if params.dangerously_disable_sandbox {
            SandboxPermissions::RequireEscalated
        } else {
            SandboxPermissions::UseDefault
        };

        ExecParams {
            command,
            cwd: turn_context.cwd.clone(),
            expiration: params.timeout_ms.into(),
            env: create_env(&turn_context.shell_environment_policy),
            sandbox_permissions,
            justification: params.description.clone(),
            arg0: None,
        }
    }

    /// Execute command in background, returning process ID
    async fn run_background(
        params: &BashToolCallParams,
        session: Arc<crate::codex::Session>,
        turn: Arc<TurnContext>,
        call_id: String,
    ) -> Result<ToolOutput, FunctionCallError> {
        let shell = session.user_shell();
        let command = shell.derive_exec_args(&params.command, true);

        let sandbox_permissions = if params.dangerously_disable_sandbox {
            SandboxPermissions::RequireEscalated
        } else {
            SandboxPermissions::UseDefault
        };

        let manager = &session.services.unified_exec_manager;
        let process_id = manager.allocate_process_id().await;

        let context = UnifiedExecContext::new(
            Arc::clone(&session),
            Arc::clone(&turn),
            call_id,
        );

        // Use a short yield time for background - we return immediately
        let yield_time_ms = 500; // Brief wait to capture initial output

        let result = manager
            .exec_command(
                ExecCommandRequest {
                    command,
                    process_id: process_id.clone(),
                    yield_time_ms,
                    max_output_tokens: None,
                    workdir: Some(turn.cwd.clone()),
                    sandbox_permissions,
                    justification: params.description.clone(),
                },
                &context,
            )
            .await
            .map_err(|e| {
                FunctionCallError::RespondToModel(format!(
                    "Failed to start background command: {e:?}"
                ))
            })?;

        // Return the background task ID
        let response = if let Some(pid) = result.process_id {
            serde_json::json!({
                "backgroundTaskId": pid,
                "message": format!("Command started in background. Use BashOutput tool with bash_id=\"{}\" to check output.", pid)
            })
        } else {
            // Command completed before yield timeout
            serde_json::json!({
                "output": result.output,
                "exitCode": result.exit_code,
                "message": "Command completed quickly (no background task created)"
            })
        };

        Ok(ToolOutput::Function {
            content: serde_json::to_string_pretty(&response).unwrap_or_default(),
            content_items: None,
            success: Some(true),
        })
    }
}

#[async_trait]
impl ToolHandler for BashHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    async fn is_mutating(&self, invocation: &ToolInvocation) -> bool {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return true;
        };

        serde_json::from_str::<BashToolCallParams>(arguments)
            .map(|params| {
                let shell = invocation.session.user_shell();
                let command = shell.derive_exec_args(&params.command, true);
                !is_known_safe_command(&command)
            })
            .unwrap_or(true)
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            tool_name,
            payload,
        } = invocation;

        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "bash handler received unsupported payload".to_string(),
            ));
        };

        let params: BashToolCallParams = serde_json::from_str(&arguments).map_err(|e| {
            FunctionCallError::RespondToModel(format!(
                "failed to parse bash arguments: {e:?}"
            ))
        })?;

        if params.run_in_background {
            // Background mode: use UnifiedExecSessionManager
            Self::run_background(&params, session, turn, call_id).await
        } else {
            // Foreground mode: reuse ShellHandler::run_exec_like()
            let exec_params = Self::to_exec_params(&params, session.as_ref(), turn.as_ref());
            ShellHandler::run_exec_like(
                tool_name.as_str(),
                exec_params,
                session,
                turn,
                tracker,
                call_id,
                true, // freeform = true for bash commands
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bash_params_parsing_minimal() {
        let json = r#"{"command": "ls -la"}"#;
        let params: BashToolCallParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.command, "ls -la");
        assert!(!params.run_in_background);
        assert!(!params.dangerously_disable_sandbox);
    }

    #[test]
    fn test_bash_params_parsing_background() {
        let json = r#"{"command": "npm install", "run_in_background": true, "description": "Install deps"}"#;
        let params: BashToolCallParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.command, "npm install");
        assert!(params.run_in_background);
        assert_eq!(params.description, Some("Install deps".to_string()));
    }

    #[test]
    fn test_bash_params_parsing_with_timeout() {
        let json = r#"{"command": "sleep 5", "timeout_ms": 10000}"#;
        let params: BashToolCallParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.timeout_ms, Some(10000));
    }
}
