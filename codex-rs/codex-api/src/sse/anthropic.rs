//! Anthropic Messages API SSE stream parser.
//!
//! Handles Anthropic's streaming format which uses events like:
//! - `message_start` - Message ID, initial usage
//! - `content_block_start` - Start of text or tool_use block
//! - `content_block_delta` - text_delta or input_json_delta
//! - `content_block_stop` - End of content block
//! - `message_delta` - Final usage stats
//! - `message_stop` - Stream complete
//!
//! See: https://docs.anthropic.com/claude/reference/messages-streaming

use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::StreamResponse;
use codex_client::TransportError;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::Stream;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

/// Spawns a task to process Anthropic SSE stream and returns a ResponseStream.
pub fn spawn_anthropic_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        process_anthropic_sse(stream_response.bytes, tx_event, idle_timeout, telemetry).await;
    });
    ResponseStream { rx_event }
}

/// Anthropic message_start event data
#[derive(Debug, Deserialize)]
struct MessageStart {
    message: MessageInfo,
}

#[derive(Debug, Deserialize)]
struct MessageInfo {
    id: String,
    #[serde(default)]
    usage: Option<MessageUsage>,
}

#[derive(Debug, Deserialize)]
struct MessageUsage {
    input_tokens: i64,
    #[allow(dead_code)]
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<i64>,
}

/// Anthropic content_block_start event data
#[derive(Debug, Deserialize)]
struct ContentBlockStart {
    index: usize,
    content_block: ContentBlock,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

/// Anthropic content_block_delta event data
#[derive(Debug, Deserialize)]
struct ContentBlockDelta {
    index: usize,
    delta: Delta,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Delta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

/// Anthropic content_block_stop event data
#[derive(Debug, Deserialize)]
struct ContentBlockStop {
    index: usize,
}

/// Anthropic message_delta event data
#[derive(Debug, Deserialize)]
struct MessageDelta {
    delta: MessageDeltaInfo,
    #[serde(default)]
    usage: Option<MessageDeltaUsage>,
}

#[derive(Debug, Deserialize)]
struct MessageDeltaInfo {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageDeltaUsage {
    output_tokens: i64,
}

/// Anthropic error event data
#[derive(Debug, Deserialize)]
struct AnthropicError {
    r#type: String,
    message: String,
}

/// State for accumulating content blocks
#[derive(Debug, Default)]
struct ContentBlockState {
    /// Accumulated text for text blocks
    text: String,
    /// Tool call ID for tool_use blocks
    tool_id: Option<String>,
    /// Tool name for tool_use blocks
    tool_name: Option<String>,
    /// Accumulated JSON arguments for tool_use blocks
    tool_arguments: String,
    /// Whether this is a tool_use block
    is_tool_use: bool,
}

/// Process Anthropic SSE stream.
pub async fn process_anthropic_sse<S>(
    stream: S,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) where
    S: Stream<Item = Result<bytes::Bytes, TransportError>> + Unpin,
{
    let mut stream = stream.eventsource();
    let mut message_id: Option<String> = None;
    let mut input_tokens: i64 = 0;
    let mut output_tokens: i64 = 0;
    let mut cached_input_tokens: i64 = 0;
    let mut content_blocks: HashMap<usize, ContentBlockState> = HashMap::new();
    let mut message_sent = false;
    let mut response_error: Option<ApiError> = None;

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }

        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(e))) => {
                debug!("Anthropic SSE Error: {e:#}");
                let _ = tx_event.send(Err(ApiError::Stream(e.to_string()))).await;
                return;
            }
            Ok(None) => {
                // Stream ended
                if let Some(err) = response_error {
                    let _ = tx_event.send(Err(err)).await;
                } else if message_id.is_some() {
                    // Emit completion event
                    let usage = TokenUsage {
                        input_tokens,
                        cached_input_tokens,
                        output_tokens,
                        reasoning_output_tokens: 0, // Anthropic doesn't expose this separately
                        total_tokens: input_tokens + output_tokens,
                    };
                    let _ = tx_event
                        .send(Ok(ResponseEvent::Completed {
                            response_id: message_id.unwrap_or_default(),
                            token_usage: Some(usage),
                        }))
                        .await;
                } else {
                    let _ = tx_event
                        .send(Err(ApiError::Stream(
                            "stream closed before message_start".into(),
                        )))
                        .await;
                }
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };

