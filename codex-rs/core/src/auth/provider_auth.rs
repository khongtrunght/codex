use async_trait::async_trait;
use http::HeaderMap;

use crate::auth::RefreshTokenError;
use crate::auth::storage::ProviderCredential;
use codex_api::AuthScheme;
use codex_client::CodexHttpClient;

/// Authentication provider for a specific LLM provider.
///
/// This is a higher-level abstraction than `codex_api::AuthProvider`:
/// - `ProviderAuth` (this trait) handles credential storage, refresh logic, and provider identification
/// - `AuthProvider` (codex-api) is the transport-level auth for HTTP requests
///
/// The `CoreAuthProvider` bridge (in api_bridge.rs) adapts ProviderAuth to AuthProvider.
#[async_trait]
pub trait ProviderAuth: Send + Sync + std::fmt::Debug {
    /// Provider ID (e.g., "openai", "anthropic", "anthropic-work")
    /// This is the config key used for credential storage.
    fn provider_id(&self) -> &str;

    /// Get current bearer token/API key for requests
    fn get_token(&self) -> Option<String>;

    /// Auth header format for this provider
    fn auth_scheme(&self) -> AuthScheme {
        AuthScheme::Bearer
    }

    /// Get account ID if available (for ChatGPT)
    fn account_id(&self) -> Option<String> {
        None
    }

    /// Additional headers required for this auth type.
    /// For example, Anthropic OAuth requires "anthropic-beta: oauth-2025-04-20".
    /// Returns empty HeaderMap if no extra headers needed.
    fn extra_headers(&self) -> HeaderMap {
        HeaderMap::new()
    }

    /// Check if this credential has an exchanged API key (OpenAI OAuth pattern).
    /// OpenAI OAuth uses RFC 8693 token exchange to get a permanent API key.
    /// When true, get_token() returns the exchanged API key instead of access_token.
    fn has_exchanged_api_key(&self) -> bool {
        false
    }

    /// Does this credential need refresh?
    /// Uses the credential's own last_refresh/expires_at fields.
    fn needs_refresh(&self) -> bool;

    /// Refresh OAuth tokens (returns None if not OAuth or refresh not needed).
    /// The returned credential includes updated last_refresh timestamp.
    async fn refresh(
        &self,
        client: &CodexHttpClient,
    ) -> Result<Option<ProviderCredential>, RefreshTokenError>;
}
