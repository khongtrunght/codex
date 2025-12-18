use async_trait::async_trait;
use chrono::{Duration, Utc};

use crate::auth::provider_auth::ProviderAuth;
use crate::auth::storage::ProviderCredential;
use crate::auth::RefreshTokenError;
use codex_api::AuthScheme;
use codex_client::CodexHttpClient;

const TOKEN_REFRESH_INTERVAL_DAYS: i64 = 8;

/// OpenAI-specific authentication provider.
/// Handles both API key and OAuth (ChatGPT) authentication modes.
#[derive(Debug, Clone)]
pub struct OpenAIAuth {
    config_key: String, // e.g., "openai" or "openai-work"
    credential: ProviderCredential,
}

impl OpenAIAuth {
    pub fn new(config_key: String, credential: ProviderCredential) -> Self {
        Self {
            config_key,
            credential,
        }
    }

    pub fn from_api_key(config_key: String, key: String) -> Self {
        Self {
            config_key,
            credential: ProviderCredential::Api { key },
        }
    }

    /// Create from a ProviderCredential (convenience alias for new with default config_key)
    pub fn from_credential(credential: ProviderCredential) -> Self {
        Self {
            config_key: "openai".to_string(),
            credential,
        }
    }
}

#[async_trait]
impl ProviderAuth for OpenAIAuth {
    fn provider_id(&self) -> &str {
        &self.config_key // Returns config key, not hardcoded "openai"
    }

    fn get_token(&self) -> Option<String> {
        match &self.credential {
            ProviderCredential::Api { key } => Some(key.clone()),
            // Prefer exchanged_api_key if available (OpenAI's token exchange pattern)
            ProviderCredential::OAuth {
                exchanged_api_key: Some(key),
                ..
            } => Some(key.clone()),
            ProviderCredential::OAuth { access_token, .. } => Some(access_token.clone()),
        }
    }

    fn auth_scheme(&self) -> AuthScheme {
        AuthScheme::Bearer
    }

    fn account_id(&self) -> Option<String> {
        match &self.credential {
            ProviderCredential::OAuth { account_id, .. } => account_id.clone(),
            _ => None,
        }
    }

    fn has_exchanged_api_key(&self) -> bool {
        matches!(
            &self.credential,
            ProviderCredential::OAuth {
                exchanged_api_key: Some(_),
                ..
            }
        )
    }

    fn needs_refresh(&self) -> bool {
        match &self.credential {
            ProviderCredential::Api { .. } => false,
            ProviderCredential::OAuth { last_refresh, .. } => match last_refresh {
                Some(lr) => *lr < Utc::now() - Duration::days(TOKEN_REFRESH_INTERVAL_DAYS),
                None => true, // Never refreshed
            },
        }
    }

    async fn refresh(
        &self,
        client: &CodexHttpClient,
    ) -> Result<Option<ProviderCredential>, RefreshTokenError> {
        match &self.credential {
            ProviderCredential::Api { .. } => Ok(None),
            ProviderCredential::OAuth {
                refresh_token,
                exchanged_api_key,
                ..
            } => {
                let response =
                    crate::auth::try_refresh_token_internal(refresh_token.clone(), client).await?;
                Ok(Some(ProviderCredential::OAuth {
                    access_token: response.access_token.unwrap_or_default(),
                    refresh_token: response
                        .refresh_token
                        .unwrap_or_else(|| refresh_token.clone()),
                    expires_at: None,
                    last_refresh: Some(Utc::now()), // Record refresh time in credential
                    account_id: None,               // Will be updated from id_token parsing
                    // Preserve the exchanged API key - it remains valid after OAuth refresh
                    exchanged_api_key: exchanged_api_key.clone(),
                    extra: response.id_token.map(|t| serde_json::json!({ "id_token": t })),
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_auth_returns_key() {
        let auth = OpenAIAuth::from_api_key("openai".to_string(), "sk-test".to_string());
        assert_eq!(auth.get_token(), Some("sk-test".to_string()));
        assert_eq!(auth.auth_scheme(), AuthScheme::Bearer);
        assert!(!auth.needs_refresh());
    }

    #[test]
    fn oauth_with_exchanged_key_prefers_api_key() {
        let auth = OpenAIAuth::from_credential(ProviderCredential::OAuth {
            access_token: "access-token".to_string(),
            refresh_token: "refresh-token".to_string(),
            expires_at: None,
            last_refresh: Some(Utc::now()),
            account_id: Some("acct-123".to_string()),
            exchanged_api_key: Some("sk-exchanged".to_string()),
            extra: None,
        });
        assert_eq!(auth.get_token(), Some("sk-exchanged".to_string()));
        assert!(auth.has_exchanged_api_key());
        assert_eq!(auth.account_id(), Some("acct-123".to_string()));
    }

    #[test]
    fn oauth_without_exchanged_key_uses_access_token() {
        let auth = OpenAIAuth::from_credential(ProviderCredential::OAuth {
            access_token: "access-token".to_string(),
            refresh_token: "refresh-token".to_string(),
            expires_at: None,
            last_refresh: Some(Utc::now()),
            account_id: None,
            exchanged_api_key: None,
            extra: None,
        });
        assert_eq!(auth.get_token(), Some("access-token".to_string()));
        assert!(!auth.has_exchanged_api_key());
    }

    #[test]
    fn oauth_needs_refresh_when_stale() {
        let stale_auth = OpenAIAuth::from_credential(ProviderCredential::OAuth {
            access_token: "token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: None,
            last_refresh: Some(Utc::now() - Duration::days(10)), // 10 days ago
            account_id: None,
            exchanged_api_key: None,
            extra: None,
        });
        assert!(stale_auth.needs_refresh());
    }

    #[test]
    fn oauth_does_not_need_refresh_when_recent() {
        let recent_auth = OpenAIAuth::from_credential(ProviderCredential::OAuth {
            access_token: "token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: None,
            last_refresh: Some(Utc::now()), // Just now
            account_id: None,
            exchanged_api_key: None,
            extra: None,
        });
        assert!(!recent_auth.needs_refresh());
    }
}
