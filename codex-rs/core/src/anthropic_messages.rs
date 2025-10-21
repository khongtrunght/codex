use std::time::Duration;

use crate::AuthManager;
use crate::ModelProviderInfo;
use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::error::CodexErr;
use crate::error::ConnectionFailedError;
use crate::error::Result;
use crate::error::UnexpectedResponseError;
use crate::model_family::ModelFamily;
use crate::protocol::TokenUsage;
use crate::util::backoff;
use bytes::Bytes;
use codex_otel::otel_event_manager::OtelEventManager;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use eventsource_stream::Eventsource;
use futures::Stream;
use futures::StreamExt;
use futures::TryStreamExt;
use reqwest::StatusCode;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

const DEFAULT_MAX_TOKENS: u32 = 4096;
const CLAUDE_CODE_SYSTEM_PROMPT: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// Implementation for the Anthropic Messages API.
pub(crate) async fn stream_anthropic_messages(
    prompt: &Prompt,
    model_family: &ModelFamily,
    client: &reqwest::Client,
    provider: &ModelProviderInfo,
    auth_manager: &Option<Arc<AuthManager>>,
    otel_event_manager: &OtelEventManager,
) -> Result<ResponseStream> {
    if prompt.output_schema.is_some() {
        return Err(CodexErr::UnsupportedOperation(
            "output_schema is not supported for Anthropic Messages API".to_string(),
        ));
    }

    // Build the Anthropic request
    let (system_prompt, messages) = build_anthropic_messages(prompt, model_family)?;
    let tools_json = build_anthropic_tools(&prompt.tools)?;

    // For now, use default max_tokens. Will be configurable via ModelFamily in Phase 4
    let max_tokens = DEFAULT_MAX_TOKENS;

    let mut payload = json!({
        "model": model_family.slug,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": true,
    });

    let system_blocks = if !system_prompt.is_empty() {
        vec![
            build_system_block(CLAUDE_CODE_SYSTEM_PROMPT, CacheType::Ephemeral),
            build_system_block(system_prompt, CacheType::Ephemeral),
        ]
    } else {
        vec![build_system_block(
            CLAUDE_CODE_SYSTEM_PROMPT,
            CacheType::Ephemeral,
        )]
    };

    payload["system"] = json!(system_blocks);

    // Add tools if present
    if !tools_json.is_empty() {
        payload["tools"] = json!(tools_json);
        payload["tool_choice"] = json!({"type": "auto"});
    }

    // Get auth from manager
    let auth = auth_manager.as_ref().and_then(|m| m.auth());

    debug!(
        "POST to {}: {}",
        provider.get_full_url(&auth),
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );

    let mut attempt = 0;
    let max_retries = provider.request_max_retries();
    loop {
        attempt += 1;

        let request_builder = provider.create_request_builder(client, &auth).await?;
        let response = request_builder.json(&payload).send().await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let idle_timeout = provider.stream_idle_timeout();
                let (tx, rx) = mpsc::channel::<Result<ResponseEvent>>(16);

                let byte_stream = resp
                    .bytes_stream()
                    .map_err(|e| std::io::Error::other(e.to_string()));

                let otel_manager_clone = otel_event_manager.clone();
                tokio::spawn(async move {
                    process_anthropic_sse(byte_stream, tx, idle_timeout, otel_manager_clone).await;
                });

                return Ok(ResponseStream { rx_event: rx });
            }

            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();

                if should_retry_status(status) && attempt <= max_retries {
                    let delay = backoff(attempt);
                    debug!(
                        "Anthropic API returned {status}, retrying in {delay:?} (attempt {attempt}/{max_retries})"
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                } else {
                    return Err(CodexErr::UnexpectedStatus(UnexpectedResponseError {
                        status,
                        body,
                        request_id: None,
                    }));
                }
            }

            Err(err) if attempt <= max_retries => {
                let delay = backoff(attempt);
                debug!(
                    "Anthropic API connection failed: {err}, retrying in {delay:?} (attempt {attempt}/{max_retries})"
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            Err(err) => {
                return Err(CodexErr::ConnectionFailed(ConnectionFailedError {
                    source: err,
                }));
            }
        }
    }
}

fn should_retry_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    ) || status.as_u16() == 529 // Anthropic's overloaded status
}

