//! Write tool handler - writes content to files.

use async_trait::async_trait;
use serde::Deserialize;
use tokio::fs;

use crate::function_tool::FunctionCallError;
use crate::tools::context::{ToolInvocation, ToolOutput, ToolPayload};
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
        let ToolInvocation { payload, turn, .. } = invocation;

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
        if path.exists() {
            if !turn.was_file_read(&path) {
                return Err(FunctionCallError::RespondToModel(format!(
                    "You must read the file before writing to it. Use read_file on '{}' first.",
                    path.display()
                )));
            }
        }

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

        // Track that this file was written (mark as read so subsequent writes are allowed)
        turn.mark_file_read(&path);

        let lines = args.content.lines().count();
        let bytes = args.content.len();

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
