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

pub struct EditFileHandler;

/// JSON arguments accepted by the `edit_file` tool handler.
#[derive(Deserialize)]
struct EditFileArgs {
    /// Absolute path to the file that will be edited.
    file_path: String,
    /// The text to search for and replace. Must be unique within the file.
    old_text: String,
    /// The text to replace old_text with.
    new_text: String,
}

#[async_trait]
impl ToolHandler for EditFileHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation { payload, .. } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "edit_file handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: EditFileArgs = serde_json::from_str(&arguments).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to parse function arguments: {err:?}"
            ))
        })?;

        let EditFileArgs {
            file_path,
            old_text,
            new_text,
        } = args;

        let path = PathBuf::from(&file_path);
        if !path.is_absolute() {
            return Err(FunctionCallError::RespondToModel(
                "file_path must be an absolute path".to_string(),
            ));
        }

        // Read the file
        let content = fs::read_to_string(&path).await.map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to read file: {err}"))
        })?;

        // Check if old_text exists in the file
        if !content.contains(&old_text) {
            return Err(FunctionCallError::RespondToModel(format!(
                "old_text not found in file: {file_path}"
            )));
        }

        // Count occurrences to ensure uniqueness
        let occurrences = content.matches(&old_text).count();
        if occurrences > 1 {
            return Err(FunctionCallError::RespondToModel(format!(
                "old_text appears {occurrences} times in the file. It must be unique. Please provide a longer, more specific string that appears only once."
            )));
        }

        // Perform the replacement
        let new_content = content.replace(&old_text, &new_text);

        // Write the file back
        let mut file = fs::File::create(&path).await.map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to open file for writing: {err}"))
        })?;

        file.write_all(new_content.as_bytes())
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to write file: {err}"))
            })?;

        file.flush().await.map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to flush file: {err}"))
        })?;

        Ok(ToolOutput::Function {
            content: format!(
                "Successfully replaced text in {file_path}. Old length: {}, New length: {}",
                content.len(),
                new_content.len()
            ),
            success: Some(true),
        })
    }
}
