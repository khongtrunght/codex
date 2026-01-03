//! Edit tool handler - performs string replacement editing on files.
//!
//! This follows OpenCode's pattern for matching strategies:
//! 1. Exact match
//! 2. Normalized whitespace match
//! 3. Line-trimmed match

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::fs;

use crate::exec::ExecToolCallOutput;
use crate::exec::StreamOutput;
use crate::function_tool::FunctionCallError;
use crate::protocol::FileChange;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::events::ToolEmitter;
use crate::tools::events::ToolEventCtx;
use crate::tools::orchestrator::ToolOrchestrator;
use crate::tools::plan_mode_restriction::check_plan_mode_write;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::runtimes::edit_file::EditFileRequest;
use crate::tools::runtimes::edit_file::EditFileRuntime;
use crate::tools::runtimes::edit_file::MatchStrategy;
use crate::tools::sandboxing::ToolCtx;
use crate::tools::sandboxing::ToolError;

#[derive(Deserialize)]
struct EditFileArgs {
    file_path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
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
            tool_name,
            payload,
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

        // Read original content for diff display before edit
        let original_content = fs::read_to_string(&path).await.map_err(|e| {
            FunctionCallError::RespondToModel(format!(
                "failed to read file {}: {e}",
                path.display()
            ))
        })?;

        // Create the request and run through orchestrator
        let req = EditFileRequest {
            file_path: path.clone(),
            old_string: args.old_string,
            new_string: args.new_string.clone(),
            replace_all: args.replace_all,
        };

        let mut orchestrator = ToolOrchestrator::new();
        let mut runtime = EditFileRuntime::new();
        let tool_ctx = ToolCtx {
            session: session.as_ref(),
            turn: turn.as_ref(),
            call_id: call_id.clone(),
            tool_name: tool_name.to_string(),
        };

        // Emit begin event before running
        let new_content_preview = original_content.replacen(&req.old_string, &req.new_string, 1);
        let unified_diff = diffy::create_patch(&original_content, &new_content_preview).to_string();
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

        // Run through the orchestrator
        let result = orchestrator
            .run(&mut runtime, &req, &tool_ctx, &turn, turn.approval_policy)
            .await;

        // Handle result and emit end event
        match result {
            Ok(output) => {
                // Read the actual new content for accurate diff
                let actual_new_content = fs::read_to_string(&path).await.unwrap_or_default();
                let final_diff = diffy::create_patch(&original_content, &actual_new_content).to_string();
                let final_changes: HashMap<std::path::PathBuf, FileChange> = [(
                    path.clone(),
                    FileChange::Update {
                        unified_diff: final_diff,
                        move_path: None,
                    },
                )]
                .into_iter()
                .collect();

                let emitter = ToolEmitter::apply_patch(final_changes, true);
                let exec_output = ExecToolCallOutput {
                    exit_code: 0,
                    stdout: StreamOutput::new(output.message.clone()),
                    stderr: StreamOutput::new(String::new()),
                    aggregated_output: StreamOutput::new(output.message.clone()),
                    duration: Duration::ZERO,
                    timed_out: false,
                };
                let event_ctx =
                    ToolEventCtx::new(session.as_ref(), turn.as_ref(), &call_id, Some(&tracker));
                let _ = emitter.finish(event_ctx, Ok(exec_output)).await;

                let strategy_note = if output.strategy != MatchStrategy::Exact {
                    format!(" (matched using {} strategy)", output.strategy.name())
                } else {
                    String::new()
                };

                let count_note = if args.replace_all && output.replacement_count > 1 {
                    format!(" ({} occurrences)", output.replacement_count)
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
            Err(ToolError::Rejected(msg)) => {
                Err(FunctionCallError::RespondToModel(msg))
            }
            Err(ToolError::Codex(err)) => {
                Err(FunctionCallError::RespondToModel(format!("edit failed: {err}")))
            }
        }
    }
}