        trace!(
            "Anthropic SSE event type: {}, data: {}",
            sse.event, sse.data
        );

        if sse.data.trim().is_empty() {
            continue;
        }

        match sse.event.as_str() {
            "message_start" => {
                let Ok(data) = serde_json::from_str::<MessageStart>(&sse.data) else {
                    debug!("Failed to parse message_start: {}", &sse.data);
                    continue;
                };
                message_id = Some(data.message.id);
                if let Some(usage) = data.message.usage {
                    input_tokens = usage.input_tokens;
                    cached_input_tokens = usage.cache_read_input_tokens.unwrap_or(0);
                    // Include cache_creation_input_tokens in input_tokens if present
                    if let Some(cache_creation) = usage.cache_creation_input_tokens {
                        input_tokens += cache_creation;
                    }
                }
                let _ = tx_event.send(Ok(ResponseEvent::Created)).await;
            }

            "content_block_start" => {
                let Ok(data) = serde_json::from_str::<ContentBlockStart>(&sse.data) else {
                    debug!("Failed to parse content_block_start: {}", &sse.data);
                    continue;
                };

                let mut state = ContentBlockState::default();
                match data.content_block {
                    ContentBlock::Text { text } => {
                        state.text = text;
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        state.is_tool_use = true;
                        state.tool_id = Some(id);
                        state.tool_name = Some(name);
                        // input may already be provided in full (non-null, non-empty object)
                        // Skip empty objects {} as they indicate streaming will follow
                        let is_empty_obj = input
                            .as_object()
                            .map(serde_json::Map::is_empty)
                            .unwrap_or(false);
                        if !input.is_null() && !is_empty_obj {
                            state.tool_arguments =
                                serde_json::to_string(&input).unwrap_or_default();
                        }
                    }
                }
                content_blocks.insert(data.index, state);

                // Send OutputItemAdded for the message when first content block starts
                if !message_sent {
                    let item = ResponseItem::Message {
                        id: None,
                        role: "assistant".to_string(),
                        content: vec![],
                    };
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemAdded(item)))
                        .await;
                    message_sent = true;
                }
            }

            "content_block_delta" => {
                let Ok(data) = serde_json::from_str::<ContentBlockDelta>(&sse.data) else {
                    debug!("Failed to parse content_block_delta: {}", &sse.data);
                    continue;
                };

                let Some(state) = content_blocks.get_mut(&data.index) else {
                    debug!("content_block_delta for unknown index: {}", data.index);
                    continue;
                };

                match data.delta {
                    Delta::TextDelta { text } => {
                        state.text.push_str(&text);
                        let _ = tx_event
                            .send(Ok(ResponseEvent::OutputTextDelta(text)))
                            .await;
                    }
                    Delta::InputJsonDelta { partial_json } => {
                        state.tool_arguments.push_str(&partial_json);
                    }
                }
            }

            "content_block_stop" => {
                let Ok(data) = serde_json::from_str::<ContentBlockStop>(&sse.data) else {
                    debug!("Failed to parse content_block_stop: {}", &sse.data);
                    continue;
                };

                let Some(state) = content_blocks.remove(&data.index) else {
                    continue;
                };

                if state.is_tool_use {
                    // Emit tool call as FunctionCall
                    if let (Some(call_id), Some(name)) = (state.tool_id, state.tool_name) {
                        let item = ResponseItem::FunctionCall {
                            id: None,
                            name,
                            arguments: state.tool_arguments,
                            call_id,
                        };
                        let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                    }
                } else if !state.text.is_empty() {
                    // Emit text message
                    let item = ResponseItem::Message {
                        id: None,
                        role: "assistant".to_string(),
                        content: vec![ContentItem::OutputText { text: state.text }],
                    };
                    let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                }
            }

            "message_delta" => {
                let Ok(data) = serde_json::from_str::<MessageDelta>(&sse.data) else {
                    debug!("Failed to parse message_delta: {}", &sse.data);
                    continue;
                };

                if let Some(usage) = data.usage {
                    output_tokens = usage.output_tokens;
                }

                // Check for stop reason indicating context window exceeded
                if data.delta.stop_reason.as_deref() == Some("max_tokens") {
                    response_error = Some(ApiError::ContextWindowExceeded);
                }
            }

            "message_stop" => {
                // Message complete - the final Completed event will be sent when stream ends
            }

            "error" => {
                let Ok(data) = serde_json::from_str::<AnthropicError>(&sse.data) else {
                    debug!("Failed to parse error: {}", &sse.data);
                    response_error = Some(ApiError::Stream(sse.data.clone()));
                    continue;
                };

                // Check for specific error types
                if data.r#type == "overloaded_error" {
                    response_error = Some(ApiError::Retryable {
                        message: data.message,
                        delay: Some(Duration::from_secs(1)),
                    });
                } else if data.r#type == "rate_limit_error" {
                    let delay = try_parse_anthropic_retry_after(&data.message);
                    response_error = Some(ApiError::Retryable {
                        message: data.message,
                        delay,
                    });
                } else if data.r#type == "invalid_request_error" && data.message.contains("context")
                {
                    response_error = Some(ApiError::ContextWindowExceeded);
                } else {
                    response_error = Some(ApiError::Stream(format!(
                        "{}: {}",
                        data.r#type, data.message
                    )));
                }
            }

            "ping" => {
                // Heartbeat event, ignore
            }

            _ => {
                // Unknown event type, log and continue
                debug!("Unknown Anthropic SSE event type: {}", sse.event);
            }
        }
    }
}