/// Convert Codex Prompt to Anthropic Messages API format.
/// Returns (system_prompt, messages_array)
fn build_anthropic_messages(
    prompt: &Prompt,
    model_family: &ModelFamily,
) -> Result<(String, Vec<serde_json::Value>)> {
    let system_prompt = prompt.get_full_instructions(model_family).to_string();
    let input = prompt.get_formatted_input();

    let mut messages = Vec::<serde_json::Value>::new();
    let mut current_role: Option<String> = None;
    let mut current_content: Vec<serde_json::Value> = Vec::new();

    for item in &input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                // Skip system messages - they're handled separately
                if role == "system" {
                    continue;
                }

                // Anthropic requires strict user/assistant alternation
                // If role changes, flush current message
                if let Some(ref prev_role) = current_role
                    && prev_role != role
                {
                    flush_message(&mut messages, prev_role, &mut current_content);
                }

                // Convert content items
                for item in content {
                    match item {
                        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                            current_content.push(json!({"type": "text", "text": text}));
                        }
                        _ => {
                            // Skip other content types for now
                        }
                    }
                }

                current_role = Some(role.clone());
            }

            ResponseItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                // Flush any pending user message
                if let Some(ref role) = current_role
                    && role == "user"
                {
                    flush_message(&mut messages, role, &mut current_content);
                }

                // Tool calls are part of assistant messages
                current_content.push(json!({
                    "type": "tool_use",
                    "id": call_id,
                    "name": name,
                    "input": serde_json::from_str::<serde_json::Value>(arguments)
                        .unwrap_or(json!({}))
                }));

                current_role = Some("assistant".to_string());
            }

            ResponseItem::FunctionCallOutput { call_id, output } => {
                // Flush any pending assistant message
                if let Some(ref role) = current_role
                    && role == "assistant"
                {
                    flush_message(&mut messages, role, &mut current_content);
                }

                // Tool results go in user messages
                current_content.push(json!({
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": output.content
                }));

                current_role = Some("user".to_string());
            }

            ResponseItem::LocalShellCall { .. }
            | ResponseItem::CustomToolCall { .. }
            | ResponseItem::CustomToolCallOutput { .. }
            | ResponseItem::Reasoning { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::Other => {
                // Skip these for now - to be implemented in later phases
                continue;
            }
        }
    }

    // Flush any remaining content
    if let Some(ref role) = current_role {
        flush_message(&mut messages, role, &mut current_content);
    }

    mark_tail_messages_for_cache(&mut messages, 2, CacheType::Ephemeral);

    Ok((system_prompt, messages))
}

fn flush_message(
    messages: &mut Vec<serde_json::Value>,
    role: &str,
    content: &mut Vec<serde_json::Value>,
) {
    if content.is_empty() {
        return;
    }

    let message = if content.len() == 1 && content[0].get("type") == Some(&json!("text")) {
        // Use string shorthand for simple text messages
        json!({
            "role": role,
            "content": content[0].get("text").unwrap_or(&json!("")).as_str().unwrap_or("")
        })
    } else {
        json!({
            "role": role,
            "content": content.clone()
        })
    };

    messages.push(message);
    content.clear();
}

/// Convert Codex tools to Anthropic format.
fn build_anthropic_tools(
    tools: &[crate::client_common::tools::ToolSpec],
) -> Result<Vec<serde_json::Value>> {
    let mut anthropic_tools = Vec::new();

    for tool in tools {
        if let crate::client_common::tools::ToolSpec::Function(func_tool) = tool {
            anthropic_tools.push(json!({
                "name": func_tool.name,
                "description": func_tool.description,
                "input_schema": func_tool.parameters
            }));
        }
    }

    Ok(anthropic_tools)
}

/// Process Anthropic SSE stream and convert to ResponseEvent stream.
async fn process_anthropic_sse<S>(
    stream: S,
    tx_event: mpsc::Sender<Result<ResponseEvent>>,
    idle_timeout: Duration,
    _otel_event_manager: OtelEventManager,
) where
    S: Stream<Item = std::io::Result<Bytes>> + Unpin,
{
    let mut stream = stream.eventsource();
    let mut state = StreamingState::default();

    loop {
        let response = timeout(idle_timeout, stream.next()).await;

        let sse = match response {
            Ok(Some(Ok(ev))) => ev,
            Ok(Some(Err(e))) => {
                let _ = tx_event
                    .send(Err(CodexErr::Stream(e.to_string(), None)))
                    .await;
                return;
            }
            Ok(None) => {
                // Stream closed
                if !state.completed {
                    let _ = tx_event
                        .send(Ok(ResponseEvent::Completed {
                            response_id: state.message_id.clone(),
                            token_usage: state.get_usage(),
                        }))
                        .await;
                }
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(CodexErr::Stream("idle timeout".to_string(), None)))
                    .await;
                return;
            }
        };

        trace!("Anthropic SSE event: {} - {}", sse.event, sse.data);

        // Parse event data
        let event_data: serde_json::Value = match serde_json::from_str(&sse.data) {
            Ok(v) => v,
            Err(e) => {
                debug!("Failed to parse Anthropic event data: {e}");
                continue;
            }
        };

        if let Err(e) = handle_event(&mut state, event_data, &tx_event).await {
            let _ = tx_event.send(Err(e)).await;
            return;
        }

        if state.completed {
            return;
        }
    }
}

