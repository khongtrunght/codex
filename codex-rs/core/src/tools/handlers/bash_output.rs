//! BashOutput tool handler - retrieves output from background shells.

use async_trait::async_trait;
use serde::Deserialize;

use crate::function_tool::FunctionCallError;
use crate::tools::context::{ToolInvocation, ToolOutput, ToolPayload};
use crate::tools::registry::{ToolHandler, ToolKind};
use crate::unified_exec::SessionStatusInfo;

pub struct BashOutputHandler;

#[derive(Debug, Deserialize)]
struct BashOutputArgs {
    bash_id: String,
    #[serde(default)]
    filter: Option<String>,
}

#[async_trait]
impl ToolHandler for BashOutputHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        false
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation { session, payload, .. } = invocation;

        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "bash_output handler received unsupported payload".to_string(),
            ));
        };

        let args: BashOutputArgs = serde_json::from_str(&arguments).map_err(|e| {
            FunctionCallError::RespondToModel(format!("failed to parse arguments: {e:?}"))
        })?;

        let manager = &session.services.unified_exec_manager;
        let snapshot = manager.get_session_output(&args.bash_id).await.map_err(|e| {
            FunctionCallError::RespondToModel(format!(
                "No shell found with ID: {}. {e:?}",
                args.bash_id
            ))
        })?;

        // Apply optional regex filter
        let output = if let Some(pattern) = &args.filter {
            let regex = regex::Regex::new(pattern).map_err(|e| {
                FunctionCallError::RespondToModel(format!("Invalid regex: {e}"))
            })?;
            snapshot.output.lines()
                .filter(|line| regex.is_match(line))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            snapshot.output
        };

        let status_str = match snapshot.status {
            SessionStatusInfo::Running => "running",
            SessionStatusInfo::Exited if snapshot.exit_code == Some(0) => "completed",
            SessionStatusInfo::Exited => "failed",
        };

        let response = serde_json::json!({
            "shellId": snapshot.process_id,
            "command": snapshot.command.join(" "),
            "status": status_str,
            "exitCode": snapshot.exit_code,
            "stdout": output,
            "stderr": "",
            "stdoutLines": output.lines().count(),
            "stderrLines": 0,
            "filterPattern": args.filter,
        });

        Ok(ToolOutput::Function {
            content: serde_json::to_string_pretty(&response).unwrap_or_default(),
            content_items: None,
            success: Some(true),
        })
    }
}
