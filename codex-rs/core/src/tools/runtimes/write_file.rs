//! Write File runtime: executes file writes with approval flow.
//!
//! Implements the Approvable trait pattern to request user approval before
//! writing files. Uses the ToolOrchestrator for consistent approval and
//! execution flow.

use crate::protocol::FileChange;
use crate::safety::SafetyCheck;
use crate::safety::assess_patch_safety;
use crate::tools::sandboxing::Approvable;
use crate::tools::sandboxing::ApprovalCtx;
use crate::tools::sandboxing::ExecApprovalRequirement;
use crate::tools::sandboxing::SandboxAttempt;
use crate::tools::sandboxing::Sandboxable;
use crate::tools::sandboxing::SandboxablePreference;
use crate::tools::sandboxing::ToolCtx;
use crate::tools::sandboxing::ToolError;
use crate::tools::sandboxing::ToolRuntime;
use crate::tools::sandboxing::with_cached_approval;
use codex_apply_patch::ApplyPatchAction;
use codex_apply_patch::ApplyPatchFileChange;
use codex_protocol::protocol::ReviewDecision;
use futures::future::BoxFuture;
use std::collections::HashMap;
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

    /// Always return NeedsApproval so we can do the proper safety check in start_approval_async.
    /// This ensures write_file matches apply_patch behavior.
    fn exec_approval_requirement(
        &self,
        _req: &WriteFileRequest,
    ) -> Option<ExecApprovalRequirement> {
        Some(ExecApprovalRequirement::NeedsApproval {
            reason: None,
            proposed_execpolicy_amendment: None,
        })
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
        let new_content = req.content.clone();
        let retry_reason = ctx.retry_reason.clone();

        Box::pin(async move {
            with_cached_approval(&session.services, key, move || async move {
                // Determine if this is a new file or update
                let (patch_change, protocol_change) = if file_path.exists() {
                    // Existing file - generate diff
                    if let Ok(old_content) = fs::read_to_string(&file_path).await {
                        let unified_diff =
                            diffy::create_patch(&old_content, &new_content).to_string();
                        (
                            ApplyPatchFileChange::Update {
                                unified_diff: unified_diff.clone(),
                                move_path: None,
                                new_content: new_content.clone(),
                            },
                            FileChange::Update {
                                unified_diff,
                                move_path: None,
                            },
                        )
                    } else {
                        (
                            ApplyPatchFileChange::Add {
                                content: new_content.clone(),
                            },
                            FileChange::Add {
                                content: new_content.clone(),
                            },
                        )
                    }
                } else {
                    // New file
                    (
                        ApplyPatchFileChange::Add {
                            content: new_content.clone(),
                        },
                        FileChange::Add {
                            content: new_content.clone(),
                        },
                    )
                };

                // Build ApplyPatchAction for safety assessment (matching apply_patch behavior)
                let mut patch_changes: HashMap<PathBuf, ApplyPatchFileChange> = HashMap::new();
                patch_changes.insert(file_path.clone(), patch_change);
                let action = ApplyPatchAction::new(patch_changes, turn.cwd.clone());

                // Use assess_patch_safety like apply_patch does
                match assess_patch_safety(
                    &action,
                    turn.approval_policy,
                    &turn.sandbox_policy,
                    &turn.cwd,
                ) {
                    SafetyCheck::AutoApprove { .. } => {
                        // Auto-approve without going to TUI2
                        ReviewDecision::Approved
                    }
                    SafetyCheck::AskUser => {
                        // Build changes for TUI2 approval request
                        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
                        changes.insert(file_path, protocol_change);

                        // Send approval request to TUI2
                        let rx = session
                            .request_patch_approval(turn, call_id, changes, retry_reason, None)
                            .await;
                        rx.await.unwrap_or_default()
                    }
                    SafetyCheck::Reject { .. } => {
                        // Reject the operation
                        ReviewDecision::Denied
                    }
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
                fs::create_dir_all(parent).await.map_err(|e| {
                    ToolError::Rejected(format!("failed to create directories: {e}"))
                })?;
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
