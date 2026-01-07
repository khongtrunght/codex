//! Edit File runtime: executes verified file edits with approval flow.
//!
//! Implements the Approvable trait pattern to request user approval before
//! performing file modifications. Uses the ToolOrchestrator for consistent
//! approval and execution flow.

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

/// Result of an edit file operation.
#[derive(Clone, Debug)]
pub struct EditFileOutput {
    pub message: String,
    pub strategy: MatchStrategy,
    pub replacement_count: usize,
}

/// Matching strategy used for finding the old_string in the file content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStrategy {
    Exact,
    NormalizedWhitespace,
    LineTrimmed,
}

impl MatchStrategy {
    pub fn name(self) -> &'static str {
        match self {
            MatchStrategy::Exact => "exact",
            MatchStrategy::NormalizedWhitespace => "normalized whitespace",
            MatchStrategy::LineTrimmed => "line-trimmed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct EditFileRequest {
    pub file_path: PathBuf,
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
}

#[derive(Default)]
pub struct EditFileRuntime;

#[derive(serde::Serialize, Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct EditFileApprovalKey {
    file_path: PathBuf,
}

impl EditFileRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Sandboxable for EditFileRuntime {
    fn sandbox_preference(&self) -> SandboxablePreference {
        // File operations don't use sandbox subprocess execution
        SandboxablePreference::Forbid
    }

    fn escalate_on_failure(&self) -> bool {
        false
    }
}

impl Approvable<EditFileRequest> for EditFileRuntime {
    type ApprovalKey = EditFileApprovalKey;

    fn approval_keys(&self, req: &EditFileRequest) -> Vec<Self::ApprovalKey> {
        vec![EditFileApprovalKey {
            file_path: req.file_path.clone(),
        }]
    }

    /// Always return NeedsApproval so we can do the proper safety check in start_approval_async.
    /// This ensures edit_file matches apply_patch behavior.
    fn exec_approval_requirement(&self, _req: &EditFileRequest) -> Option<ExecApprovalRequirement> {
        Some(ExecApprovalRequirement::NeedsApproval {
            reason: None,
            proposed_execpolicy_amendment: None,
        })
    }

