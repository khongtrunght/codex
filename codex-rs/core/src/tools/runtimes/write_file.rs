//! Write File runtime: executes file writes with approval flow.
//!
//! Uses `assess_patch_safety` for safety checking and the orchestrator
//! for consistent approval and execution flow.

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
use codex_utils_absolute_path::AbsolutePathBuf;
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
    type ApprovalKey = AbsolutePathBuf;

    fn approval_keys(&self, req: &WriteFileRequest) -> Vec<Self::ApprovalKey> {
        if let Ok(abs_path) = AbsolutePathBuf::try_from(req.file_path.clone()) {
            vec![abs_path]
        } else {
            vec![]
        }
    }

    fn exec_approval_requirement(
        &self,
        _req: &WriteFileRequest,
    ) -> Option<ExecApprovalRequirement> {
        // Return NeedsApproval by default; actual decision is made in start_approval_async
        // using assess_patch_safety
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
        let approval_keys = self.approval_keys(req);
        let session = ctx.session;
        let turn = ctx.turn;
        let call_id = ctx.call_id.to_string();
        let file_path = req.file_path.clone();
        let new_content = req.content.clone();
        let retry_reason = ctx.retry_reason.clone();

        Box::pin(async move {
            // Handle retry case: always prompt for approval
            if let Some(reason) = retry_reason {
                let (protocol_change, _) = build_file_change(&file_path, &new_content).await;
                let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
                changes.insert(file_path, protocol_change);

                let rx = session
                    .request_patch_approval(turn, call_id, changes, Some(reason), None)
                    .await;
                return rx.await.unwrap_or_default();
            }

            with_cached_approval(&session.services, "write_file", approval_keys, || async {
                // Build changes for safety assessment
                let (protocol_change, patch_change) =
                    build_file_change(&file_path, &new_content).await;

                // Build ApplyPatchAction for safety assessment
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
                    SafetyCheck::AutoApprove { .. } => ReviewDecision::Approved,
                    SafetyCheck::AskUser => {
                        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
                        changes.insert(file_path, protocol_change);

                        let rx = session
                            .request_patch_approval(turn, call_id, changes, None, None)
                            .await;
                        rx.await.unwrap_or_default()
                    }
                    SafetyCheck::Reject { .. } => ReviewDecision::Denied,
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
        if let Some(parent) = req.file_path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::Rejected(format!("failed to create directories: {e}")))?;
        }

        // Write the content
        fs::write(&req.file_path, &req.content)
            .await
            .map_err(|e| ToolError::Rejected(format!("failed to write file: {e}")))?;

        let lines = req.content.lines().count();
        let bytes = req.content.len();

        Ok(WriteFileOutput {
            message: format!(
                "Successfully wrote {lines} lines ({bytes} bytes) to {}",
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

/// Build both protocol FileChange and ApplyPatchFileChange for a write operation.
async fn build_file_change(
    file_path: &PathBuf,
    new_content: &str,
) -> (FileChange, ApplyPatchFileChange) {
    if file_path.exists() {
        if let Ok(old_content) = fs::read_to_string(file_path).await {
            let unified_diff = diffy::create_patch(&old_content, new_content).to_string();
            (
                FileChange::Update {
                    unified_diff: unified_diff.clone(),
                    move_path: None,
                },
                ApplyPatchFileChange::Update {
                    unified_diff,
                    move_path: None,
                    new_content: new_content.to_string(),
                },
            )
        } else {
            // Can't read existing file, treat as add
            (
                FileChange::Add {
                    content: new_content.to_string(),
                },
                ApplyPatchFileChange::Add {
                    content: new_content.to_string(),
                },
            )
        }
    } else {
        // New file
        (
            FileChange::Add {
                content: new_content.to_string(),
            },
            ApplyPatchFileChange::Add {
                content: new_content.to_string(),
            },
        )
    }
}
