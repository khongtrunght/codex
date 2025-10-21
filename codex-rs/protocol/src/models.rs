use std::collections::HashMap;

use base64::Engine;
use mcp_types::CallToolResult;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::ser::Serializer;
use ts_rs::TS;

use crate::protocol::InputItem;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseInputItem {
    Message {
        role: String,
        content: Vec<ContentItem>,
    },
    FunctionCallOutput {
        call_id: String,
        output: FunctionCallOutputPayload,
    },
    McpToolCallOutput {
        call_id: String,
        result: Result<CallToolResult, String>,
    },
    CustomToolCallOutput {
        call_id: String,
        output: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentItem {
    InputText { text: String },
    InputImage { image_url: String },
    OutputText { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseItem {
    Message {
        #[serde(skip_serializing)]
        id: Option<String>,
        role: String,
        content: Vec<ContentItem>,
    },
    Reasoning {
        #[serde(default, skip_serializing)]
        id: String,
        summary: Vec<ReasoningItemReasoningSummary>,
        #[serde(default, skip_serializing_if = "should_serialize_reasoning_content")]
        content: Option<Vec<ReasoningItemContent>>,
        encrypted_content: Option<String>,
    },
    LocalShellCall {
        /// Set when using the chat completions API.
        #[serde(skip_serializing)]
        id: Option<String>,
        /// Set when using the Responses API.
        call_id: Option<String>,
        status: LocalShellStatus,
        action: LocalShellAction,
    },
    FunctionCall {
        #[serde(skip_serializing)]
        id: Option<String>,
        name: String,
        // The Responses API returns the function call arguments as a *string* that contains
        // JSON, not as an already‑parsed object. We keep it as a raw string here and let
        // Session::handle_function_call parse it into a Value. This exactly matches the
        // Chat Completions + Responses API behavior.
        arguments: String,
        call_id: String,
    },
    // NOTE: The input schema for `function_call_output` objects that clients send to the
    // OpenAI /v1/responses endpoint is NOT the same shape as the objects the server returns on the
    // SSE stream. When *sending* we must wrap the string output inside an object that includes a
    // required `success` boolean. The upstream TypeScript CLI does this implicitly. To ensure we
    // serialize exactly the expected shape we introduce a dedicated payload struct and flatten it
    // here.
    FunctionCallOutput {
        call_id: String,
        output: FunctionCallOutputPayload,
    },
    CustomToolCall {
        #[serde(skip_serializing)]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,

        call_id: String,
        name: String,
        input: String,
    },
    CustomToolCallOutput {
        call_id: String,
        output: String,
    },
    // Emitted by the Responses API when the agent triggers a web search.
    // Example payload (from SSE `response.output_item.done`):
    // {
    //   "id":"ws_...",
    //   "type":"web_search_call",
    //   "status":"completed",
    //   "action": {"type":"search","query":"weather: San Francisco, CA"}
    // }
    WebSearchCall {
        #[serde(skip_serializing)]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        action: WebSearchAction,
    },
    #[serde(other)]
    Other,
}

fn should_serialize_reasoning_content(content: &Option<Vec<ReasoningItemContent>>) -> bool {
    match content {
        Some(content) => !content
            .iter()
            .any(|c| matches!(c, ReasoningItemContent::ReasoningText { .. })),
        None => false,
    }
}

impl From<ResponseInputItem> for ResponseItem {
    fn from(item: ResponseInputItem) -> Self {
        match item {
            ResponseInputItem::Message { role, content } => Self::Message {
                role,
                content,
                id: None,
            },
            ResponseInputItem::FunctionCallOutput { call_id, output } => {
                Self::FunctionCallOutput { call_id, output }
            }
            ResponseInputItem::McpToolCallOutput { call_id, result } => Self::FunctionCallOutput {
                call_id,
                output: FunctionCallOutputPayload {
                    success: Some(result.is_ok()),
                    content: result.map_or_else(
                        |tool_call_err| format!("err: {tool_call_err:?}"),
                        |result| {
                            serde_json::to_string(&result)
                                .unwrap_or_else(|e| format!("JSON serialization error: {e}"))
                        },
                    ),
                },
            },
            ResponseInputItem::CustomToolCallOutput { call_id, output } => {
                Self::CustomToolCallOutput { call_id, output }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "snake_case")]
pub enum LocalShellStatus {
    Completed,
    InProgress,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalShellAction {
    Exec(LocalShellExecAction),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct LocalShellExecAction {
    pub command: Vec<String>,
    pub timeout_ms: Option<u64>,
    pub working_directory: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebSearchAction {
    Search {
        query: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningItemReasoningSummary {
    SummaryText { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningItemContent {
    ReasoningText { text: String },
    Text { text: String },
}

impl From<Vec<InputItem>> for ResponseInputItem {
    fn from(items: Vec<InputItem>) -> Self {
        Self::Message {
            role: "user".to_string(),
            content: items
                .into_iter()
                .filter_map(|c| match c {
                    InputItem::Text { text } => Some(ContentItem::InputText { text }),
                    InputItem::Image { image_url } => Some(ContentItem::InputImage { image_url }),
                    InputItem::LocalImage { path } => match std::fs::read(&path) {
                        Ok(bytes) => {
                            let mime = mime_guess::from_path(&path)
                                .first()
                                .map(|m| m.essence_str().to_owned())
                                .unwrap_or_else(|| "image".to_string());
                            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                            Some(ContentItem::InputImage {
                                image_url: format!("data:{mime};base64,{encoded}"),
                            })
                        }
                        Err(err) => {
                            tracing::warn!(
                                "Skipping image {} – could not read file: {}",
                                path.display(),
                                err
                            );
                            None
                        }
                    },
                    InputItem::LocalFile { path, max_lines } => {
                        match read_file_with_context(&path, max_lines) {
                            Ok(content) => Some(ContentItem::InputText { text: content }),
                            Err(err) => {
                                tracing::warn!(
                                    "Skipping file {} – could not read: {}",
                                    path.display(),
                                    err
                                );
                                None
                            }
                        }
                    }
                    InputItem::LocalFolder { path, max_depth } => {
                        match read_folder_with_context(&path, max_depth) {
                            Ok(content) => Some(ContentItem::InputText { text: content }),
                            Err(err) => {
                                tracing::warn!(
                                    "Skipping folder {} – could not read: {}",
                                    path.display(),
                                    err
                                );
                                None
                            }
                        }
                    }
                })
                .collect::<Vec<ContentItem>>(),
        }
    }
}

/// If the `name` of a `ResponseItem::FunctionCall` is either `container.exec`
/// or shell`, the `arguments` field should deserialize to this struct.
#[derive(Deserialize, Debug, Clone, PartialEq, TS)]
pub struct ShellToolCallParams {
    pub command: Vec<String>,
    pub workdir: Option<String>,

    /// This is the maximum time in milliseconds that the command is allowed to run.
    #[serde(alias = "timeout")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_escalated_permissions: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
}

#[derive(Debug, Clone, PartialEq, TS)]
pub struct FunctionCallOutputPayload {
    pub content: String,
    // TODO(jif) drop this.
    pub success: Option<bool>,
}

// The Responses API expects two *different* shapes depending on success vs failure:
//   • success → output is a plain string (no nested object)
//   • failure → output is an object { content, success:false }
// The upstream TypeScript CLI implements this by special‑casing the serialize path.
// We replicate that behavior with a manual Serialize impl.

impl Serialize for FunctionCallOutputPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // The upstream TypeScript CLI always serializes `output` as a *plain string* regardless
        // of whether the function call succeeded or failed. The boolean is purely informational
        // for local bookkeeping and is NOT sent to the OpenAI endpoint. Sending the nested object
        // form `{ content, success:false }` triggers the 400 we are still seeing. Mirror the JS CLI
        // exactly: always emit a bare string.

        serializer.serialize_str(&self.content)
    }
}

impl<'de> Deserialize<'de> for FunctionCallOutputPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(FunctionCallOutputPayload {
            content: s,
            success: None,
        })
    }
}

// Implement Display so callers can treat the payload like a plain string when logging or doing
// trivial substring checks in tests (existing tests call `.contains()` on the output). Display
// returns the raw `content` field.

impl std::fmt::Display for FunctionCallOutputPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.content)
    }
}

impl std::ops::Deref for FunctionCallOutputPayload {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.content
    }
}

// (Moved event mapping logic into codex-core to avoid coupling protocol to UI-facing events.)

/// Maximum file size we'll read (10MB).
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum number of entries to show in a directory tree.
const MAX_FOLDER_ENTRIES: usize = 500;

/// Default maximum depth for directory tree traversal.
const DEFAULT_MAX_DEPTH: usize = 3;

/// Read a file and format it with line numbers and metadata.
fn read_file_with_context(
    path: &std::path::Path,
    max_lines: Option<usize>,
) -> Result<String, std::io::Error> {
    use std::fs::File;
    use std::fs::metadata;
    use std::io::BufRead;
    use std::io::BufReader;
    use std::io::ErrorKind;

    let meta = metadata(path)?;
    let file_size = meta.len();

    // Check file size limit
    if file_size > MAX_FILE_SIZE {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("File too large: {file_size} bytes (max: {MAX_FILE_SIZE} bytes)"),
        ));
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let max = max_lines.unwrap_or(2000);

    let mut lines = Vec::new();
    let mut line_num = 1;
    let mut total_lines = 0;
    let mut truncated = false;

    for line_result in reader.lines() {
        total_lines += 1;
        if line_num <= max {
            let line = line_result?;
            // Truncate very long lines
            let truncated_line = if line.len() > 500 {
                format!("{}...", &line[..497])
            } else {
                line
            };
            lines.push(format!("{line_num:6}: {truncated_line}"));
            line_num += 1;
        } else if !truncated {
            truncated = true;
        }
    }

    let mut result = String::new();

    // Add file metadata header
    result.push_str(&format!("File: {}\n", path.display()));
    result.push_str(&format!("Size: {file_size} bytes, {total_lines} lines\n"));

    if truncated {
        result.push_str(&format!("(Showing first {max} of {total_lines} lines)\n"));
    }

    // Add language hint for syntax
    if let Some(ext) = path.extension() {
        result.push_str(&format!("```{}\n", ext.to_string_lossy()));
    } else {
        result.push_str("```\n");
    }

    result.push_str(&lines.join("\n"));
    result.push_str("\n```");

    Ok(result)
}

/// Read a folder and format it as a tree structure.
fn read_folder_with_context(
    path: &std::path::Path,
    max_depth: Option<usize>,
) -> Result<String, std::io::Error> {
    let max_depth = max_depth.unwrap_or(DEFAULT_MAX_DEPTH);

    let mut result = String::new();
    result.push_str(&format!("```tree\nFolder: {}\n", path.display()));

    let mut entry_count = 0;
    let walker = ignore::WalkBuilder::new(path)
        .max_depth(Some(max_depth))
        .build();

    let mut entries: Vec<(usize, String, bool)> = Vec::new();

    for entry_result in walker {
        if entry_count >= MAX_FOLDER_ENTRIES {
            result.push_str(&format!(
                "\n... (truncated, showing first {} entries)\n",
                MAX_FOLDER_ENTRIES
            ));
            break;
        }

        let entry = match entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };

        let depth = entry.depth();
        if depth == 0 {
            continue; // Skip root
        }

        let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
        let file_name = entry.file_name().to_string_lossy().to_string();

        entries.push((depth, file_name, is_dir));
        entry_count += 1;
    }

    // Format entries as tree
    for (depth, name, is_dir) in entries {
        let indent = "  ".repeat(depth.saturating_sub(1));
        let prefix = if depth > 0 { "├─ " } else { "" };
        let suffix = if is_dir { "/" } else { "" };
        result.push_str(&format!("{}{}{}{}\n", indent, prefix, name, suffix));
    }

    result.push_str("```");
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn test_read_file_with_context_formatting() -> Result<()> {
        use std::io::Write;
        let temp_dir = tempfile::tempdir()?;
        let file_path = temp_dir.path().join("test.rs");

        let mut file = std::fs::File::create(&file_path)?;
        writeln!(file, "fn main() {{")?;
        writeln!(file, "    println!(\"Hello, world!\");")?;
        writeln!(file, "    let x = 42;")?;
        writeln!(file, "}}")?;
        drop(file);

        let result = read_file_with_context(&file_path, Some(2000))?;

        // Verify the output contains expected components
        assert!(result.contains("File:"));
        assert!(result.contains("test.rs"));
        assert!(result.contains("Size:"));
        assert!(result.contains("bytes, 4 lines"));
        assert!(result.contains("```rs"));
        assert!(result.contains("     1: fn main() {"));
        assert!(result.contains("     2:     println!(\"Hello, world!\");"));
        assert!(result.contains("```"));

        println!("Formatted output:\n{result}");
        Ok(())
    }

    #[test]
    fn test_read_file_with_truncation() -> Result<()> {
        use std::io::Write;
        let temp_dir = tempfile::tempdir()?;
        let file_path = temp_dir.path().join("large.txt");

        let mut file = std::fs::File::create(&file_path)?;
        for i in 1..=100 {
            writeln!(file, "Line {i}")?;
        }
        drop(file);

        let result = read_file_with_context(&file_path, Some(50))?;

        assert!(result.contains("bytes, 100 lines"));
        assert!(result.contains("(Showing first 50 of 100 lines)"));
        assert!(result.contains("    50: Line 50"));
        assert!(!result.contains("Line 51"));

        Ok(())
    }

    #[test]
    fn test_read_file_too_large() {
        use std::io::Write;
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("huge.txt");

        let mut file = std::fs::File::create(&file_path).unwrap();
        // Write more than MAX_FILE_SIZE
        let large_data = vec![b'x'; (MAX_FILE_SIZE + 1) as usize];
        file.write_all(&large_data).unwrap();
        drop(file);

        let result = read_file_with_context(&file_path, Some(2000));
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("File too large"));
    }

    #[test]
    fn serializes_success_as_plain_string() -> Result<()> {
        let item = ResponseInputItem::FunctionCallOutput {
            call_id: "call1".into(),
            output: FunctionCallOutputPayload {
                content: "ok".into(),
                success: None,
            },
        };

        let json = serde_json::to_string(&item)?;
        let v: serde_json::Value = serde_json::from_str(&json)?;

        // Success case -> output should be a plain string
        assert_eq!(v.get("output").unwrap().as_str().unwrap(), "ok");
        Ok(())
    }

    #[test]
    fn serializes_failure_as_string() -> Result<()> {
        let item = ResponseInputItem::FunctionCallOutput {
            call_id: "call1".into(),
            output: FunctionCallOutputPayload {
                content: "bad".into(),
                success: Some(false),
            },
        };

        let json = serde_json::to_string(&item)?;
        let v: serde_json::Value = serde_json::from_str(&json)?;

        assert_eq!(v.get("output").unwrap().as_str().unwrap(), "bad");
        Ok(())
    }

    #[test]
    fn deserialize_shell_tool_call_params() -> Result<()> {
        let json = r#"{
            "command": ["ls", "-l"],
            "workdir": "/tmp",
            "timeout": 1000
        }"#;

        let params: ShellToolCallParams = serde_json::from_str(json)?;
        assert_eq!(
            ShellToolCallParams {
                command: vec!["ls".to_string(), "-l".to_string()],
                workdir: Some("/tmp".to_string()),
                timeout_ms: Some(1000),
                with_escalated_permissions: None,
                justification: None,
            },
            params
        );
        Ok(())
    }

    #[test]
    fn test_read_folder_with_context() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let base = temp_dir.path();

        // Create a test directory structure
        std::fs::create_dir(base.join("subdir"))?;
        std::fs::write(base.join("file1.txt"), "content")?;
        std::fs::write(base.join("subdir/file2.txt"), "content")?;

        let result = read_folder_with_context(base, Some(2))?;

        // Verify the output contains expected elements
        assert!(result.contains("```tree"));
        assert!(result.contains("Folder:"));
        assert!(result.contains("file1.txt"));
        assert!(result.contains("subdir/"));

        Ok(())
    }
}