    fn start_approval_async<'a>(
        &'a mut self,
        req: &'a EditFileRequest,
        ctx: ApprovalCtx<'a>,
    ) -> BoxFuture<'a, ReviewDecision> {
        let keys = self.approval_keys(req);
        let session = ctx.session;
        let turn = ctx.turn;
        let call_id = ctx.call_id.to_string();
        let file_path = req.file_path.clone();
        let old_string = req.old_string.clone();
        let new_string = req.new_string.clone();
        let retry_reason = ctx.retry_reason.clone();

        Box::pin(async move {
            with_cached_approval(&session.services, keys, move || async move {
                // Read current file content to generate diff
                let content = match fs::read_to_string(&file_path).await {
                    Ok(c) => c,
                    Err(_) => return ReviewDecision::Denied,
                };

                let new_content = content.replacen(&old_string, &new_string, 1);
                let unified_diff = diffy::create_patch(&content, &new_content).to_string();

                // Build ApplyPatchAction for safety assessment (matching apply_patch behavior)
                let mut patch_changes: HashMap<PathBuf, ApplyPatchFileChange> = HashMap::new();
                patch_changes.insert(
                    file_path.clone(),
                    ApplyPatchFileChange::Update {
                        unified_diff: unified_diff.clone(),
                        move_path: None,
                        new_content: new_content.clone(),
                    },
                );
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
                        changes.insert(
                            file_path.clone(),
                            FileChange::Update {
                                unified_diff,
                                move_path: None,
                            },
                        );

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

impl ToolRuntime<EditFileRequest, EditFileOutput> for EditFileRuntime {
    async fn run(
        &mut self,
        req: &EditFileRequest,
        _attempt: &SandboxAttempt<'_>,
        _ctx: &ToolCtx<'_>,
    ) -> Result<EditFileOutput, ToolError> {
        // Read the file content
        let content = fs::read_to_string(&req.file_path)
            .await
            .map_err(|e| ToolError::Rejected(format!("failed to read file: {e}")))?;

        // Try to replace with fallback matching strategies
        let (new_content, strategy, count) = match try_replace(
            &content,
            &req.old_string,
            &req.new_string,
            req.replace_all,
        ) {
            Some(result) => result,
            None => {
                let old_preview = if req.old_string.len() > 100 {
                    format!("{}...", &req.old_string[..100])
                } else {
                    req.old_string.clone()
                };
                return Err(ToolError::Rejected(format!(
                    "old_string not found in file. The string:\n```\n{}\n```\nwas not found in {}",
                    old_preview,
                    req.file_path.display()
                )));
            }
        };

        // Write the new content
        fs::write(&req.file_path, &new_content)
            .await
            .map_err(|e| ToolError::Rejected(format!("failed to write file: {e}")))?;

        Ok(EditFileOutput {
            message: format!("Successfully edited {}", req.file_path.display()),
            strategy,
            replacement_count: count,
        })
    }
}

// ============================================================================
// Matching and replacement logic (moved from handler)
// ============================================================================

/// Normalize whitespace by collapsing multiple spaces/tabs into single spaces.
fn normalize_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_whitespace = false;

    for c in s.chars() {
        if c == ' ' || c == '\t' {
            if !last_was_whitespace {
                result.push(' ');
                last_was_whitespace = true;
            }
        } else {
            result.push(c);
            last_was_whitespace = false;
        }
    }

    result
}

/// Trim each line individually.
fn line_trim(s: &str) -> String {
    s.lines().map(str::trim).collect::<Vec<_>>().join("\n")
}

/// Try to find and replace the old_string in the content using various matching strategies.
/// Returns (new_content, match_strategy, replacement_count) if successful.
fn try_replace(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Option<(String, MatchStrategy, usize)> {
    // Strategy 1: Exact match
    if content.contains(old_string) {
        let count = content.matches(old_string).count();
        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };
        return Some((
            new_content,
            MatchStrategy::Exact,
            if replace_all { count } else { 1 },
        ));
    }

    // Strategy 2: Normalized whitespace match
    let normalized_content = normalize_whitespace(content);
    let normalized_old = normalize_whitespace(old_string);

    if normalized_content.contains(&normalized_old) {
        if let Some(result) = replace_with_normalized(content, old_string, new_string, replace_all)
        {
            return Some((result.0, MatchStrategy::NormalizedWhitespace, result.1));
        }
    }

    // Strategy 3: Line-trimmed match
    let trimmed_content = line_trim(content);
    let trimmed_old = line_trim(old_string);

    if trimmed_content.contains(&trimmed_old)
        && let Some(result) = replace_with_line_trim(content, old_string, new_string, replace_all)
    {
        return Some((result.0, MatchStrategy::LineTrimmed, result.1));
    }

    None
}

/// Replace using normalized whitespace matching.
fn replace_with_normalized(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Option<(String, usize)> {
    let lines: Vec<String> = content.lines().map(ToString::to_string).collect();
    let old_lines: Vec<&str> = old_string.lines().collect();

    let mut result_lines: Vec<String> = lines.clone();
    let mut replacements = 0;
    let mut i = 0;

    while i <= lines.len().saturating_sub(old_lines.len()) {
        let mut matches = true;
        for (j, old_line) in old_lines.iter().enumerate() {
            if normalize_whitespace(&lines[i + j]) != normalize_whitespace(old_line) {
                matches = false;
                break;
            }
        }

        if matches {
            let new_lines: Vec<String> = new_string.lines().map(ToString::to_string).collect();
            let new_lines_len = new_lines.len();

            let before: Vec<String> = result_lines[..i].to_vec();
            let after: Vec<String> = if i + old_lines.len() < result_lines.len() {
                result_lines[i + old_lines.len()..].to_vec()
            } else {
                vec![]
            };

            result_lines = before;
            result_lines.extend(new_lines);
            result_lines.extend(after);

            replacements += 1;

            if !replace_all {
                break;
            }

            i += new_lines_len;
        } else {
            i += 1;
        }
    }

    if replacements > 0 {
        let mut result = result_lines.join("\n");
        if content.ends_with('\n') && !result.ends_with('\n') {
            result.push('\n');
        }
        Some((result, replacements))
    } else {
        None
    }
}

/// Replace using line-trimmed matching.
fn replace_with_line_trim(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Option<(String, usize)> {
    let lines: Vec<String> = content.lines().map(ToString::to_string).collect();
    let old_lines: Vec<&str> = old_string.lines().collect();

    let mut result_lines: Vec<String> = lines.clone();
    let mut replacements = 0;
    let mut i = 0;

    while i <= lines.len().saturating_sub(old_lines.len()) {
        let mut matches = true;
        for (j, old_line) in old_lines.iter().enumerate() {
            if lines[i + j].trim() != old_line.trim() {
                matches = false;
                break;
            }
        }

        if matches {
            let leading_whitespace: String =
                lines[i].chars().take_while(|c| c.is_whitespace()).collect();

            let new_lines: Vec<String> = new_string
                .lines()
                .enumerate()
                .map(|(idx, line)| {
                    if idx == 0 {
                        format!("{}{}", leading_whitespace, line.trim_start())
                    } else {
                        line.to_string()
                    }
                })
                .collect();

            let before: Vec<String> = result_lines[..i].to_vec();
            let after: Vec<String> = if i + old_lines.len() < result_lines.len() {
                result_lines[i + old_lines.len()..].to_vec()
            } else {
                vec![]
            };

            result_lines = before;
            result_lines.extend(new_lines);
            result_lines.extend(after);

            replacements += 1;

            if !replace_all {
                break;
            }
        }

        i += 1;
    }

    if replacements > 0 {
        let mut result = result_lines.join("\n");
        if content.ends_with('\n') && !result.ends_with('\n') {
            result.push('\n');
        }
        Some((result, replacements))
    } else {
        None
    }
}
