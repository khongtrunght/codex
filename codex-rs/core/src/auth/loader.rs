use std::sync::Arc;
use std::sync::RwLock;

use crate::auth::provider_auth::ProviderAuth;
use crate::auth::providers::AnthropicAuth;
use crate::auth::providers::OpenAIAuth;
use crate::auth::storage::AuthDotJson;
use crate::auth::storage::AuthStorageBackend;
use crate::auth::storage::ProviderCredential;
use crate::model_provider_info::ModelProviderInfo;
use crate::model_provider_info::WireApi;

/// Load credentials for a provider from multiple sources.
///
/// Implements the credential loading priority:
/// 1. Environment variable (`provider_info.env_key`)
/// 2. Stored credentials (`auth.json`, keyed by config_key)
/// 3. Config file override (`provider_info.experimental_bearer_token`)
pub struct CredentialLoader {
    auth_storage: Arc<dyn AuthStorageBackend>,
    cached_auth: RwLock<Option<AuthDotJson>>,
}

impl CredentialLoader {
    pub(super) fn new(auth_storage: Arc<dyn AuthStorageBackend>) -> Self {
        let cached_auth = auth_storage.load().ok().flatten();
        Self {
            auth_storage,
            cached_auth: RwLock::new(cached_auth),
        }
    }

    /// Load credential for a provider with priority:
    /// 1. Environment variable (if provider_info.env_key is set)
    /// 2. Stored credentials (auth.json, keyed by config_key)
    /// 3. Config file (provider_info.experimental_bearer_token)
    pub fn load_credential(
        &self,
        config_key: &str,
        provider_info: &ModelProviderInfo,
    ) -> Option<ProviderCredential> {
        // Priority 1: Environment variable
        if let Some(ref env_key) = provider_info.env_key
            && let Ok(key) = std::env::var(env_key)
            && !key.trim().is_empty()
        {
            return Some(ProviderCredential::Api { key });
        }

        // Priority 2: Stored credentials (keyed by config_key)
        if let Ok(guard) = self.cached_auth.read()
            && let Some(ref auth) = *guard
            && let Some(cred) = auth.credentials.get(config_key)
        {
            return Some(cred.clone());
        }

        // Priority 3: Config file override
        if let Some(ref token) = provider_info.experimental_bearer_token {
            return Some(ProviderCredential::Api { key: token.clone() });
        }

        None
    }

    /// Create a ProviderAuth instance for a provider.
    /// Returns None if no credentials found (e.g., local Ollama).
    pub fn create_provider_auth(
        &self,
        config_key: &str,
        provider_info: &ModelProviderInfo,
    ) -> Option<Box<dyn ProviderAuth>> {
        // Returns None early if no credentials (Ollama, etc.)
        let credential = self.load_credential(config_key, provider_info)?;

        // Determine auth type based on wire_api.
        // This allows "anthropic-work" config key to use AnthropicAuth.
        match provider_info.wire_api {
            WireApi::Anthropic => Some(Box::new(AnthropicAuth::new(
                config_key.to_string(),
                credential,
            ))),
            // OpenAI-compatible providers (Chat, Responses)
            _ => Some(Box::new(OpenAIAuth::new(
                config_key.to_string(),
                credential,
            ))),
        }
    }

    /// Get last refresh time for a provider (from OAuth credential).
    pub fn get_last_refresh(&self, provider_id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        self.cached_auth
            .read()
            .ok()?
            .as_ref()?
            .get_last_refresh(provider_id)
    }

    /// Save credential for a provider.
    pub fn save_credential(
        &self,
        provider_id: &str,
        credential: ProviderCredential,
    ) -> std::io::Result<()> {
        let mut guard = self
            .cached_auth
            .write()
            .map_err(|_| std::io::Error::other("lock poisoned"))?;

        let auth_json = guard.get_or_insert_with(AuthDotJson::default);
        auth_json.set_credential(provider_id, credential);

        self.auth_storage.save(auth_json)?;
        Ok(())
    }

    /// Remove credential for a provider (last_refresh is removed automatically with it).
    pub fn remove_credential(
        &self,
        provider_id: &str,
    ) -> std::io::Result<Option<ProviderCredential>> {
        let mut guard = self
            .cached_auth
            .write()
            .map_err(|_| std::io::Error::other("lock poisoned"))?;

        let removed = if let Some(ref mut auth) = *guard {
            let removed = auth.remove_credential(provider_id);
            self.auth_storage.save(auth)?;
            removed
        } else {
            None
        };

        Ok(removed)
    }

    /// Set last refresh time for a provider's OAuth credential.
    /// Note: This only works for OAuth credentials (API keys don't have last_refresh).
    pub fn set_last_refresh(
        &self,
        provider_id: &str,
        time: chrono::DateTime<chrono::Utc>,
    ) -> std::io::Result<()> {
        let mut guard = self
            .cached_auth
            .write()
            .map_err(|_| std::io::Error::other("lock poisoned"))?;

        if let Some(ref mut auth_json) = *guard {
            auth_json.set_last_refresh(provider_id, time);
            self.auth_storage.save(auth_json)?;
        }
        Ok(())
    }

    /// Reload from storage.
    pub fn reload(&self) -> std::io::Result<()> {
        let loaded = self.auth_storage.load()?;
        if let Ok(mut guard) = self.cached_auth.write() {
            *guard = loaded;
        }
        Ok(())
    }
}

