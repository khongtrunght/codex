//! Anthropic Messages API request builder.
//!
//! Converts canonical prompts to Anthropic's Messages API format:
//! - System prompt as array of content blocks
//! - Strict user/assistant message alternation
//! - Tools use `input_schema` instead of `parameters`
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

/// Assembled request body plus headers for Anthropic Messages API.
pub struct AnthropicRequest {
    pub body: Value,
    pub headers: HeaderMap,
}

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
        // Build system prompt as content blocks
        let system = vec![json!({
            "type": "text",
            "text": self.instructions
        })];

        // Convert ResponseItems to Anthropic messages format
        let messages = self.build_messages()?;

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
    /// Anthropic requires strict user/assistant alternation.
    fn build_messages(&self) -> Result<Vec<Value>, ApiError> {
        let mut messages: Vec<Value> = Vec::new();
        let mut pending_tool_results: Vec<Value> = Vec::new();
        let mut last_role: Option<&str> = None;

        for item in self.input {
            match item {
                ResponseItem::Message { role, content, .. } => {
                    // Flush any pending tool results before user message
                    if role == "user" && !pending_tool_results.is_empty() {
                        // If last message was user, we need an assistant placeholder
                        if last_role == Some("user") {
                            messages.push(json!({
                                "role": "assistant",
                                "content": [{"type": "text", "text": "I'll help with that."}]
                            }));
                        }
                        messages.push(json!({
                            "role": "user",
                            "content": std::mem::take(&mut pending_tool_results)
                        }));
                        last_role = Some("user");
                    }

                    let anthropic_content = self.convert_content(content);

                    // Handle alternation requirement
                    if last_role == Some(role.as_str()) {
                        // Same role twice - merge or insert placeholder
                        if role == "user" {
                            // Merge user messages
                            if let Some(last_msg) = messages.last_mut()
                                && let Some(content_arr) = last_msg.get_mut("content")
                                && let Some(arr) = content_arr.as_array_mut()
                            {
                                for c in anthropic_content {
                                    arr.push(c);
                                }
                                continue;
                            }
                        } else {
                            // Insert user placeholder between assistant messages
                            messages.push(json!({
                                "role": "user",
                                "content": [{"type": "text", "text": "Continue."}]
                            }));
                        }
                    }

                    messages.push(json!({
                        "role": role,
                        "content": anthropic_content
                    }));
                    last_role = Some(if role == "user" { "user" } else { "assistant" });
                }

                ResponseItem::FunctionCall {
                    name,
                    arguments,
                    call_id,
                    ..
                } => {
                    // Anthropic uses tool_use blocks within assistant messages
                    let input: Value = serde_json::from_str(arguments).unwrap_or(json!({}));

                    // Insert user placeholder if last was assistant
                    if last_role == Some("assistant") {
                        messages.push(json!({
                            "role": "user",
                            "content": [{"type": "text", "text": "Continue."}]
                        }));
                    }

                    messages.push(json!({
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": call_id,
                            "name": name,
                            "input": input
                        }]
                    }));
                    last_role = Some("assistant");
                }

                ResponseItem::LocalShellCall { id, action, .. } => {
                    // Convert local shell call to Anthropic tool_use
                    let input =
                        serde_json::to_value(action).unwrap_or_else(|_| json!({"command": []}));

                    // Insert user placeholder if last was assistant
                    if last_role == Some("assistant") {
                        messages.push(json!({
                            "role": "user",
                            "content": [{"type": "text", "text": "Continue."}]
                        }));
                    }

                    messages.push(json!({
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": id.clone().unwrap_or_else(|| "shell_call".to_string()),
                            "name": "shell",
                            "input": input
                        }]
                    }));
                    last_role = Some("assistant");
                }

                ResponseItem::FunctionCallOutput { call_id, output } => {
                    // Anthropic uses tool_result blocks in user messages
                    let content = if let Some(items) = &output.content_items {
                        let parts: Vec<Value> = items
                            .iter()
                            .map(|it| match it {
                                FunctionCallOutputContentItem::InputText { text } => {
                                    json!({"type": "text", "text": text})
                                }
                                FunctionCallOutputContentItem::InputImage { image_url } => {
                                    // Anthropic uses base64 images differently
                                    if image_url.starts_with("data:") {
                                        // Parse data URL
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
                            .collect();
                        json!(parts)
                    } else {
                        json!([{"type": "text", "text": &output.content}])
                    };

                    pending_tool_results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": content
                    }));
                }

                ResponseItem::CustomToolCall {
                    id, name, input, ..
                } => {
                    // Insert user placeholder if last was assistant
                    if last_role == Some("assistant") {
                        messages.push(json!({
                            "role": "user",
                            "content": [{"type": "text", "text": "Continue."}]
                        }));
                    }

                    messages.push(json!({
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input
                        }]
                    }));
                    last_role = Some("assistant");
                }

                ResponseItem::CustomToolCallOutput { call_id, output } => {
                    pending_tool_results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": [{"type": "text", "text": output}]
                    }));
                }

                // Skip items that don't translate to messages
                ResponseItem::Reasoning { .. }
                | ResponseItem::WebSearchCall { .. }
                | ResponseItem::GhostSnapshot { .. }
                | ResponseItem::Compaction { .. }
                | ResponseItem::Other => continue,
            }
        }

        // Flush any remaining tool results
        if !pending_tool_results.is_empty() {
            messages.push(json!({
                "role": "user",
                "content": pending_tool_results
            }));
        }

        // Ensure messages start with user and alternate
        self.ensure_valid_alternation(&mut messages);

        Ok(messages)
    }

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
        assert_eq!(request.body["system"][0]["text"], "Be helpful");

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
}
