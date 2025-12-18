use async_trait::async_trait;
use chrono::{Duration, Utc};
use http::HeaderMap;

use crate::auth::provider_auth::ProviderAuth;
use crate::auth::storage::ProviderCredential;
use crate::auth::RefreshTokenError;
use codex_api::AuthScheme;
use codex_client::CodexHttpClient;

/// Anthropic token endpoint for OAuth refresh
const ANTHROPIC_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
/// Anthropic OAuth client ID (public, used by CLI tools)
const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Refresh tokens before expiration (buffer time)
const TOKEN_REFRESH_BUFFER_HOURS: i64 = 1;
/// Fallback refresh interval if no expires_at is set
const TOKEN_REFRESH_INTERVAL_DAYS: i64 = 8;

/// Anthropic-specific authentication provider.
/// Handles both API key and OAuth authentication modes.
///
/// For API keys: Uses `x-api-key` header format
/// For OAuth: Uses `Authorization: Bearer` with `anthropic-beta: oauth-2025-04-20` header
#[derive(Debug, Clone)]
pub struct AnthropicAuth {
    config_key: String, // e.g., "anthropic" or "anthropic-work"
    credential: ProviderCredential,
}

impl AnthropicAuth {
    pub fn new(config_key: String, credential: ProviderCredential) -> Self {
        Self {
            config_key,
            credential,
        }
    }

    pub fn from_api_key(config_key: String, api_key: String) -> Self {
        Self {
            config_key,
            credential: ProviderCredential::Api { key: api_key },
        }
    }

    /// Create from a ProviderCredential (convenience alias for new with default config_key)
    pub fn from_credential(credential: ProviderCredential) -> Self {
        Self {
            config_key: "anthropic".to_string(),
            credential,
        }
    }

    /// Check if this is an OAuth credential
    #[allow(dead_code)]
    fn is_oauth(&self) -> bool {
        matches!(self.credential, ProviderCredential::OAuth { .. })
    }
}

#[async_trait]
impl ProviderAuth for AnthropicAuth {
    fn provider_id(&self) -> &str {
        &self.config_key // Returns config key, not hardcoded "anthropic"
    }

    fn get_token(&self) -> Option<String> {
        match &self.credential {
            ProviderCredential::Api { key } => Some(key.clone()),
            ProviderCredential::OAuth { access_token, .. } => Some(access_token.clone()),
        }
    }

    fn auth_scheme(&self) -> AuthScheme {
        // API keys use x-api-key header, OAuth tokens use Bearer
        // Reference: index.mjs lines 148-153 show OAuth uses Authorization: Bearer
        match &self.credential {
            ProviderCredential::Api { .. } => AuthScheme::ApiKey,
            ProviderCredential::OAuth { .. } => AuthScheme::Bearer,
        }
    }

    /// For OAuth requests, Anthropic requires the anthropic-beta header
    /// Reference: index.mjs line 146 - "oauth-2025-04-20" beta is required
    fn extra_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let ProviderCredential::OAuth { .. } = &self.credential {
            headers.insert(
                http::HeaderName::from_static("anthropic-beta"),
                http::HeaderValue::from_static("oauth-2025-04-20"),
            );
        }
        headers
    }

    fn needs_refresh(&self) -> bool {
        match &self.credential {
            ProviderCredential::Api { .. } => false, // API keys don't expire
            ProviderCredential::OAuth {
                expires_at,
                last_refresh,
                ..
            } => {
                // Check explicit expiration time first
                if let Some(exp) = expires_at {
                    return *exp < Utc::now() + Duration::hours(TOKEN_REFRESH_BUFFER_HOURS);
                }
                // Fall back to last_refresh check (refresh every 8 days like OpenAI)
                match last_refresh {
                    Some(lr) => *lr < Utc::now() - Duration::days(TOKEN_REFRESH_INTERVAL_DAYS),
                    None => true, // Never refreshed, should refresh
                }
            }
        }
    }

    async fn refresh(
        &self,
        client: &CodexHttpClient,
    ) -> Result<Option<ProviderCredential>, RefreshTokenError> {
        match &self.credential {
            ProviderCredential::Api { .. } => Ok(None), // API keys don't refresh
            ProviderCredential::OAuth { refresh_token, .. } => {
                let response = refresh_anthropic_token(refresh_token, client).await?;
                Ok(Some(ProviderCredential::OAuth {
                    access_token: response.access_token,
                    refresh_token: response
                        .refresh_token
                        .unwrap_or_else(|| refresh_token.clone()),
                    expires_at: response
                        .expires_in
                        .map(|secs| Utc::now() + Duration::seconds(secs as i64)),
                    last_refresh: Some(Utc::now()), // Record refresh time in credential
                    account_id: None,
                    // Anthropic OAuth doesn't use exchanged_api_key (uses access_token directly)
                    exchanged_api_key: None,
                    extra: None,
                }))
            }
        }
    }
}