impl std::fmt::Debug for CredentialLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialLoader")
            .field("cached_auth", &self.cached_auth)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::storage::AuthCredentialsStoreMode;
    use crate::auth::storage::create_auth_storage;
    use tempfile::tempdir;

    fn create_test_provider(env_key: Option<&str>, wire_api: WireApi) -> ModelProviderInfo {
        ModelProviderInfo {
            config_key: "test".into(),
            name: "Test".into(),
            base_url: Some("https://api.example.com".into()),
            env_key: env_key.map(|s| s.to_string()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
        }
    }

    #[test]
    fn test_load_credential_from_storage() {
        let codex_home = tempdir().unwrap();
        let storage = create_auth_storage(
            codex_home.path().to_path_buf(),
            AuthCredentialsStoreMode::File,
        );

        // Pre-save a credential
        let mut auth = AuthDotJson::default();
        auth.set_credential(
            "test-provider",
            ProviderCredential::Api {
                key: "sk-stored-key".to_string(),
            },
        );
        storage.save(&auth).unwrap();

        let loader = CredentialLoader::new(storage);
        let provider_info = create_test_provider(None, WireApi::Chat);

        let cred = loader
            .load_credential("test-provider", &provider_info)
            .expect("credential should be loaded");

        assert!(matches!(
            cred,
            ProviderCredential::Api { key } if key == "sk-stored-key"
        ));
    }

    #[test]
    fn test_load_credential_from_config_override() {
        let codex_home = tempdir().unwrap();
        let storage = create_auth_storage(
            codex_home.path().to_path_buf(),
            AuthCredentialsStoreMode::File,
        );
        let loader = CredentialLoader::new(storage);

        let mut provider_info = create_test_provider(None, WireApi::Chat);
        provider_info.experimental_bearer_token = Some("config-token".to_string());

        let cred = loader
            .load_credential("any-key", &provider_info)
            .expect("credential should be loaded from config");

        assert!(matches!(
            cred,
            ProviderCredential::Api { key } if key == "config-token"
        ));
    }

    #[test]
    fn test_create_provider_auth_openai() {
        let codex_home = tempdir().unwrap();
        let storage = create_auth_storage(
            codex_home.path().to_path_buf(),
            AuthCredentialsStoreMode::File,
        );

        // Pre-save OpenAI credential
        let mut auth = AuthDotJson::default();
        auth.set_credential(
            "openai",
            ProviderCredential::Api {
                key: "sk-openai".to_string(),
            },
        );
        storage.save(&auth).unwrap();

        let loader = CredentialLoader::new(storage);
        let provider_info = create_test_provider(None, WireApi::Responses);

        let provider_auth = loader
            .create_provider_auth("openai", &provider_info)
            .expect("provider auth should be created");

        assert_eq!(provider_auth.provider_id(), "openai");
        assert_eq!(provider_auth.get_token(), Some("sk-openai".to_string()));
        assert_eq!(provider_auth.auth_scheme(), codex_api::AuthScheme::Bearer);
    }

    #[test]
    fn test_create_provider_auth_anthropic() {
        let codex_home = tempdir().unwrap();
        let storage = create_auth_storage(
            codex_home.path().to_path_buf(),
            AuthCredentialsStoreMode::File,
        );

        // Pre-save Anthropic credential
        let mut auth = AuthDotJson::default();
        auth.set_credential(
            "anthropic",
            ProviderCredential::Api {
                key: "sk-ant-xxx".to_string(),
            },
        );
        storage.save(&auth).unwrap();

        let loader = CredentialLoader::new(storage);
        let provider_info = create_test_provider(None, WireApi::Anthropic);

        let provider_auth = loader
            .create_provider_auth("anthropic", &provider_info)
            .expect("provider auth should be created");

        assert_eq!(provider_auth.provider_id(), "anthropic");
        assert_eq!(provider_auth.get_token(), Some("sk-ant-xxx".to_string()));
        assert_eq!(provider_auth.auth_scheme(), codex_api::AuthScheme::ApiKey);
    }

    #[test]
    fn test_no_credential_returns_none() {
        let codex_home = tempdir().unwrap();
        let storage = create_auth_storage(
            codex_home.path().to_path_buf(),
            AuthCredentialsStoreMode::File,
        );
        let loader = CredentialLoader::new(storage);
        let provider_info = create_test_provider(None, WireApi::Chat);

        let provider_auth = loader.create_provider_auth("ollama", &provider_info);
        assert!(provider_auth.is_none());
    }

    #[test]
    fn test_save_and_load_credential() {
        let codex_home = tempdir().unwrap();
        let storage = create_auth_storage(
            codex_home.path().to_path_buf(),
            AuthCredentialsStoreMode::File,
        );
        let loader = CredentialLoader::new(storage);

        loader
            .save_credential(
                "new-provider",
                ProviderCredential::Api {
                    key: "new-key".to_string(),
                },
            )
            .unwrap();

        let provider_info = create_test_provider(None, WireApi::Chat);
        let cred = loader
            .load_credential("new-provider", &provider_info)
            .expect("saved credential should be loadable");

        assert!(matches!(
            cred,
            ProviderCredential::Api { key } if key == "new-key"
        ));
    }

    #[test]
    fn test_remove_credential() {
        let codex_home = tempdir().unwrap();
        let storage = create_auth_storage(
            codex_home.path().to_path_buf(),
            AuthCredentialsStoreMode::File,
        );
        let loader = CredentialLoader::new(storage);

        loader
            .save_credential(
                "to-remove",
                ProviderCredential::Api {
                    key: "temp-key".to_string(),
                },
            )
            .unwrap();

        let removed = loader.remove_credential("to-remove").unwrap();
        assert!(removed.is_some());

        let provider_info = create_test_provider(None, WireApi::Chat);
        let cred = loader.load_credential("to-remove", &provider_info);
        assert!(cred.is_none());
    }
}
