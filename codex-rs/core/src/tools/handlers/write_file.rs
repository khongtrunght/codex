//! Write tool handler - writes content to files.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::fs;

use std::time::Duration;

use crate::exec::ExecToolCallOutput;
use crate::exec::StreamOutput;
use crate::function_tool::FunctionCallError;
use crate::protocol::FileChange;
use crate::tools::context::{ToolInvocation, ToolOutput, ToolPayload};
use crate::tools::events::{ToolEmitter, ToolEventCtx};
use crate::tools::registry::{ToolHandler, ToolKind};

#[derive(Deserialize)]
struct WriteFileArgs {
    file_path: String,
    content: String,
}

pub struct WriteFileHandler;

#[async_trait]
impl ToolHandler for WriteFileHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            payload,
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

        let args: WriteFileArgs = serde_json::from_str(&arguments).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to parse function arguments: {err:?}"
            ))
        })?;

        let file_path = args.file_path.trim();
        if file_path.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "file_path must not be empty".to_string(),
            ));
        }

        // Resolve the path relative to turn cwd
        let path = turn.resolve_path(Some(file_path.to_string()));

        // Check if file exists - if it does, require it to have been read first
        // Also capture original content for diff display
        let original_content = if path.exists() {
            if !turn.was_file_read(&path) {
                return Err(FunctionCallError::RespondToModel(format!(
                    "You must read the file before writing to it. Use read_file on '{}' first.",
                    path.display()
                )));
            }
            // Read original content for diff generation
            fs::read_to_string(&path).await.ok()
        } else {
            None
        };

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to create parent directories for {}: {e}",
                        path.display()
                    ))
                })?;
            }
        }

        // Write the content
        fs::write(&path, &args.content).await.map_err(|e| {
            FunctionCallError::RespondToModel(format!(
                "failed to write file {}: {e}",
                path.display()
            ))
        })?;

        // Emit diff events for TUI display using ToolEmitter pattern
        let change = match original_content {
            Some(old) => {
                // File was overwritten - show diff
                let unified_diff = diffy::create_patch(&old, &args.content).to_string();
                FileChange::Update {
                    unified_diff,
                    move_path: None,
                }
            }
            None => {
                // New file was created - show as added
                FileChange::Add {
                    content: args.content.clone(),
                }
            }
        };

        let changes: HashMap<std::path::PathBuf, FileChange> =
            [(path.clone(), change)].into_iter().collect();

        let emitter = ToolEmitter::apply_patch(changes, true);
        let event_ctx = ToolEventCtx::new(session.as_ref(), turn.as_ref(), &call_id, Some(&tracker));
        emitter.begin(event_ctx).await;

        // Create success output and emit end event
        let lines = args.content.lines().count();
        let bytes = args.content.len();
        let success_msg = format!(
            "Successfully wrote {} lines ({} bytes) to {}",
            lines, bytes, path.display()
        );
        let exec_output = ExecToolCallOutput {
            exit_code: 0,
            stdout: StreamOutput::new(success_msg.clone()),
            stderr: StreamOutput::new(String::new()),
            aggregated_output: StreamOutput::new(success_msg),
            duration: Duration::ZERO,
            timed_out: false,
        };
        let event_ctx = ToolEventCtx::new(session.as_ref(), turn.as_ref(), &call_id, Some(&tracker));
        let _ = emitter.finish(event_ctx, Ok(exec_output)).await;

        // Track that this file was written (mark as read so subsequent writes are allowed)
        turn.mark_file_read(&path);

        Ok(ToolOutput::Function {
            content: format!(
                "Successfully wrote {} lines ({} bytes) to {}",
                lines,
                bytes,
                path.display()
            ),
            content_items: None,
            success: Some(true),
        })
    }
}
