//! Anthropic Messages API request builder.
//!
//! Converts canonical prompts to Anthropic's Messages API format:
//! - System prompt as array of content blocks with cache control
//! - Strict user/assistant message alternation
//! - Tools use `input_schema` instead of `parameters`
//! - Cache control on tail messages for prompt caching
//!
//! See: https://docs.anthropic.com/claude/reference/messages

use crate::error::ApiError;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ResponseItem;
use http::HeaderMap;
use http::HeaderValue;
use serde_json::Value;
use serde_json::json;

/// Anthropic API version header value
pub const ANTHROPIC_API_VERSION: &str = "2023-06-01";

/// Default max tokens for Anthropic requests (required field)
pub const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Maximum number of cache breakpoints allowed by Anthropic API
const MAX_CACHE_BREAKPOINTS: usize = 4;

/// Number of tail messages to mark for caching
const CACHE_TAIL_MESSAGES: usize = 2;

/// Assembled request body plus headers for Anthropic Messages API.
pub struct AnthropicRequest {
    pub body: Value,
    pub headers: HeaderMap,
}

/// System prompt header for Claude Code identification
pub const CLAUDE_CODE_SYSTEM_HEADER: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

/// Builder for Anthropic Messages API requests.
pub struct AnthropicRequestBuilder<'a> {
    model: &'a str,
    instructions: &'a str,
    input: &'a [ResponseItem],
    tools: &'a [Value],
    max_tokens: u32,
}

