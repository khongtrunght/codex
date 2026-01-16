//! Write tool handler - writes content to files.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

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
use crate::tools::runtimes::write_file::WriteFileRequest;
use crate::tools::runtimes::write_file::WriteFileRuntime;
use crate::tools::sandboxing::ToolCtx;
use crate::tools::sandboxing::ToolError;

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

        // Check plan mode write restriction
        if let Err(msg) = check_plan_mode_write(&session, &path).await {
            return Err(FunctionCallError::RespondToModel(msg));
        }

        // Check if file exists - if it does, require it to have been read first (session-level)
        // Also capture original content for diff display
        let original_content = if path.exists() {
            if let Err(e) = session.validate_file_for_edit(&path, true).await {
                return Err(FunctionCallError::RespondToModel(e.to_string()));
            }
            // Read original content for diff generation
            tokio::fs::read_to_string(&path).await.ok()
        } else {
            None
        };

        // Create the request and run through orchestrator
        let req = WriteFileRequest {
            file_path: path.clone(),
            content: args.content.clone(),
        };

        let mut orchestrator = ToolOrchestrator::new();
        let mut runtime = WriteFileRuntime::new();
        let tool_ctx = ToolCtx {
            session: session.as_ref(),
            turn: turn.as_ref(),
            call_id: call_id.clone(),
            tool_name: tool_name.to_string(),
        };

        // Emit begin event for TUI display
        let change = match &original_content {
            Some(old) => {
                let unified_diff = diffy::create_patch(old, &args.content).to_string();
                FileChange::Update {
                    unified_diff,
                    move_path: None,
                }
            }
            None => FileChange::Add {
                content: args.content.clone(),
            },
        };

        let changes: HashMap<std::path::PathBuf, FileChange> =
            [(path.clone(), change)].into_iter().collect();

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
                // Build final change for event emission
                let final_change = if output.is_new_file {
                    FileChange::Add {
                        content: output.new_content.clone(),
                    }
                } else {
                    let old = output.original_content.as_deref().unwrap_or("");
                    let unified_diff = diffy::create_patch(old, &output.new_content).to_string();
                    FileChange::Update {
                        unified_diff,
                        move_path: None,
                    }
                };

                let final_changes: HashMap<std::path::PathBuf, FileChange> =
                    [(path.clone(), final_change)].into_iter().collect();

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

                // Update session state with new content and mtime
                session
                    .update_file_after_write(&path, output.new_content.clone())
                    .await;

                Ok(ToolOutput::Function {
                    content: format!(
                        "Successfully wrote {} lines ({} bytes) to {}",
                        output.lines,
                        output.bytes,
                        path.display()
                    ),
                    content_items: None,
                    success: Some(true),
                })
            }
            Err(ToolError::Rejected(msg)) => Err(FunctionCallError::RespondToModel(msg)),
            Err(ToolError::Codex(err)) => Err(FunctionCallError::RespondToModel(format!(
                "write failed: {err}"
            ))),
        }
    }
}
