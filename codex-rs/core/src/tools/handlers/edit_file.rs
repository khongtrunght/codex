//! Edit tool handler - performs string replacement editing on files.
//!
//! This follows OpenCode's pattern for matching strategies:
//! 1. Exact match
//! 2. Normalized whitespace match
//! 3. Line-trimmed match

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::fs;

use std::time::Duration;

use crate::exec::ExecToolCallOutput;
use crate::exec::StreamOutput;
use crate::function_tool::FunctionCallError;
use crate::protocol::FileChange;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::events::ToolEmitter;
use crate::tools::events::ToolEventCtx;
use crate::tools::plan_mode_restriction::check_plan_mode_write;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

#[derive(Deserialize)]
struct EditFileArgs {
    file_path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

/// Matching strategy used for finding the old_string in the file content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchStrategy {
    Exact,
    NormalizedWhitespace,
    LineTrimmed,
}

impl MatchStrategy {
    fn name(self) -> &'static str {
        match self {
            MatchStrategy::Exact => "exact",
            MatchStrategy::NormalizedWhitespace => "normalized whitespace",
            MatchStrategy::LineTrimmed => "line-trimmed",
        }
    }
}

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
        // Find positions in normalized content, then map back to original
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
        // Check if old_lines match at position i
        let mut matches = true;
        for (j, old_line) in old_lines.iter().enumerate() {
            if normalize_whitespace(&lines[i + j]) != normalize_whitespace(old_line) {
                matches = false;
                break;
            }
        }

        if matches {
            // Replace the matching lines with new_string lines
            let new_lines: Vec<String> = new_string.lines().map(ToString::to_string).collect();
            let new_lines_len = new_lines.len();

            // Remove old lines and insert new ones
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
        // Handle trailing newline from original content
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
        // Check if old_lines match at position i (trimmed comparison)
        let mut matches = true;
        for (j, old_line) in old_lines.iter().enumerate() {
            if lines[i + j].trim() != old_line.trim() {
                matches = false;
                break;
            }
        }

        if matches {
            // Preserve leading whitespace from the first matching line
            let leading_whitespace: String =
                lines[i].chars().take_while(|c| c.is_whitespace()).collect();

            // Replace the matching lines with new_string lines, preserving indentation
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

            // Remove old lines and insert new ones
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
        // Handle trailing newline from original content
        let mut result = result_lines.join("\n");
        if content.ends_with('\n') && !result.ends_with('\n') {
            result.push('\n');
        }
        Some((result, replacements))
    } else {
        None
    }
}

pub struct EditFileHandler;

#[async_trait]
impl ToolHandler for EditFileHandler {
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
                    "edit_file handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: EditFileArgs = serde_json::from_str(&arguments).map_err(|err| {
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

        // Check plan mode write restriction
        if let Err(msg) = check_plan_mode_write(&session, &path).await {
            return Err(FunctionCallError::RespondToModel(msg));
        }

        // Check if file exists
        if !path.exists() {
            return Err(FunctionCallError::RespondToModel(format!(
                "file does not exist: {}",
                path.display()
            )));
        }

        // Require the file to have been read first
        if !turn.was_file_read(&path) {
            return Err(FunctionCallError::RespondToModel(format!(
                "You must read the file before editing it. Use read_file on '{}' first.",
                path.display()
            )));
        }

        // Validate that old_string and new_string are different
        if args.old_string == args.new_string {
            return Err(FunctionCallError::RespondToModel(
                "old_string and new_string must be different".to_string(),
            ));
        }

        // Read the file content
        let content = fs::read_to_string(&path).await.map_err(|e| {
            FunctionCallError::RespondToModel(format!(
                "failed to read file {}: {e}",
                path.display()
            ))
        })?;

        // Try to replace with fallback matching strategies
        let (new_content, strategy, count) = match try_replace(
            &content,
            &args.old_string,
            &args.new_string,
            args.replace_all,
        ) {
            Some(result) => result,
            None => {
                // Provide helpful error message
                let old_preview = if args.old_string.len() > 100 {
                    format!("{}...", &args.old_string[..100])
                } else {
                    args.old_string.clone()
                };

                return Err(FunctionCallError::RespondToModel(format!(
                    "old_string not found in file. The string:\n```\n{}\n```\nwas not found in {}",
                    old_preview,
                    path.display()
                )));
            }
        };

        // Write the new content
        fs::write(&path, &new_content).await.map_err(|e| {
            FunctionCallError::RespondToModel(format!(
                "failed to write file {}: {e}",
                path.display()
            ))
        })?;

        // Emit diff events for TUI display using ToolEmitter pattern
        let unified_diff = diffy::create_patch(&content, &new_content).to_string();
        let changes: HashMap<std::path::PathBuf, FileChange> = [(
            path.clone(),
            FileChange::Update {
                unified_diff,
                move_path: None,
            },
        )]
        .into_iter()
        .collect();

        let emitter = ToolEmitter::apply_patch(changes, true);
        let event_ctx =
            ToolEventCtx::new(session.as_ref(), turn.as_ref(), &call_id, Some(&tracker));
        emitter.begin(event_ctx).await;

        // Create success output and emit end event
        let success_msg = format!("Successfully edited {}", path.display());
        let exec_output = ExecToolCallOutput {
            exit_code: 0,
            stdout: StreamOutput::new(success_msg.clone()),
            stderr: StreamOutput::new(String::new()),
            aggregated_output: StreamOutput::new(success_msg),
            duration: Duration::ZERO,
            timed_out: false,
        };
        let event_ctx =
            ToolEventCtx::new(session.as_ref(), turn.as_ref(), &call_id, Some(&tracker));
        let _ = emitter.finish(event_ctx, Ok(exec_output)).await;

        let strategy_note = if strategy != MatchStrategy::Exact {
            format!(" (matched using {} strategy)", strategy.name())
        } else {
            String::new()
        };

        let count_note = if args.replace_all && count > 1 {
            format!(" ({count} occurrences)")
        } else {
            String::new()
        };

        Ok(ToolOutput::Function {
            content: format!(
                "Successfully edited {}{}{}",
                path.display(),
                count_note,
                strategy_note
            ),
            content_items: None,
            success: Some(true),
        })
    }
}