impl<'a> AnthropicRequestBuilder<'a> {
    /// Create a new request builder.
    pub fn new(
        model: &'a str,
        instructions: &'a str,
        input: &'a [ResponseItem],
        tools: &'a [Value],
    ) -> Self {
        Self {
            model,
            instructions,
            input,
            tools,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    /// Set the maximum tokens for the response.
    pub fn max_tokens(mut self, tokens: u32) -> Self {
        self.max_tokens = tokens;
        self
    }

    /// Build the Anthropic request.
    pub fn build(self) -> Result<AnthropicRequest, ApiError> {
        // Build system prompt as content blocks with cache control
        // Both blocks get cache_control for optimal caching
        let system = vec![
            Self::build_system_block(CLAUDE_CODE_SYSTEM_HEADER),
            Self::build_system_block(self.instructions),
        ];

        // Convert ResponseItems to Anthropic messages format
        let mut messages = self.build_messages()?;

        // Mark tail messages for caching (uses remaining cache breakpoints)
        Self::mark_tail_messages_for_cache(&mut messages, CACHE_TAIL_MESSAGES);

        // Convert tools to Anthropic format
        let tools = self.convert_tools();

        // Build request body
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "stream": true,
            "system": system,
            "messages": messages
        });

        // Add tools if any
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        // Build headers
        let mut headers = HeaderMap::new();
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_API_VERSION),
        );
        headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        Ok(AnthropicRequest { body, headers })
    }

    /// Convert ResponseItems to Anthropic messages format.
    ///
    /// Follows the simple 1:1 mapping pattern from CLIProxyAPI:
    /// - message → user/assistant message based on role
    /// - function_call → assistant message with tool_use
    /// - function_call_output → user message with tool_result
    ///
    /// After processing, consecutive messages of the same role are merged.
    fn build_messages(&self) -> Result<Vec<Value>, ApiError> {
        let mut messages: Vec<Value> = Vec::new();

        // Simple 1:1 mapping: each input item → one message
        for item in self.input {
            match item {
                ResponseItem::Message { role, content, .. } => {
                    let anthropic_content = self.convert_content(content);
                    let msg_role = if role == "user" { "user" } else { "assistant" };
                    messages.push(json!({
                        "role": msg_role,
                        "content": anthropic_content
                    }));
                }

                ResponseItem::FunctionCall {
                    name,
                    arguments,
                    call_id,
                    ..
                } => {
                    // Map to assistant tool_use (like Go: case "function_call")
                    let input: Value = Self::convert_function_call_input(name, arguments);
                    tracing::info!(
                        "Anthropic tool_use input for call_id {}: {}",
                        call_id,
                        input
                    );
                    messages.push(json!({
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": call_id,
                            "name": name,
                            "input": input
                        }]
                    }));
                }

                ResponseItem::FunctionCallOutput { call_id, output } => {
                    // Map to user tool_result (like Go: case "function_call_output")
                    let content = self.convert_function_call_output(output);
                    messages.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": call_id,
                            "content": content
                        }]
                    }));
                }

                ResponseItem::LocalShellCall { id, action, .. } => {
                    // Convert local shell call to assistant tool_use
                    let input =
                        serde_json::to_value(action).unwrap_or_else(|_| json!({"command": []}));
                    messages.push(json!({
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": id.clone().unwrap_or_else(|| "shell_call".to_string()),
                            "name": "shell",
                            "input": input
                        }]
                    }));
                }

                ResponseItem::CustomToolCall {
                    call_id, name, input, ..
                } => {
                    // Map to assistant tool_use
                    // input is a String - parse as JSON or wrap for apply_patch
                    let tool_input = if name == "apply_patch" {
                        // apply_patch needs {"input": "patch content"}
                        json!({"input": input})
                    } else {
                        // Try to parse as JSON, fallback to wrapping
                        serde_json::from_str(input).unwrap_or_else(|_| json!({"input": input}))
                    };
                    // Use call_id (not id) to match tool_result's tool_use_id
                    messages.push(json!({
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": call_id,
                            "name": name,
                            "input": tool_input
                        }]
                    }));
                }

                ResponseItem::CustomToolCallOutput { call_id, output } => {
                    // Map to user tool_result
                    messages.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": call_id,
                            "content": [{"type": "text", "text": output}]
                        }]
                    }));
                }

                // Skip items that don't translate to messages
                // Attachment should be expanded before reaching here
                ResponseItem::Reasoning { .. }
                | ResponseItem::WebSearchCall { .. }
                | ResponseItem::GhostSnapshot { .. }
                | ResponseItem::Compaction { .. }
                | ResponseItem::Attachment { .. }
                | ResponseItem::Other => continue,
            }
        }

        // Merge consecutive messages of the same role
        messages = Self::merge_consecutive_messages(messages);

        // Ensure messages start with user and alternate
        self.ensure_valid_alternation(&mut messages);

        Ok(messages)
    }

    /// Merge consecutive messages of the same role into single messages.
    /// This ensures proper alternation for Anthropic's API.
    fn merge_consecutive_messages(messages: Vec<Value>) -> Vec<Value> {
        if messages.is_empty() {
            return messages;
        }

        let mut merged: Vec<Value> = Vec::new();

        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let content = msg.get("content").cloned().unwrap_or(json!([]));

            // Check if we can merge with the last message
            if let Some(last) = merged.last_mut() {
                let last_role = last.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if last_role == role {
                    // Same role - merge content arrays
                    if let Some(last_content) = last.get_mut("content")
                        && let Some(arr) = last_content.as_array_mut()
                        && let Some(new_content) = content.as_array()
                    {
                        for item in new_content {
                            arr.push(item.clone());
                        }
                    }
                    continue;
                }
            }

            // Different role or first message - add as new
            merged.push(msg);
        }

        merged
    }

    // =========================================================================
    // Cache Control Helpers
    // =========================================================================

    /// Build a system block with cache control.
    /// Following the pattern from anthropic_messages.rs
    fn build_system_block(text: &str) -> Value {
        json!({
            "type": "text",
            "text": text,
            "cache_control": {
                "type": "ephemeral"
            }
        })
    }

    /// Attach cache_control to a content block.
    fn attach_cache_control(content_block: &mut Value) {
        if let Some(obj) = content_block.as_object_mut() {
            obj.insert("cache_control".to_string(), json!({"type": "ephemeral"}));
        }
    }

    /// Mark the last N messages with cache_control on their final content block.
    /// This enables prompt caching for frequently repeated conversation prefixes.
    ///
    /// Anthropic allows maximum 4 cache breakpoints per request.
    /// We use 2 for system blocks, leaving 2 for tail messages.
    fn mark_tail_messages_for_cache(messages: &mut [Value], n_messages: usize) {
        // Respect the maximum cache breakpoints (minus 2 used for system blocks)
        let available_breakpoints = MAX_CACHE_BREAKPOINTS.saturating_sub(2);
        let n_to_mark = n_messages.min(available_breakpoints);

        for msg in messages.iter_mut().rev().take(n_to_mark) {
            if let Some(content) = msg.get_mut("content") {
                match content {
                    Value::Array(blocks) => {
                        // Mark the last block in the content array
                        if let Some(last_block) = blocks.last_mut() {
                            Self::attach_cache_control(last_block);
                        }
                    }
                    Value::String(text) => {
                        // Convert string shorthand to array form with cache marker
                        let text_clone = text.clone();
                        *content = json!([{
                            "type": "text",
                            "text": text_clone,
                            "cache_control": {"type": "ephemeral"}
                        }]);
                    }
                    _ => {}
                }
            }
        }
    }

    // =========================================================================
    // Content Conversion Helpers
    // =========================================================================

    /// Convert content items to Anthropic format.
    fn convert_content(&self, content: &[ContentItem]) -> Vec<Value> {
        content
            .iter()
            .map(|c| match c {
                ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                    json!({"type": "text", "text": text})
                }
                ContentItem::InputImage { image_url } => {
                    if image_url.starts_with("data:") {
                        // Parse data URL for base64 image
                        let parts: Vec<&str> = image_url.splitn(2, ',').collect();
                        if parts.len() == 2 {
                            let media_type = parts[0]
                                .strip_prefix("data:")
                                .and_then(|s| s.strip_suffix(";base64"))
                                .unwrap_or("image/png");
                            json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": media_type,
                                    "data": parts[1]
                                }
                            })
                        } else {
                            json!({"type": "text", "text": "[Image]"})
                        }
                    } else {
                        // URL-based image
                        json!({
                            "type": "image",
                            "source": {
                                "type": "url",
                                "url": image_url
                            }
                        })
                    }
                }
            })
            .collect()
    }

    /// Convert function call arguments to Anthropic input format.
    ///
    /// For apply_patch: wrap in `{"input": ...}` since Anthropic expects object input.
    /// For other tools: parse as JSON directly (original behavior).
    fn convert_function_call_input(name: &str, arguments: &str) -> Value {
        if name == "apply_patch" {
            // apply_patch expects {"input": "patch content"}
            json!({"input": arguments})
        } else {
            // Original behavior for other tools
            serde_json::from_str(arguments).unwrap_or(json!({}))
        }
    }

    /// Convert function call output to Anthropic content format.
    fn convert_function_call_output(
        &self,
        output: &codex_protocol::models::FunctionCallOutputPayload,
    ) -> Value {
        if let Some(items) = &output.content_items {
            let parts: Vec<Value> = items
                .iter()
                .map(|it| match it {
                    FunctionCallOutputContentItem::InputText { text } => {
                        json!({"type": "text", "text": text})
                    }
                    FunctionCallOutputContentItem::InputImage { image_url } => {
                        if image_url.starts_with("data:") {
                            let parts: Vec<&str> = image_url.splitn(2, ',').collect();
                            if parts.len() == 2 {
                                let media_type = parts[0]
                                    .strip_prefix("data:")
                                    .and_then(|s| s.strip_suffix(";base64"))
                                    .unwrap_or("image/png");
                                json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": media_type,
                                        "data": parts[1]
                                    }
                                })
                            } else {
                                json!({"type": "text", "text": "[Image]"})
                            }
                        } else {
                            json!({
                                "type": "image",
                                "source": {
                                    "type": "url",
                                    "url": image_url
                                }
                            })
                        }
                    }
                })
                .collect();
            json!(parts)
        } else {
            json!([{"type": "text", "text": &output.content}])
        }
    }

    /// Convert OpenAI-style tools to Anthropic format.
    fn convert_tools(&self) -> Vec<Value> {
        self.tools
            .iter()
            .filter_map(|tool| {
                // OpenAI format: {"type": "function", "function": {"name": ..., "description": ..., "parameters": ...}}
                let func = tool.get("function")?;
                let name = func.get("name")?.as_str()?;
                let description = func.get("description").and_then(|d| d.as_str());
                let parameters = func.get("parameters").cloned().unwrap_or(json!({}));

                // Anthropic format: {"name": ..., "description": ..., "input_schema": ...}
                let mut anthropic_tool = json!({
                    "name": name,
                    "input_schema": parameters
                });

                if let Some(desc) = description {
                    anthropic_tool["description"] = json!(desc);
                }

                Some(anthropic_tool)
            })
            .collect()
    }

    /// Ensure messages follow Anthropic's alternation requirements.
    fn ensure_valid_alternation(&self, messages: &mut Vec<Value>) {
        if messages.is_empty() {
            return;
        }

        // Must start with user
        if let Some(first) = messages.first()
            && first.get("role").and_then(|r| r.as_str()) != Some("user")
        {
            messages.insert(
                0,
                json!({
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello."}]
                }),
            );
        }

        // Must end with user (for tool calls, this is already handled)
        // Note: Anthropic actually requires ending with user OR assistant depending on context
        // For streaming, we typically end with user waiting for assistant response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_basic_message_conversion() {
        let input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Hello".to_string(),
            }],
        }];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .build()
            .expect("should build");

        assert_eq!(request.body["model"], "claude-3-sonnet");
        // system[0] is the Claude Code header, system[1] is the instructions
        assert_eq!(request.body["system"][0]["text"], CLAUDE_CODE_SYSTEM_HEADER);
        assert_eq!(request.body["system"][1]["text"], "Be helpful");

        let messages = request.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["text"], "Hello");
    }

    #[test]
    fn test_tool_conversion() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Run a shell command",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string"}
                    },
                    "required": ["command"]
                }
            }
        })];

        let input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "List files".to_string(),
            }],
        }];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &tools)
            .build()
            .expect("should build");

        let anthropic_tools = request.body["tools"].as_array().unwrap();
        assert_eq!(anthropic_tools.len(), 1);
        assert_eq!(anthropic_tools[0]["name"], "shell");
        assert_eq!(anthropic_tools[0]["description"], "Run a shell command");
        assert!(anthropic_tools[0]["input_schema"]["properties"]["command"].is_object());
    }

    #[test]
    fn test_function_call_and_output() {
        let input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "List files".to_string(),
                }],
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                arguments: r#"{"command":"ls"}"#.to_string(),
                call_id: "call_123".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call_123".to_string(),
                output: codex_protocol::models::FunctionCallOutputPayload {
                    success: Some(true),
                    content: "file1.txt\nfile2.txt".to_string(),
                    content_items: None,
                },
            },
        ];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .build()
            .expect("should build");

        let messages = request.body["messages"].as_array().unwrap();

        // Should have: user, assistant (tool_use), user (tool_result)
        assert_eq!(messages.len(), 3);

        // Check assistant message has tool_use
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["id"], "call_123");
        assert_eq!(messages[1]["content"][0]["name"], "shell");

        // Check user message has tool_result
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "call_123");
    }

    #[test]
    fn test_headers() {
        let input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Hello".to_string(),
            }],
        }];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .build()
            .expect("should build");

        assert_eq!(
            request.headers.get("anthropic-version").unwrap(),
            ANTHROPIC_API_VERSION
        );
        assert_eq!(
            request.headers.get("content-type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_max_tokens() {
        let input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Hello".to_string(),
            }],
        }];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .max_tokens(4096)
            .build()
            .expect("should build");

        assert_eq!(request.body["max_tokens"], 4096);
    }

    #[test]
    fn test_assistant_text_and_tool_call_combined() {
        // This tests the fix for the "Continue." bug - assistant text and tool_use
        // should be combined into a single message, not split with "Continue." between them
        let input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Create a file".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "I'll create a file.".to_string(),
                }],
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                arguments: r#"{"command":"touch test.py"}"#.to_string(),
                call_id: "call_123".to_string(),
            },
        ];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .build()
            .expect("should build");

        let messages = request.body["messages"].as_array().unwrap();

        // Should have: user, assistant (text + tool_use combined)
        // NOT: user, assistant (text), user (Continue.), assistant (tool_use)
        assert_eq!(
            messages.len(),
            2,
            "Expected 2 messages, got {}: {:?}",
            messages.len(),
            messages
        );

        // First message is user
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["text"], "Create a file");

        // Second message is assistant with BOTH text and tool_use
        assert_eq!(messages[1]["role"], "assistant");
        let content = messages[1]["content"].as_array().unwrap();
        assert_eq!(
            content.len(),
            2,
            "Expected 2 content blocks in assistant message"
        );
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "I'll create a file.");
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["name"], "shell");

        // Verify no "Continue." exists anywhere
        for msg in messages {
            if let Some(content) = msg["content"].as_array() {
                for part in content {
                    if part["type"] == "text" {
                        assert_ne!(
                            part["text"], "Continue.",
                            "Found unexpected 'Continue.' message"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_multiple_tool_calls_combined() {
        // Multiple tool calls from the same assistant turn should be combined
        let input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Create two files".to_string(),
                }],
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                arguments: r#"{"command":"touch file1.txt"}"#.to_string(),
                call_id: "call_1".to_string(),
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                arguments: r#"{"command":"touch file2.txt"}"#.to_string(),
                call_id: "call_2".to_string(),
            },
        ];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .build()
            .expect("should build");

        let messages = request.body["messages"].as_array().unwrap();

        // Should have: user, assistant (both tool_use blocks combined)
        assert_eq!(messages.len(), 2);

        // Second message should have both tool_use blocks
        assert_eq!(messages[1]["role"], "assistant");
        let content = messages[1]["content"].as_array().unwrap();
        assert_eq!(
            content.len(),
            2,
            "Expected 2 tool_use blocks in assistant message"
        );
        assert_eq!(content[0]["type"], "tool_use");
        assert_eq!(content[0]["id"], "call_1");
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["id"], "call_2");
    }

    #[test]
    fn test_consecutive_messages_merged() {
        // Following CLIProxyAPI pattern: simple 1:1 mapping then merge consecutive same-role.
        // All consecutive assistant messages are merged into one.
        // All consecutive user messages (tool_results) are merged into one.
        let input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Find Rust files".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "I'll search for Rust files.".to_string(),
                }],
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                arguments: r#"{"command":"rg AgentTypeConfig"}"#.to_string(),
                call_id: "call_A".to_string(),
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "glob".to_string(),
                arguments: r#"{"pattern":"**/*.rs"}"#.to_string(),
                call_id: "call_B".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call_A".to_string(),
                output: codex_protocol::models::FunctionCallOutputPayload {
                    success: Some(true),
                    content: "found matches".to_string(),
                    content_items: None,
                },
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call_B".to_string(),
                output: codex_protocol::models::FunctionCallOutputPayload {
                    success: Some(true),
                    content: "file1.rs\nfile2.rs".to_string(),
                    content_items: None,
                },
            },
        ];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .build()
            .expect("should build");

        let messages = request.body["messages"].as_array().unwrap();

        // Should have: user, assistant (all merged), user (all tool_results merged)
        assert_eq!(messages.len(), 3, "Expected 3 messages: {:?}", messages);

        // First message is user
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["text"], "Find Rust files");

        // Second message is assistant with text + all tool_uses merged
        assert_eq!(messages[1]["role"], "assistant");
        let asst_content = messages[1]["content"].as_array().unwrap();
        assert_eq!(asst_content.len(), 3, "Expected text + 2 tool_uses");
        assert_eq!(asst_content[0]["type"], "text");
        assert_eq!(asst_content[1]["type"], "tool_use");
        assert_eq!(asst_content[1]["id"], "call_A");
        assert_eq!(asst_content[2]["type"], "tool_use");
        assert_eq!(asst_content[2]["id"], "call_B");

        // Third message is user with all tool_results merged
        assert_eq!(messages[2]["role"], "user");
        let user_content = messages[2]["content"].as_array().unwrap();
        assert_eq!(user_content.len(), 2, "Expected 2 tool_results");
        assert_eq!(user_content[0]["type"], "tool_result");
        assert_eq!(user_content[0]["tool_use_id"], "call_A");
        assert_eq!(user_content[1]["type"], "tool_result");
        assert_eq!(user_content[1]["tool_use_id"], "call_B");

        // Verify no "Continue." exists anywhere
        for msg in messages {
            if let Some(content) = msg["content"].as_array() {
                for part in content {
                    if part["type"] == "text" {
                        let text = part["text"].as_str().unwrap_or("");
                        assert_ne!(text, "Continue.", "Found unexpected 'Continue.' message");
                    }
                }
            }
        }
    }

    #[test]
    fn test_cache_control_on_system_and_tail_messages() {
        // Verify cache_control is properly applied to:
        // 1. Both system blocks (ephemeral)
        // 2. Last 2 messages (tail caching)
        let input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Hello".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "Hi there!".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "How are you?".to_string(),
                }],
            },
        ];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .build()
            .expect("should build");

        // Check system blocks have cache_control
        let system = request.body["system"].as_array().unwrap();
        assert_eq!(system.len(), 2);
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(system[1]["cache_control"]["type"], "ephemeral");

        // Check tail messages have cache_control on last content block
        let messages = request.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);

        // Last 2 messages should have cache_control
        // messages[1] (assistant) - last content block should have cache_control
        let asst_content = messages[1]["content"].as_array().unwrap();
        let last_asst_block = asst_content.last().unwrap();
        assert_eq!(
            last_asst_block["cache_control"]["type"], "ephemeral",
            "Assistant message should have cache_control"
        );

        // messages[2] (user) - last content block should have cache_control
        let user_content = messages[2]["content"].as_array().unwrap();
        let last_user_block = user_content.last().unwrap();
        assert_eq!(
            last_user_block["cache_control"]["type"], "ephemeral",
            "Last user message should have cache_control"
        );

        // messages[0] (first user) should NOT have cache_control (only last 2 are marked)
        let first_user_content = messages[0]["content"].as_array().unwrap();
        let first_block = &first_user_content[0];
        assert!(
            first_block.get("cache_control").is_none(),
            "First message should NOT have cache_control"
        );
    }

    #[test]
    fn test_apply_patch_tool_input_format() {
        // apply_patch tool should have input wrapped as {"input": "patch content"}
        // NOT as a raw string
        let patch_content = "*** Begin Patch\n*** Add File: temp.txt\n+hello world\n*** End Patch";
        let input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Create a file".to_string(),
                }],
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "apply_patch".to_string(),
                arguments: patch_content.to_string(), // Raw patch string, not JSON
                call_id: "call_patch_123".to_string(),
            },
        ];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .build()
            .expect("should build");

        let messages = request.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);

        // Check assistant message has tool_use with properly wrapped input
        let tool_use = &messages[1]["content"][0];
        assert_eq!(tool_use["type"], "tool_use");
        assert_eq!(tool_use["name"], "apply_patch");

        // CRITICAL: input must be an object {"input": "..."}, NOT a raw string
        let tool_input = &tool_use["input"];
        assert!(
            tool_input.is_object(),
            "apply_patch input must be an object, got: {}",
            tool_input
        );
        assert_eq!(
            tool_input["input"], patch_content,
            "apply_patch input.input should contain the patch content"
        );
    }

    #[test]
    fn test_regular_tool_input_format() {
        // Regular tools (not apply_patch) should parse JSON arguments directly
        let input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "List files".to_string(),
                }],
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                arguments: r#"{"command":"ls -la"}"#.to_string(),
                call_id: "call_shell_123".to_string(),
            },
        ];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .build()
            .expect("should build");

        let messages = request.body["messages"].as_array().unwrap();
        let tool_use = &messages[1]["content"][0];

        assert_eq!(tool_use["type"], "tool_use");
        assert_eq!(tool_use["name"], "shell");

        // Regular tool input should be parsed JSON object
        let tool_input = &tool_use["input"];
        assert!(tool_input.is_object());
        assert_eq!(tool_input["command"], "ls -la");
    }

    #[test]
    fn test_custom_tool_call_apply_patch_format() {
        // CustomToolCall with apply_patch should also wrap input correctly
        // Use different id vs call_id to verify we use call_id for tool_use.id
        let patch_content = "*** Begin Patch\n*** Add File: test.txt\n+hello\n*** End Patch";
        let input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Create a file".to_string(),
                }],
            },
            ResponseItem::CustomToolCall {
                id: Some("internal_item_id".to_string()), // Internal ID (not used)
                status: None,
                call_id: "call_abc123".to_string(), // This should be used for tool_use.id
                name: "apply_patch".to_string(),
                input: patch_content.to_string(),
            },
        ];

        let request = AnthropicRequestBuilder::new("claude-3-sonnet", "Be helpful", &input, &[])
            .build()
            .expect("should build");

        let messages = request.body["messages"].as_array().unwrap();
        let tool_use = &messages[1]["content"][0];

        assert_eq!(tool_use["type"], "tool_use");
        assert_eq!(tool_use["name"], "apply_patch");
        // CRITICAL: tool_use.id must be call_id, not internal id
        assert_eq!(
            tool_use["id"], "call_abc123",
            "tool_use.id should be call_id, not internal id"
        );

        // CRITICAL: input must be {"input": "..."}, NOT raw string
        let tool_input = &tool_use["input"];
        assert!(
            tool_input.is_object(),
            "CustomToolCall apply_patch input must be object, got: {}",
            tool_input
        );
        assert_eq!(tool_input["input"], patch_content);
    }
}