#[derive(Default)]
struct StreamingState {
    message_id: String,
    content_blocks: Vec<ContentBlockState>,
    input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    completed: bool,
}

enum ContentBlockState {
    Text {
        index: usize,
        accumulated: String,
    },
    ToolUse {
        index: usize,
        id: String,
        name: String,
        args_json: String,
    },
}

impl ContentBlockState {
    fn index(&self) -> usize {
        match self {
            Self::Text { index, .. } => *index,
            Self::ToolUse { index, .. } => *index,
        }
    }
}

impl StreamingState {
    fn get_usage(&self) -> Option<TokenUsage> {
        match self.output_tokens {
            Some(output) => {
                let input = self.input_tokens.unwrap_or(0);
                let cache_creation = self.cache_creation_input_tokens.unwrap_or(0);
                let cache_read = self.cache_read_input_tokens.unwrap_or(0);

                // Total input includes new tokens and cache creation tokens
                let total_input = input + cache_creation;

                Some(TokenUsage {
                    input_tokens: total_input,
                    cached_input_tokens: cache_read,
                    output_tokens: output,
                    reasoning_output_tokens: 0, // Not separately tracked in basic API
                    total_tokens: total_input + cache_read + output,
                })
            }
            None => None,
        }
    }
}

async fn handle_event(
    state: &mut StreamingState,
    event: serde_json::Value,
    tx: &mpsc::Sender<Result<ResponseEvent>>,
) -> Result<()> {
    let event_type = event["type"].as_str().unwrap_or("");

    match event_type {
        "message_start" => {
            state.message_id = event["message"]["id"].as_str().unwrap_or("").to_string();
            if let Some(usage) = event["message"]["usage"].as_object() {
                state.input_tokens = usage
                    .get("input_tokens")
                    .and_then(serde_json::Value::as_u64);
                state.cache_creation_input_tokens = usage
                    .get("cache_creation_input_tokens")
                    .and_then(serde_json::Value::as_u64);
                state.cache_read_input_tokens = usage
                    .get("cache_read_input_tokens")
                    .and_then(serde_json::Value::as_u64);
                state.output_tokens = usage
                    .get("output_tokens")
                    .and_then(serde_json::Value::as_u64);
            }
            // Send Created event
            let _ = tx.send(Ok(ResponseEvent::Created)).await;
        }

        "content_block_start" => {
            let index = event["index"].as_u64().unwrap_or(0) as usize;
            let block = &event["content_block"];
            let block_type = block["type"].as_str().unwrap_or("");

            match block_type {
                "text" => {
                    state.content_blocks.push(ContentBlockState::Text {
                        index,
                        accumulated: String::new(),
                    });
                }
                "tool_use" => {
                    state.content_blocks.push(ContentBlockState::ToolUse {
                        index,
                        id: block["id"].as_str().unwrap_or("").to_string(),
                        name: block["name"].as_str().unwrap_or("").to_string(),
                        args_json: String::new(),
                    });
                }
                _ => {
                    debug!("Unknown content block type: {block_type}");
                }
            }
        }

        "content_block_delta" => {
            let index = event["index"].as_u64().unwrap_or(0) as usize;
            let delta = &event["delta"];
            let delta_type = delta["type"].as_str().unwrap_or("");

            if let Some(block) = state.content_blocks.iter_mut().find(|b| b.index() == index) {
                match (delta_type, block) {
                    ("text_delta", ContentBlockState::Text { accumulated, .. }) => {
                        if let Some(text) = delta["text"].as_str() {
                            accumulated.push_str(text);
                            let _ = tx
                                .send(Ok(ResponseEvent::OutputTextDelta(text.to_string())))
                                .await;
                        }
                    }
                    ("input_json_delta", ContentBlockState::ToolUse { args_json, .. }) => {
                        if let Some(partial) = delta["partial_json"].as_str() {
                            args_json.push_str(partial);
                        }
                    }
                    _ => {
                        debug!("Unexpected delta type/block combination: {delta_type}");
                    }
                }
            }
        }

        "content_block_stop" => {
            let index = event["index"].as_u64().unwrap_or(0) as usize;

            if let Some(pos) = state.content_blocks.iter().position(|b| b.index() == index) {
                let block = state.content_blocks.remove(pos);

                match block {
                    ContentBlockState::Text { accumulated, .. } => {
                        if !accumulated.is_empty() {
                            let _ = tx
                                .send(Ok(ResponseEvent::OutputItemDone(ResponseItem::Message {
                                    role: "assistant".to_string(),
                                    content: vec![ContentItem::OutputText { text: accumulated }],
                                    id: None,
                                })))
                                .await;
                        }
                    }
                    ContentBlockState::ToolUse {
                        id,
                        name,
                        args_json,
                        ..
                    } => {
                        let _ = tx
                            .send(Ok(ResponseEvent::OutputItemDone(
                                ResponseItem::FunctionCall {
                                    id: None,
                                    name,
                                    arguments: args_json,
                                    call_id: id,
                                },
                            )))
                            .await;
                    }
                }
            }
        }

        "message_delta" => {
            if let Some(usage) = event["usage"].as_object() {
                // Update all token counts from delta event (they're cumulative)
                if let Some(input) = usage
                    .get("input_tokens")
                    .and_then(serde_json::Value::as_u64)
                {
                    state.input_tokens = Some(input);
                }
                if let Some(cache_creation) = usage
                    .get("cache_creation_input_tokens")
                    .and_then(serde_json::Value::as_u64)
                {
                    state.cache_creation_input_tokens = Some(cache_creation);
                }
                if let Some(cache_read) = usage
                    .get("cache_read_input_tokens")
                    .and_then(serde_json::Value::as_u64)
                {
                    state.cache_read_input_tokens = Some(cache_read);
                }
                if let Some(output) = usage
                    .get("output_tokens")
                    .and_then(serde_json::Value::as_u64)
                {
                    state.output_tokens = Some(output);
                }
            }
        }

        "message_stop" => {
            let _ = tx
                .send(Ok(ResponseEvent::Completed {
                    response_id: state.message_id.clone(),
                    token_usage: state.get_usage(),
                }))
                .await;
            state.completed = true;
        }

        "ping" => {
            // Ignore keep-alive
        }

        "error" => {
            let error_msg = event["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            return Err(CodexErr::Stream(error_msg, None));
        }

        _ => {
            debug!("Unknown Anthropic event type: {event_type}");
        }
    }

    Ok(())
}

/// Represents Anthropic-style cache control settings.
#[derive(Debug, Clone)]
pub enum CacheType {
    /// Ephemeral cache block — reused within a short window.
    Ephemeral,
}

pub fn cache_control_value(cache_type: &CacheType) -> serde_json::Value {
    match cache_type {
        CacheType::Ephemeral => json!({ "type": "ephemeral" }),
    }
}

/// Attaches a cache_control block to an existing JSON object representing a content item.
pub fn attach_cache_control(content_block: &mut serde_json::Value, cache_type: &CacheType) {
    if let Some(obj) = content_block.as_object_mut() {
        obj.insert("cache_control".into(), cache_control_value(cache_type));
    }
}

/// Build a standardized system block with optional text and cache control.
fn build_system_block<S: AsRef<str>>(text: S, cache_type: CacheType) -> serde_json::Value {
    let mut block = json!({
        "type": "text",
        "text": text.as_ref()
    });
    attach_cache_control(&mut block, &cache_type);
    block
}

/// Marks the last `n_messages` with cache_control on their final content block.
/// Works regardless of role (assistant/user).
fn mark_tail_messages_for_cache(
    messages: &mut [serde_json::Value],
    n_messages: usize,
    cache_type: CacheType,
) {
    for msg in messages.iter_mut().rev().take(n_messages) {
        if let Some(content) = msg.get_mut("content") {
            match content {
                serde_json::Value::Array(blocks) => {
                    if let Some(last_block) = blocks.last_mut() {
                        attach_cache_control(last_block, &cache_type);
                    }
                }
                serde_json::Value::String(text) => {
                    // Convert shorthand message into array form with cache marker.
                    *content = json!([{
                        "type": "text",
                        "text": text,
                        "cache_control": cache_control_value(&cache_type)
                    }]);
                }
                _ => tracing::debug!("Unexpected message content type during caching"),
            }
        }
    }
}