/// Response from Anthropic token endpoint
#[derive(Debug, serde::Deserialize)]
struct AnthropicTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

/// Request body for Anthropic token refresh
#[derive(Debug, serde::Serialize)]
struct AnthropicTokenRequest {
    grant_type: &'static str,
    refresh_token: String,
    client_id: &'static str,
}

/// Refresh Anthropic OAuth token
/// Reference: index.mjs lines 99-116 - uses JSON body, not form encoding
async fn refresh_anthropic_token(
    refresh_token: &str,
    client: &CodexHttpClient,
) -> Result<AnthropicTokenResponse, RefreshTokenError> {
    let body = AnthropicTokenRequest {
        grant_type: "refresh_token",
        refresh_token: refresh_token.to_string(),
        client_id: ANTHROPIC_CLIENT_ID,
    };

    let response = client
        .post(ANTHROPIC_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| RefreshTokenError::Transient(std::io::Error::other(e.to_string())))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        // Use Transient for potentially recoverable HTTP errors
        return Err(RefreshTokenError::Transient(std::io::Error::other(format!(
            "Anthropic token refresh failed: {status} - {body}"
        ))));
    }

    response
        .json::<AnthropicTokenResponse>()
        .await
        .map_err(|e| RefreshTokenError::Transient(std::io::Error::other(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_uses_api_key_scheme() {
        let auth = AnthropicAuth::from_api_key("anthropic".to_string(), "sk-ant-xxx".to_string());
        assert_eq!(auth.auth_scheme(), AuthScheme::ApiKey);
        assert_eq!(auth.get_token(), Some("sk-ant-xxx".to_string()));
        assert!(!auth.needs_refresh());
        assert!(auth.extra_headers().is_empty()); // No extra headers for API key
    }

    #[test]
    fn oauth_uses_bearer_scheme_with_beta_header() {
        let auth = AnthropicAuth::from_credential(ProviderCredential::OAuth {
            access_token: "ant-access-token".to_string(),
            refresh_token: "ant-refresh-token".to_string(),
            expires_at: None,
            last_refresh: Some(Utc::now()),
            account_id: None,
            exchanged_api_key: None,
            extra: None,
        });
        assert_eq!(auth.auth_scheme(), AuthScheme::Bearer);
        assert_eq!(auth.get_token(), Some("ant-access-token".to_string()));

        // OAuth requires anthropic-beta header
        let extra = auth.extra_headers();
        assert!(!extra.is_empty());
        assert_eq!(
            extra.get("anthropic-beta").map(|v| v.to_str().unwrap()),
            Some("oauth-2025-04-20")
        );
    }

    #[test]
    fn oauth_needs_refresh_when_expiring_soon() {
        let expiring_auth = AnthropicAuth::from_credential(ProviderCredential::OAuth {
            access_token: "token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: Some(Utc::now() + Duration::minutes(30)), // Expires in 30 min
            last_refresh: Some(Utc::now()),
            account_id: None,
            exchanged_api_key: None,
            extra: None,
        });
        assert!(expiring_auth.needs_refresh()); // Within 1 hour buffer
    }

    #[test]
    fn oauth_does_not_need_refresh_when_valid() {
        let valid_auth = AnthropicAuth::from_credential(ProviderCredential::OAuth {
            access_token: "token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: Some(Utc::now() + Duration::hours(24)), // Expires in 24 hours
            last_refresh: Some(Utc::now()),
            account_id: None,
            exchanged_api_key: None,
            extra: None,
        });
        assert!(!valid_auth.needs_refresh());
    }

    #[test]
    fn oauth_needs_refresh_when_never_refreshed() {
        let never_refreshed = AnthropicAuth::from_credential(ProviderCredential::OAuth {
            access_token: "token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: None,
            last_refresh: None, // Never refreshed
            account_id: None,
            exchanged_api_key: None,
            extra: None,
        });
        assert!(never_refreshed.needs_refresh());
    }

    #[test]
    fn oauth_does_not_need_refresh_when_recently_refreshed() {
        let recent = AnthropicAuth::from_credential(ProviderCredential::OAuth {
            access_token: "token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: None,           // No explicit expiration
            last_refresh: Some(Utc::now()), // Just refreshed
            account_id: None,
            exchanged_api_key: None,
            extra: None,
        });
        assert!(!recent.needs_refresh());
    }

    #[test]
    fn config_key_is_provider_id() {
        let auth = AnthropicAuth::new(
            "anthropic-work".to_string(),
            ProviderCredential::Api {
                key: "sk-ant".to_string(),
            },
        );
        assert_eq!(auth.provider_id(), "anthropic-work");
    }
}
