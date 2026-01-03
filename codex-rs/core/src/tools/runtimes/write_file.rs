//! Write File runtime: executes file writes with approval flow.
//!
//! Implements the Approvable trait pattern to request user approval before
//! writing files. Uses the ToolOrchestrator for consistent approval and
//! execution flow.

use crate::tools::sandboxing::Approvable;
use crate::tools::sandboxing::ApprovalCtx;
use crate::tools::sandboxing::SandboxAttempt;
use crate::tools::sandboxing::Sandboxable;
use crate::tools::sandboxing::SandboxablePreference;
use crate::tools::sandboxing::ToolCtx;
use crate::tools::sandboxing::ToolError;
use crate::tools::sandboxing::ToolRuntime;
use crate::tools::sandboxing::with_cached_approval;
use codex_protocol::protocol::ReviewDecision;
use futures::future::BoxFuture;
use std::path::PathBuf;
use tokio::fs;

/// Result of a write file operation.
#[derive(Clone, Debug)]
pub struct WriteFileOutput {
    pub message: String,
    pub lines: usize,
    pub bytes: usize,
    pub is_new_file: bool,
    /// Original content if the file existed (for diff generation)
    pub original_content: Option<String>,
    /// New content that was written
    pub new_content: String,
}

#[derive(Clone, Debug)]
pub struct WriteFileRequest {
    pub file_path: PathBuf,
    pub content: String,
}

#[derive(Default)]
pub struct WriteFileRuntime;

#[derive(serde::Serialize, Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct WriteFileApprovalKey {
    file_path: PathBuf,
}

impl WriteFileRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Sandboxable for WriteFileRuntime {
    fn sandbox_preference(&self) -> SandboxablePreference {
        // File operations don't use sandbox subprocess execution
        SandboxablePreference::Forbid
    }

    fn escalate_on_failure(&self) -> bool {
        false
    }
}

impl Approvable<WriteFileRequest> for WriteFileRuntime {
    type ApprovalKey = WriteFileApprovalKey;

    fn approval_key(&self, req: &WriteFileRequest) -> Self::ApprovalKey {
        WriteFileApprovalKey {
            file_path: req.file_path.clone(),
        }
    }

    fn start_approval_async<'a>(
        &'a mut self,
        req: &'a WriteFileRequest,
        ctx: ApprovalCtx<'a>,
    ) -> BoxFuture<'a, ReviewDecision> {
        let key = self.approval_key(req);
        let session = ctx.session;
        let turn = ctx.turn;
        let call_id = ctx.call_id.to_string();
        let file_path = req.file_path.clone();
        let retry_reason = ctx.retry_reason.clone();

        Box::pin(async move {
            with_cached_approval(&session.services, key, move || async move {
                if let Some(reason) = retry_reason {
                    session
                        .request_command_approval(
                            turn,
                            call_id,
                            vec!["write_file".to_string(), file_path.display().to_string()],
                            file_path.parent().unwrap_or(&file_path).to_path_buf(),
                            Some(reason),
                            None,
                        )
                        .await
                } else {
                    // Auto-approve for write_file since we've already verified conditions
                    ReviewDecision::Approved
                }
            })
            .await
        })
    }
}

impl ToolRuntime<WriteFileRequest, WriteFileOutput> for WriteFileRuntime {
    async fn run(
        &mut self,
        req: &WriteFileRequest,
        _attempt: &SandboxAttempt<'_>,
        _ctx: &ToolCtx<'_>,
    ) -> Result<WriteFileOutput, ToolError> {
        // Check if file exists and capture original content for diff
        let (is_new_file, original_content) = if req.file_path.exists() {
            let content = fs::read_to_string(&req.file_path).await.ok();
            (false, content)
        } else {
            (true, None)
        };

        // Ensure parent directory exists
        if let Some(parent) = req.file_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| ToolError::Rejected(format!("failed to create directories: {e}")))?;
            }
        }

        // Write the content
        fs::write(&req.file_path, &req.content)
            .await
            .map_err(|e| ToolError::Rejected(format!("failed to write file: {e}")))?;

        let lines = req.content.lines().count();
        let bytes = req.content.len();

        Ok(WriteFileOutput {
            message: format!(
                "Successfully wrote {} lines ({} bytes) to {}",
                lines,
                bytes,
                req.file_path.display()
            ),
            lines,
            bytes,
            is_new_file,
            original_content,
            new_content: req.content.clone(),
        })
    }
}
