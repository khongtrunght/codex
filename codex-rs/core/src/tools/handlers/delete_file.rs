use async_trait::async_trait;
use serde::Deserialize;
use tokio::fs;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct DeleteFileHandler;

/// JSON arguments accepted by the `delete_file` tool handler.
#[derive(Deserialize)]
struct DeleteFileArgs {
    /// Absolute path to the file that will be deleted.
    file_path: String,
}

#[async_trait]
impl ToolHandler for DeleteFileHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            payload,
            session,
            call_id,
            turn,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "delete_file handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: DeleteFileArgs = serde_json::from_str(&arguments).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to parse function arguments: {err:?}"
            ))
        })?;

        let DeleteFileArgs { file_path } = args;
        // Resolve relative paths against cwd
        let path = turn.resolve_path(Some(file_path.clone()));

        // Read file content before deletion for the begin event
        let file_content = fs::read_to_string(&path).await.unwrap_or_default();

        // Emit FileEditBeginEvent
        {
            use codex_protocol::protocol::Event;
            use codex_protocol::protocol::EventMsg;
            use codex_protocol::protocol::FileChange;
            use codex_protocol::protocol::FileEditBeginEvent;
            use std::collections::HashMap;

            let mut changes = HashMap::new();
            changes.insert(
                path.clone(),
                FileChange::Delete {
                    content: file_content,
                },
            );

            let begin_event = Event {
                id: call_id.clone(),
                msg: EventMsg::FileEditBegin(FileEditBeginEvent {
                    call_id: call_id.clone(),
                    auto_approved: true,
                    changes,
                }),
            };
            session.send_event(begin_event).await;
        }

        // Execute the actual file deletion
        let result = async {
            // Check if the file exists
            if !path.exists() {
                return Err(FunctionCallError::RespondToModel(format!(
                    "file does not exist: {file_path}"
                )));
            }

            // Check if it's a file (not a directory)
            let metadata = fs::metadata(&path).await.map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to get file metadata: {err}"))
            })?;

            if !metadata.is_file() {
                return Err(FunctionCallError::RespondToModel(format!(
                    "path is not a file: {file_path}"
                )));
            }

            // Delete the file
            fs::remove_file(&path).await.map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to delete file: {err}"))
            })?;

            Ok(ToolOutput::Function {
                content: format!("Successfully deleted {file_path}"),
                success: Some(true),
            })
        }
        .await;

        // Emit FileEditEndEvent
        {
            use codex_protocol::protocol::Event;
            use codex_protocol::protocol::EventMsg;
            use codex_protocol::protocol::FileEditEndEvent;

            let (success, stderr) = match &result {
                Ok(_) => (true, String::new()),
                Err(FunctionCallError::RespondToModel(msg)) => (false, msg.clone()),
                Err(e) => (false, format!("{e:?}")),
            };

            let end_event = Event {
                id: call_id.clone(),
                msg: EventMsg::FileEditEnd(FileEditEndEvent {
                    call_id,
                    success,
                    stderr,
                }),
            };
            session.send_event(end_event).await;
        }

        result
    }
}
