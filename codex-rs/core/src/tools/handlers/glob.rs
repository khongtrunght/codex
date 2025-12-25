//! Glob tool handler - finds files matching glob patterns.

use async_trait::async_trait;
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::tools::context::{ToolInvocation, ToolOutput, ToolPayload};
use crate::function_tool::FunctionCallError;
use crate::tools::registry::{ToolHandler, ToolKind};

const MAX_RESULTS: usize = 100;

#[derive(Deserialize)]
struct GlobArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}

pub struct GlobHandler;

#[async_trait]
impl ToolHandler for GlobHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation { payload, turn, .. } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "glob handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: GlobArgs = serde_json::from_str(&arguments).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to parse function arguments: {err:?}"
            ))
        })?;

        let pattern = args.pattern.trim();
        if pattern.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "pattern must not be empty".to_string(),
            ));
        }

        // Resolve search path
        let search_path = turn.resolve_path(args.path);

        if !search_path.exists() {
            return Err(FunctionCallError::RespondToModel(format!(
                "path does not exist: {}",
                search_path.display()
            )));
        }

        if !search_path.is_dir() {
            return Err(FunctionCallError::RespondToModel(format!(
                "path is not a directory: {}",
                search_path.display()
            )));
        }

        // Use ignore crate with glob pattern
        let mut walker = WalkBuilder::new(&search_path);
        walker
            .hidden(false)  // Include hidden files
            .follow_links(true)
            .git_ignore(true)
            .git_global(true);

        // Add glob pattern as an override
        let mut override_builder = OverrideBuilder::new(&search_path);
        override_builder.add(pattern).map_err(|e| {
            FunctionCallError::RespondToModel(format!("invalid glob pattern: {e}"))
        })?;

        // Exclude .git directory
        let _ = override_builder.add("!.git/**");

        let overrides = override_builder.build().map_err(|e| {
            FunctionCallError::RespondToModel(format!("failed to build glob matcher: {e}"))
        })?;
        walker.overrides(overrides);

        // Collect files with their modification times
        let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();

        for entry in walker.build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Skip directories
            if entry.file_type().is_none_or(|ft| ft.is_dir()) {
                continue;
            }

            let path = entry.path().to_path_buf();
            let mtime = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);

            files.push((path, mtime));
        }

        // Sort by modification time (newest first)
        files.sort_by(|a, b| b.1.cmp(&a.1));

        // Limit results
        let truncated = files.len() > MAX_RESULTS;
        files.truncate(MAX_RESULTS);

        // Format output
        if files.is_empty() {
            return Ok(ToolOutput::Function {
                content: "No files found matching pattern.".to_string(),
                content_items: None,
                success: Some(false),
            });
        }

        let mut output: Vec<String> = files
            .iter()
            .map(|(p, _)| p.display().to_string())
            .collect();

        if truncated {
            output.push(String::new());
            output.push("(Results truncated. Consider using a more specific pattern or path.)".to_string());
        }

        Ok(ToolOutput::Function {
            content: output.join("\n"),
            content_items: None,
            success: Some(true),
        })
    }
}
