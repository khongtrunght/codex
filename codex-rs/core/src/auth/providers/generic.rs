use async_trait::async_trait;
use http::HeaderMap;

use crate::auth::provider_auth::ProviderAuth;
use crate::auth::storage::ProviderCredential;
use crate::auth::RefreshTokenError;
use codex_api::AuthScheme;
use codex_client::CodexHttpClient;

/// Generic auth for unknown providers - uses Bearer scheme by default.
/// This is a fallback for providers that don't have specific authentication logic.
#[derive(Debug, Clone)]
pub struct GenericAuth {
    config_key: String,
    credential: ProviderCredential,
}

impl GenericAuth {
    pub fn new(config_key: String, credential: ProviderCredential) -> Self {
        Self {
            config_key,
            credential,
        }
    }
}

#[async_trait]
impl ProviderAuth for GenericAuth {
    fn provider_id(&self) -> &str {
        &self.config_key
    }

    fn get_token(&self) -> Option<String> {
        match &self.credential {
            ProviderCredential::Api { key } => Some(key.clone()),
            // Prefer exchanged_api_key if available
            ProviderCredential::OAuth {
                exchanged_api_key: Some(key),
                ..
            } => Some(key.clone()),
            ProviderCredential::OAuth { access_token, .. } => Some(access_token.clone()),
        }
    }

    fn auth_scheme(&self) -> AuthScheme {
        AuthScheme::Bearer // Default to Bearer for unknown providers
    }

    fn extra_headers(&self) -> HeaderMap {
        HeaderMap::new() // No extra headers for generic providers
    }

    fn needs_refresh(&self) -> bool {
        false // Generic providers don't auto-refresh
    }

    async fn refresh(
        &self,
        _client: &CodexHttpClient,
    ) -> Result<Option<ProviderCredential>, RefreshTokenError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_uses_bearer_scheme() {
        let auth = GenericAuth::new(
            "custom-provider".to_string(),
            ProviderCredential::Api {
                key: "custom-key".to_string(),
            },
        );
        assert_eq!(auth.auth_scheme(), AuthScheme::Bearer);
        assert_eq!(auth.get_token(), Some("custom-key".to_string()));
        assert_eq!(auth.provider_id(), "custom-provider");
    }

    #[test]
    fn generic_never_needs_refresh() {
        let auth = GenericAuth::new(
            "custom".to_string(),
            ProviderCredential::OAuth {
                access_token: "token".to_string(),
                refresh_token: "refresh".to_string(),
                expires_at: None,
                last_refresh: None,
                account_id: None,
                exchanged_api_key: None,
                extra: None,
            },
        );
        assert!(!auth.needs_refresh());
    }
}
