//! Anthropic Messages API client.
//!
//! Provides streaming support for Anthropic's Messages API.

use crate::auth::AuthProvider;
use crate::common::Prompt as ApiPrompt;
use crate::common::ResponseStream;
use crate::endpoint::streaming::StreamingClient;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::AnthropicRequest;
use crate::requests::AnthropicRequestBuilder;
use crate::sse::anthropic::spawn_anthropic_stream;
use crate::telemetry::SseTelemetry;
use codex_client::HttpTransport;
use codex_client::RequestTelemetry;
use http::HeaderMap;
use serde_json::Value;
use std::sync::Arc;

/// Client for Anthropic Messages API.
pub struct AnthropicClient<T: HttpTransport, A: AuthProvider> {
    streaming: StreamingClient<T, A>,
}

impl<T: HttpTransport, A: AuthProvider> AnthropicClient<T, A> {
    /// Create a new Anthropic client.
    pub fn new(transport: T, provider: Provider, auth: A) -> Self {
        Self {
            streaming: StreamingClient::new(transport, provider, auth),
        }
    }

    /// Configure telemetry for request and SSE events.
    pub fn with_telemetry(
        self,
        request: Option<Arc<dyn RequestTelemetry>>,
        sse: Option<Arc<dyn SseTelemetry>>,
    ) -> Self {
        Self {
            streaming: self.streaming.with_telemetry(request, sse),
        }
    }

    /// Stream a pre-built request.
    pub async fn stream_request(
        &self,
        request: AnthropicRequest,
    ) -> Result<ResponseStream, ApiError> {
        self.stream(request.body, request.headers).await
    }

    /// Stream a prompt using the Anthropic Messages API.
    pub async fn stream_prompt(
        &self,
        model: &str,
        prompt: &ApiPrompt,
    ) -> Result<ResponseStream, ApiError> {
        let request =
            AnthropicRequestBuilder::new(model, &prompt.instructions, &prompt.input, &prompt.tools)
                .build()?;

        self.stream_request(request).await
    }

    /// The API path for messages endpoint.
    fn path(&self) -> &'static str {
        "messages"
    }

    /// Stream with raw body and headers.
    pub async fn stream(
        &self,
        body: Value,
        extra_headers: HeaderMap,
    ) -> Result<ResponseStream, ApiError> {
        self.streaming
            .stream(self.path(), body, extra_headers, spawn_anthropic_stream)
            .await
    }
}