/// Try to parse retry-after duration from Anthropic error message.
fn try_parse_anthropic_retry_after(message: &str) -> Option<Duration> {
    // Anthropic rate limit messages sometimes contain "try again in X seconds"
    let re = anthropic_retry_regex();
    if let Some(captures) = re.captures(message)
        && let Some(seconds) = captures.get(1)
        && let Ok(secs) = seconds.as_str().parse::<f64>()
    {
        return Some(Duration::from_secs_f64(secs));
    }
    None
}

fn anthropic_retry_regex() -> &'static regex_lite::Regex {
    static RE: std::sync::OnceLock<regex_lite::Regex> = std::sync::OnceLock::new();
    #[expect(clippy::unwrap_used)]
    RE.get_or_init(|| {
        regex_lite::Regex::new(r"(?i)try again in\s*(\d+(?:\.\d+)?)\s*(?:s|seconds?)").unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use futures::TryStreamExt;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tokio_util::io::ReaderStream;

    fn build_anthropic_sse(events: &[(&str, serde_json::Value)]) -> String {
        let mut body = String::new();
        for (event_type, data) in events {
            body.push_str(&format!("event: {event_type}\ndata: {data}\n\n"));
        }
        body
    }

    async fn collect_events(body: &str) -> Vec<Result<ResponseEvent, ApiError>> {
        let reader = ReaderStream::new(std::io::Cursor::new(body.to_string()))
            .map_err(|err| TransportError::Network(err.to_string()));
        let (tx, mut rx) = mpsc::channel::<Result<ResponseEvent, ApiError>>(16);
        tokio::spawn(process_anthropic_sse(
            reader,
            tx,
            Duration::from_millis(1000),
            None,
        ));

        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            out.push(ev);
        }
        out
    }

    #[tokio::test]
    async fn test_text_streaming() {
        let events = vec![
            (
                "message_start",
                json!({
                    "type": "message_start",
                    "message": {
                        "id": "msg_123",
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "model": "claude-3-sonnet-20240229",
                        "usage": {"input_tokens": 10, "output_tokens": 0}
                    }
                }),
            ),
            (
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "text", "text": ""}
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": "Hello"}
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": " World"}
                }),
            ),
            (
                "content_block_stop",
                json!({"type": "content_block_stop", "index": 0}),
            ),
            (
                "message_delta",
                json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn"},
                    "usage": {"output_tokens": 5}
                }),
            ),
            ("message_stop", json!({"type": "message_stop"})),
        ];

        let body = build_anthropic_sse(&events);
        let results = collect_events(&body).await;

        // Verify Created event
        assert_matches!(&results[0], Ok(ResponseEvent::Created));

        // Verify OutputItemAdded for message
        assert_matches!(&results[1], Ok(ResponseEvent::OutputItemAdded(ResponseItem::Message { role, .. })) if role == "assistant");

        // Verify text deltas
        assert_matches!(&results[2], Ok(ResponseEvent::OutputTextDelta(t)) if t == "Hello");
        assert_matches!(&results[3], Ok(ResponseEvent::OutputTextDelta(t)) if t == " World");

        // Verify OutputItemDone for message
        assert_matches!(
            &results[4],
            Ok(ResponseEvent::OutputItemDone(ResponseItem::Message { content, .. }))
            if content.len() == 1
        );

        // Verify Completed event with usage
        assert_matches!(
            results.last(),
            Some(Ok(ResponseEvent::Completed { response_id, token_usage }))
            if response_id == "msg_123" && token_usage.as_ref().map(|u| u.output_tokens) == Some(5)
        );
    }

    #[tokio::test]
    async fn test_tool_use_streaming() {
        let events = vec![
            (
                "message_start",
                json!({
                    "type": "message_start",
                    "message": {
                        "id": "msg_456",
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "usage": {"input_tokens": 20, "output_tokens": 0}
                    }
                }),
            ),
            (
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "tool_use",
                        "id": "toolu_01",
                        "name": "shell",
                        "input": {}
                    }
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "input_json_delta", "partial_json": "{\"command\":"}
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "input_json_delta", "partial_json": "\"ls -la\"}"}
                }),
            ),
            (
                "content_block_stop",
                json!({"type": "content_block_stop", "index": 0}),
            ),
            (
                "message_delta",
                json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "tool_use"},
                    "usage": {"output_tokens": 15}
                }),
            ),
            ("message_stop", json!({"type": "message_stop"})),
        ];

        let body = build_anthropic_sse(&events);
        let results = collect_events(&body).await;

        // Find the FunctionCall event
        let tool_call = results.iter().find(|r| {
            matches!(
                r,
                Ok(ResponseEvent::OutputItemDone(
                    ResponseItem::FunctionCall { .. }
                ))
            )
        });

        assert_matches!(
            tool_call,
            Some(Ok(ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            }))) if call_id == "toolu_01" && name == "shell" && arguments == "{\"command\":\"ls -la\"}"
        );
    }

    #[tokio::test]
    async fn test_cached_tokens() {
        let events = vec![
            (
                "message_start",
                json!({
                    "type": "message_start",
                    "message": {
                        "id": "msg_789",
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "usage": {
                            "input_tokens": 100,
                            "output_tokens": 0,
                            "cache_read_input_tokens": 50,
                            "cache_creation_input_tokens": 10
                        }
                    }
                }),
            ),
            (
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "text", "text": ""}
                }),
            ),
            (
                "content_block_delta",
                json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": "Hi"}
                }),
            ),
            (
                "content_block_stop",
                json!({"type": "content_block_stop", "index": 0}),
            ),
            (
                "message_delta",
                json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn"},
                    "usage": {"output_tokens": 10}
                }),
            ),
            ("message_stop", json!({"type": "message_stop"})),
        ];

        let body = build_anthropic_sse(&events);
        let results = collect_events(&body).await;

        // Verify Completed event has correct cached token count
        assert_matches!(
            results.last(),
            Some(Ok(ResponseEvent::Completed { token_usage: Some(usage), .. }))
            if usage.cached_input_tokens == 50 && usage.input_tokens == 110 // 100 + 10 cache_creation
        );
    }

    #[tokio::test]
    async fn test_error_handling() {
        let events = vec![(
            "error",
            json!({
                "type": "overloaded_error",
                "message": "Overloaded"
            }),
        )];

        let body = build_anthropic_sse(&events);
        let results = collect_events(&body).await;

        assert_matches!(
            results.last(),
            Some(Err(ApiError::Retryable { message, .. })) if message == "Overloaded"
        );
    }

    #[tokio::test]
    async fn test_rate_limit_error() {
        let events = vec![(
            "error",
            json!({
                "type": "rate_limit_error",
                "message": "Rate limited. Please try again in 30 seconds."
            }),
        )];

        let body = build_anthropic_sse(&events);
        let results = collect_events(&body).await;

        assert_matches!(
            results.last(),
            Some(Err(ApiError::Retryable { delay: Some(d), .. })) if *d == Duration::from_secs(30)
        );
    }

    #[test]
    fn test_retry_after_parsing() {
        assert_eq!(
            try_parse_anthropic_retry_after("Please try again in 30 seconds."),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            try_parse_anthropic_retry_after("try again in 1.5s"),
            Some(Duration::from_secs_f64(1.5))
        );
        assert_eq!(try_parse_anthropic_retry_after("no retry info here"), None);
    }
}
