use std::path::PathBuf;

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
        let ToolInvocation { payload, .. } = invocation;

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

        let path = PathBuf::from(&file_path);
        if !path.is_absolute() {
            return Err(FunctionCallError::RespondToModel(
                "file_path must be an absolute path".to_string(),
            ));
        }

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
}
