use std::path::PathBuf;

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
    content: String,
}

#[async_trait]
impl ToolHandler for WriteFileHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation { payload, .. } = invocation;

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

        let WriteFileArgs { file_path, content } = args;

        let path = PathBuf::from(&file_path);
        if !path.is_absolute() {
            return Err(FunctionCallError::RespondToModel(
                "file_path must be an absolute path".to_string(),
            ));
        }

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

        file.write_all(content.as_bytes()).await.map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to write file: {err}"))
        })?;

        file.flush().await.map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to flush file: {err}"))
        })?;

        Ok(ToolOutput::Function {
            content: format!("Successfully wrote {} bytes to {file_path}", content.len()),
            success: Some(true),
        })
    }
}
