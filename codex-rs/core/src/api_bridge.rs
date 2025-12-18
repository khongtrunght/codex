use chrono::DateTime;
use chrono::Utc;
use codex_api::AuthProvider as ApiAuthProvider;
use codex_api::AuthScheme;
use codex_api::TransportError;
use codex_api::error::ApiError;
use codex_api::rate_limits::parse_rate_limit;
use http::HeaderMap;
use serde::Deserialize;

use crate::auth::CodexAuth;
use crate::auth::ProviderAuth;
use crate::error::CodexErr;
use crate::error::RetryLimitReachedError;
use crate::error::UnexpectedResponseError;
use crate::error::UsageLimitReachedError;
use crate::model_provider_info::ModelProviderInfo;
use crate::token_data::PlanType;

pub(crate) fn map_api_error(err: ApiError) -> CodexErr {
    match err {
        ApiError::ContextWindowExceeded => CodexErr::ContextWindowExceeded,
        ApiError::QuotaExceeded => CodexErr::QuotaExceeded,
        ApiError::UsageNotIncluded => CodexErr::UsageNotIncluded,
        ApiError::Retryable { message, delay } => CodexErr::Stream(message, delay),
        ApiError::Stream(msg) => CodexErr::Stream(msg, None),
        ApiError::Api { status, message } => CodexErr::UnexpectedStatus(UnexpectedResponseError {
            status,
            body: message,
            request_id: None,
        }),
        ApiError::Transport(transport) => match transport {
            TransportError::Http {
                status,
                headers,
                body,
            } => {
                let body_text = body.unwrap_or_default();

                if status == http::StatusCode::BAD_REQUEST {
                    if body_text
                        .contains("The image data you provided does not represent a valid image")
                    {
                        CodexErr::InvalidImageRequest()
                    } else {
                        CodexErr::InvalidRequest(body_text)
                    }
                } else if status == http::StatusCode::INTERNAL_SERVER_ERROR {
                    CodexErr::InternalServerError
                } else if status == http::StatusCode::TOO_MANY_REQUESTS {
                    if let Ok(err) = serde_json::from_str::<UsageErrorResponse>(&body_text) {
                        if err.error.error_type.as_deref() == Some("usage_limit_reached") {
                            let rate_limits = headers.as_ref().and_then(parse_rate_limit);
                            let resets_at = err
                                .error
                                .resets_at
                                .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0));
                            return CodexErr::UsageLimitReached(UsageLimitReachedError {
                                plan_type: err.error.plan_type,
                                resets_at,
                                rate_limits,
                            });
                        } else if err.error.error_type.as_deref() == Some("usage_not_included") {
                            return CodexErr::UsageNotIncluded;
                        }
                    }

                    CodexErr::RetryLimit(RetryLimitReachedError {
                        status,
                        request_id: extract_request_id(headers.as_ref()),
                    })
                } else {
                    CodexErr::UnexpectedStatus(UnexpectedResponseError {
                        status,
                        body: body_text,
                        request_id: extract_request_id(headers.as_ref()),
                    })
                }
            }
            TransportError::RetryLimit => CodexErr::RetryLimit(RetryLimitReachedError {
                status: http::StatusCode::INTERNAL_SERVER_ERROR,
                request_id: None,
            }),
            TransportError::Timeout => CodexErr::Timeout,
            TransportError::Network(msg) | TransportError::Build(msg) => {
                CodexErr::Stream(msg, None)
            }
        },
        ApiError::RateLimit(msg) => CodexErr::Stream(msg, None),
    }
}

fn extract_request_id(headers: Option<&HeaderMap>) -> Option<String> {
    headers.and_then(|map| {
        ["cf-ray", "x-request-id", "x-oai-request-id"]
            .iter()
            .find_map(|name| {
                map.get(*name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            })
    })
}

pub(crate) async fn auth_provider_from_auth(
    auth: Option<CodexAuth>,
    provider: &ModelProviderInfo,
) -> crate::error::Result<CoreAuthProvider> {
    if let Some(api_key) = provider.api_key()? {
        return Ok(CoreAuthProvider {
            token: Some(api_key),
            account_id: None,
            auth_scheme: AuthScheme::Bearer, // Legacy always uses Bearer
            extra_headers: HeaderMap::new(),
        });
    }

    if let Some(token) = provider.experimental_bearer_token.clone() {
        return Ok(CoreAuthProvider {
            token: Some(token),
            account_id: None,
            auth_scheme: AuthScheme::Bearer,
            extra_headers: HeaderMap::new(),
        });
    }

    if let Some(auth) = auth {
        let token = auth.get_token().await?;
        Ok(CoreAuthProvider {
            token: Some(token),
            account_id: auth.get_account_id(),
            auth_scheme: AuthScheme::Bearer, // Legacy CodexAuth is always OpenAI
            extra_headers: HeaderMap::new(),
        })
    } else {
        Ok(CoreAuthProvider::default())
    }
}

/// Create auth provider from provider-specific auth (new path for multi-provider support)
#[allow(dead_code)] // Will be used when stream_anthropic_api is implemented in Phase 6
pub(crate) fn auth_provider_from_provider_auth(
    provider_auth: Option<&dyn ProviderAuth>,
    provider: &ModelProviderInfo,
) -> crate::error::Result<CoreAuthProvider> {
    // Priority 1: Provider-specific env key
    if let Some(api_key) = provider.api_key()? {
        return Ok(CoreAuthProvider {
            token: Some(api_key),
            account_id: None,
            auth_scheme: provider_auth
                .map(ProviderAuth::auth_scheme)
                .unwrap_or(AuthScheme::Bearer),
            extra_headers: provider_auth
                .map(ProviderAuth::extra_headers)
                .unwrap_or_default(),
        });
    }

    // Priority 2: Config override
    if let Some(token) = provider.experimental_bearer_token.clone() {
        return Ok(CoreAuthProvider {
            token: Some(token),
            account_id: None,
            auth_scheme: AuthScheme::Bearer,
            extra_headers: HeaderMap::new(), // Config override doesn't use extra headers
        });
    }

    // Priority 3: Provider-specific auth
    if let Some(auth) = provider_auth {
        return Ok(CoreAuthProvider {
            token: auth.get_token(),
            account_id: auth.account_id(),
            auth_scheme: auth.auth_scheme(),
            extra_headers: auth.extra_headers(),
        });
    }

    // No auth
    Ok(CoreAuthProvider::default())
}

#[derive(Debug, Deserialize)]
struct UsageErrorResponse {
    error: UsageErrorBody,
}

#[derive(Debug, Deserialize)]
struct UsageErrorBody {
    #[serde(rename = "type")]
    error_type: Option<String>,
    plan_type: Option<PlanType>,
    resets_at: Option<i64>,
}

#[derive(Clone, Default)]
pub(crate) struct CoreAuthProvider {
    token: Option<String>,
    account_id: Option<String>,
    auth_scheme: AuthScheme,
    /// Additional headers required for this auth type (e.g., anthropic-beta for OAuth)
    extra_headers: HeaderMap,
}

impl ApiAuthProvider for CoreAuthProvider {
    fn bearer_token(&self) -> Option<String> {
        self.token.clone()
    }

    fn account_id(&self) -> Option<String> {
        self.account_id.clone()
    }

    fn auth_scheme(&self) -> AuthScheme {
        self.auth_scheme
    }

    fn extra_headers(&self) -> HeaderMap {
        self.extra_headers.clone()
    }
}
