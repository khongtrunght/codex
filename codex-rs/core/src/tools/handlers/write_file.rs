use async_trait::async_trait;
use serde::Deserialize;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct WriteFileHandler;

/// JSON arguments accepted by the `write_file` tool handler.
#[derive(Deserialize)]
struct WriteFileArgs {
    /// Absolute path to the file that will be written.
    file_path: String,
    /// Content to write to the file.
    #[serde(alias = "file_content")]
    content: String,
}

#[async_trait]
impl ToolHandler for WriteFileHandler {
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
                    "write_file handler received unsupported payload".to_string(),
                ));
            }
        };

        // Parse arguments first
        let args: WriteFileArgs = serde_json::from_str(&arguments).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to parse function arguments: {err:?}"
            ))
        })?;

        let WriteFileArgs {
            file_path,
            content: file_content,
        } = args;

        // Resolve relative paths against cwd
        let path = turn.resolve_path(Some(file_path.clone()));

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
                FileChange::Add {
                    content: file_content.clone(),
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

        // Execute the actual file write
        let result = async {
            // Create parent directories if they don't exist
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await.map_err(|err| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to create parent directories: {err}"
                    ))
                })?;
            }

            // Write the file
            let mut file = fs::File::create(&path).await.map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to create file: {err}"))
            })?;

            file.write_all(file_content.as_bytes())
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!("failed to write file: {err}"))
                })?;

            file.flush().await.map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to flush file: {err}"))
            })?;

            Ok(ToolOutput::Function {
                content: format!(
                    "Successfully wrote {} bytes to {file_path}",
                    file_content.len()
                ),
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
