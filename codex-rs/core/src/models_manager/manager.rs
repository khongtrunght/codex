use chrono::Utc;
use codex_api::ModelsClient;
use codex_api::ReqwestTransport;
use codex_app_server_protocol::AuthMode;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ModelsResponse;
use futures::future::join_all;
use http::HeaderMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::sync::TryLockError;
use tracing::error;

use super::cache;
use super::cache::ModelsCache;
use crate::api_bridge::auth_provider_from_auth;
use crate::api_bridge::map_api_error;
use crate::auth::AuthManager;
use crate::config::Config;
use crate::default_client::build_reqwest_client;
use crate::error::Result as CoreResult;
use crate::features::Feature;
use crate::model_provider_info::ModelProviderInfo;
use crate::models_manager::model_family::ModelFamily;
use crate::models_manager::model_presets::builtin_model_presets;

// Legacy cache file (kept for backwards compatibility reference)
#[allow(dead_code)]
const MODEL_CACHE_FILE: &str = "models_cache.json";
const DEFAULT_MODEL_CACHE_TTL: Duration = Duration::from_secs(86400); // 24 hours
const OPENAI_DEFAULT_API_MODEL: &str = "gpt-5.1-codex-max";
const OPENAI_DEFAULT_CHATGPT_MODEL: &str = "gpt-5.2-codex";
const CODEX_AUTO_BALANCED_MODEL: &str = "codex-auto-balanced";

/// Coordinates remote model discovery plus cached metadata on disk.
/// Supports fetching models from multiple providers concurrently.
#[derive(Debug)]
pub struct ModelsManager {
    // todo(aibrahim) merge available_models and model family creation into one struct
    local_models: Vec<ModelPreset>,
    /// Remote models indexed by provider key (e.g., "openai", "ollama")
    remote_models: RwLock<HashMap<String, Vec<ModelInfo>>>,
    auth_manager: Arc<AuthManager>,
    /// ETags indexed by provider key for cache validation
    etags: RwLock<HashMap<String, Option<String>>>,
    codex_home: PathBuf,
    cache_ttl: Duration,
}

impl ModelsManager {
    /// Construct a manager scoped to the provided `AuthManager`.
    pub fn new(auth_manager: Arc<AuthManager>) -> Self {
        let codex_home = auth_manager.codex_home().to_path_buf();
        // Load bundled models as initial OpenAI models for backwards compatibility
        let mut initial_models = HashMap::new();
        if let Ok(bundled) = Self::load_remote_models_from_file() {
            initial_models.insert("openai".to_string(), bundled);
        }
        Self {
            local_models: builtin_model_presets(auth_manager.get_auth_mode()),
            remote_models: RwLock::new(initial_models),
            auth_manager,
            etags: RwLock::new(HashMap::new()),
            codex_home,
            cache_ttl: DEFAULT_MODEL_CACHE_TTL,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Construct a manager scoped to the provided `AuthManager` with a specific provider. Used for integration tests.
    pub fn with_provider(auth_manager: Arc<AuthManager>, provider: ModelProviderInfo) -> Self {
        let codex_home = auth_manager.codex_home().to_path_buf();
        let mut initial_models = HashMap::new();
        if let Ok(bundled) = Self::load_remote_models_from_file() {
            initial_models.insert(provider.config_key.clone(), bundled);
        }
        Self {
            local_models: builtin_model_presets(auth_manager.get_auth_mode()),
            remote_models: RwLock::new(initial_models),
            auth_manager,
            etags: RwLock::new(HashMap::new()),
            codex_home,
            cache_ttl: DEFAULT_MODEL_CACHE_TTL,
        }
    }

    /// Fetch models from all configured providers concurrently.
    /// Providers that fail to respond are silently skipped.
    pub async fn refresh_all_providers(&self, config: &Config) -> CoreResult<()> {
        if !config.features.enabled(Feature::RemoteModels) {
            tracing::debug!("[refresh_all_providers] RemoteModels feature disabled, skipping");
            return Ok(());
        }

        let providers: Vec<(String, ModelProviderInfo)> = config
            .model_providers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let provider_keys: Vec<&str> = providers.iter().map(|(k, _)| k.as_str()).collect();
        tracing::debug!("[refresh_all_providers] Starting fetch for {} providers: {:?}",
            providers.len(), provider_keys);

        let fetch_futures: Vec<_> = providers
            .into_iter()
            .map(|(provider_key, provider_info)| {
                self.fetch_provider_models(provider_key, provider_info, config)
            })
            .collect();

        // Run all fetches concurrently, collecting results
        let results = join_all(fetch_futures).await;

        // Log errors but don't fail - unavailable providers just won't have models
        for result in results {
            if let Err(e) = result {
                tracing::debug!("Provider fetch failed (skipping): {}", e);
            }
        }

        tracing::debug!("[refresh_all_providers] Completed all provider fetches");

        Ok(())
    }

    /// Fetch models from a single provider, respecting cache.
    async fn fetch_provider_models(
        &self,
        provider_key: String,
        provider: ModelProviderInfo,
        _config: &Config,
    ) -> CoreResult<()> {
        tracing::debug!("[fetch_provider_models] Starting for provider: {}", provider_key);

        // Skip API key mode - only fetch for ChatGPT auth or no-auth providers
        let auth_mode = self.auth_manager.get_auth_mode();
        tracing::debug!("[fetch_provider_models] {} - auth_mode: {:?}, requires_openai_auth: {}",
            provider_key, auth_mode, provider.requires_openai_auth);

        if auth_mode == Some(AuthMode::ApiKey) && provider.requires_openai_auth {
            tracing::debug!("[fetch_provider_models] {} - SKIPPED: ApiKey mode with requires_openai_auth", provider_key);
            return Ok(());
        }

        // Skip if provider requires auth we don't have
        if provider.requires_openai_auth && auth_mode.is_none() {
            tracing::debug!("[fetch_provider_models] {} - SKIPPED: requires_openai_auth but no auth_mode", provider_key);
            return Ok(());
        }

        // Skip if required env_key is missing
        if provider.env_key.is_some() && provider.api_key().is_err() {
            tracing::debug!("[fetch_provider_models] {} - SKIPPED: env_key {:?} not set", provider_key, provider.env_key);
            return Ok(());
        }

        // Try loading from per-provider cache first
        if self.try_load_provider_cache(&provider_key).await {
            tracing::debug!("[fetch_provider_models] {} - loaded from cache, skipping network fetch", provider_key);
            return Ok(());
        }

        tracing::debug!("[fetch_provider_models] {} - proceeding to network fetch", provider_key);

        // Fetch from network
        let auth = self.auth_manager.auth();
        let api_provider = match provider.to_api_provider(auth_mode) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("[fetch_provider_models] {} - ERROR creating api_provider: {:?}", provider_key, e);
                return Err(e);
            }
        };
        tracing::debug!("[fetch_provider_models] {} - api_provider base_url: {}", provider_key, api_provider.base_url);

        let api_auth = match auth_provider_from_auth(auth.clone(), &provider).await {
            Ok(a) => a,
            Err(e) => {
                tracing::debug!("[fetch_provider_models] {} - ERROR creating api_auth: {:?}", provider_key, e);
                return Err(e);
            }
        };

        let transport = ReqwestTransport::new(build_reqwest_client());
        let client = ModelsClient::new(transport, api_provider, api_auth);

        let client_version = format_client_version_to_whole();
        tracing::debug!("[fetch_provider_models] {} - calling list_models...", provider_key);

        let ModelsResponse { models, etag } = match client
            .list_models(&client_version, HeaderMap::new())
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::debug!("[fetch_provider_models] {} - NETWORK ERROR: {:?}", provider_key, e);
                return Err(map_api_error(e));
            }
        };

        tracing::debug!("[fetch_provider_models] {} - SUCCESS: fetched {} models", provider_key, models.len());

        let etag = (!etag.is_empty()).then_some(etag);

        // Store with provider key
        self.remote_models
            .write()
            .await
            .insert(provider_key.clone(), models.clone());
        self.etags
            .write()
            .await
            .insert(provider_key.clone(), etag.clone());

        // Persist to per-provider cache file
        self.persist_provider_cache(&provider_key, &models, etag).await;

        tracing::debug!("[fetch_provider_models] {} - stored and cached {} models", provider_key, models.len());

        Ok(())
    }

    /// Legacy method for backwards compatibility - refreshes only the current provider.
    pub async fn refresh_available_models(&self, config: &Config) -> CoreResult<()> {
        self.fetch_provider_models(
            config.model_provider_id.clone(),
            config.model_provider.clone(),
            config,
        )
        .await
    }

    /// List models from all providers with provider prefixes.
    pub async fn list_models(&self, config: &Config) -> Vec<ModelPreset> {
        if let Err(err) = self.refresh_all_providers(config).await {
            error!("failed to refresh available models: {err}");
        }
        self.build_available_models_multi_provider(config).await
    }

    /// Non-blocking version - returns what's cached from all providers.
    pub fn try_list_models(&self, config: &Config) -> Result<Vec<ModelPreset>, TryLockError> {
        self.try_build_available_models_multi_provider(config)
    }

    fn find_family_for_model(slug: &str) -> ModelFamily {
        super::model_family::find_family_for_model(slug)
    }

    /// Look up the requested model family while applying remote metadata overrides.
    /// Handles prefixed model names like "openai/gpt-4" by extracting the provider.
    pub async fn construct_model_family(&self, model: &str, config: &Config) -> ModelFamily {
        let (provider_key, model_slug) = parse_model_with_provider(model);

        // Get remote models from the correct provider
        let remote_models = if let Some(key) = provider_key {
            self.remote_models
                .read()
                .await
                .get(key)
                .cloned()
                .unwrap_or_default()
        } else {
            self.remote_models
                .read()
                .await
                .get(&config.model_provider_id)
                .cloned()
                .unwrap_or_default()
        };

        Self::find_family_for_model(model_slug)
            .with_remote_overrides(remote_models)
            .with_config_overrides(config)
    }

    pub async fn get_model(&self, model: &Option<String>, config: &Config) -> String {
        if let Some(model) = model.as_ref() {
            return model.to_string();
        }
        if let Err(err) = self.refresh_available_models(config).await {
            error!("failed to refresh available models: {err}");
        }
        // if codex-auto-balanced exists & signed in with chatgpt mode, return it, otherwise return the default model
        let auth_mode = self.auth_manager.get_auth_mode();
        let available_models = self.build_available_models_multi_provider(config).await;
        // Check for codex-auto-balanced with or without prefix
        if auth_mode == Some(AuthMode::ChatGPT)
            && available_models.iter().any(|m| {
                m.model == CODEX_AUTO_BALANCED_MODEL
                    || m.model.ends_with(&format!("/{}", CODEX_AUTO_BALANCED_MODEL))
            })
        {
            return CODEX_AUTO_BALANCED_MODEL.to_string();
        } else if auth_mode == Some(AuthMode::ChatGPT) {
            return OPENAI_DEFAULT_CHATGPT_MODEL.to_string();
        }
        OPENAI_DEFAULT_API_MODEL.to_string()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn get_model_offline(model: Option<&str>) -> String {
        model.unwrap_or(OPENAI_DEFAULT_CHATGPT_MODEL).to_string()
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Offline helper that builds a `ModelFamily` without consulting remote state.
    pub fn construct_model_family_offline(model: &str, config: &Config) -> ModelFamily {
        Self::find_family_for_model(model).with_config_overrides(config)
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Test helper to get remote models for a specific provider.
    pub async fn remote_models(&self, config: &Config) -> Vec<ModelInfo> {
        if config.features.enabled(Feature::RemoteModels) {
            self.remote_models
                .read()
                .await
                .get(&config.model_provider_id)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Test helper to build available models without provider prefix (legacy behavior).
    pub fn build_available_models(&self, mut remote_models: Vec<ModelInfo>) -> Vec<ModelPreset> {
        remote_models.sort_by(|a, b| a.priority.cmp(&b.priority));

        let remote_presets: Vec<ModelPreset> = remote_models.into_iter().map(Into::into).collect();
        let existing_presets = self.local_models.clone();
        let mut merged_presets = Self::merge_presets(remote_presets, existing_presets);
        merged_presets = self.filter_visible_models(merged_presets);

        let has_default = merged_presets.iter().any(|preset| preset.is_default);
        if let Some(default) = merged_presets.first_mut()
            && !has_default
        {
            default.is_default = true;
        }

        merged_presets
    }

    #[cfg(any(test, feature = "test-support"))]
    fn merge_presets(
        remote_presets: Vec<ModelPreset>,
        existing_presets: Vec<ModelPreset>,
    ) -> Vec<ModelPreset> {
        if remote_presets.is_empty() {
            return existing_presets;
        }

        let remote_slugs: HashSet<&str> = remote_presets
            .iter()
            .map(|preset| preset.model.as_str())
            .collect();

        let mut merged_presets = remote_presets.clone();
        for mut preset in existing_presets {
            if remote_slugs.contains(preset.model.as_str()) {
                continue;
            }
            preset.is_default = false;
            merged_presets.push(preset);
        }

        merged_presets
    }

    fn load_remote_models_from_file() -> Result<Vec<ModelInfo>, std::io::Error> {
        let file_contents = include_str!("../../models.json");
        let response: ModelsResponse = serde_json::from_str(file_contents)?;
        Ok(response.models)
    }

    /// Attempt to load models for a specific provider from cache.
    async fn try_load_provider_cache(&self, provider_key: &str) -> bool {
        let cache_path = self.cache_path_for_provider(provider_key);
        tracing::debug!("[try_load_provider_cache] {} - checking cache at {:?}", provider_key, cache_path);

        let cache = match cache::load_cache(&cache_path).await {
            Ok(cache) => cache,
            Err(err) => {
                tracing::debug!("[try_load_provider_cache] {} - failed to load: {}", provider_key, err);
                return false;
            }
        };
        let cache = match cache {
            Some(cache) => cache,
            None => {
                tracing::debug!("[try_load_provider_cache] {} - cache file not found", provider_key);
                return false;
            }
        };
        if !cache.is_fresh(self.cache_ttl) {
            tracing::debug!("[try_load_provider_cache] {} - cache expired (fetched_at: {:?}, ttl: {:?})",
                provider_key, cache.fetched_at, self.cache_ttl);
            return false;
        }
        let models = cache.models.clone();
        tracing::debug!("[try_load_provider_cache] {} - loaded {} models from cache", provider_key, models.len());
        self.remote_models
            .write()
            .await
            .insert(provider_key.to_string(), models);
        self.etags
            .write()
            .await
            .insert(provider_key.to_string(), cache.etag.clone());
        true
    }

    /// Serialize the latest fetch to disk for a specific provider.
    async fn persist_provider_cache(
        &self,
        provider_key: &str,
        models: &[ModelInfo],
        etag: Option<String>,
    ) {
        let cache = ModelsCache {
            fetched_at: Utc::now(),
            etag,
            models: models.to_vec(),
        };
        let cache_path = self.cache_path_for_provider(provider_key);
        if let Err(err) = cache::save_cache(&cache_path, &cache).await {
            error!(
                "failed to write provider cache for {}: {}",
                provider_key, err
            );
        }
    }

    /// Build models from all providers with provider prefixes (async version).
    async fn build_available_models_multi_provider(&self, config: &Config) -> Vec<ModelPreset> {
        if !config.features.enabled(Feature::RemoteModels) {
            return self.build_local_models_only(config);
        }

        let remote_models_map = self.remote_models.read().await.clone();
        self.build_prefixed_models(remote_models_map, config)
    }

    /// Build models from all providers with provider prefixes (non-blocking version).
    fn try_build_available_models_multi_provider(
        &self,
        config: &Config,
    ) -> Result<Vec<ModelPreset>, TryLockError> {
        if !config.features.enabled(Feature::RemoteModels) {
            return Ok(self.build_local_models_only(config));
        }

        let remote_models_map = self.remote_models.try_read()?.clone();
        Ok(self.build_prefixed_models(remote_models_map, config))
    }

    /// Build local models only (when remote models are disabled).
    fn build_local_models_only(&self, config: &Config) -> Vec<ModelPreset> {
        let provider_key = &config.model_provider_id;
        let mut presets: Vec<ModelPreset> = self
            .local_models
            .clone()
            .into_iter()
            .map(|mut preset| {
                preset.model = format!("{}/{}", provider_key, preset.model);
                preset.display_name = format!("{}/{}", provider_key, preset.display_name);
                preset
            })
            .collect();
        presets = self.filter_visible_models(presets);

        if !presets.iter().any(|p| p.is_default) {
            if let Some(first) = presets.first_mut() {
                first.is_default = true;
            }
        }
        presets
    }

    /// Build prefixed models from remote models map.
    fn build_prefixed_models(
        &self,
        remote_models_map: HashMap<String, Vec<ModelInfo>>,
        config: &Config,
    ) -> Vec<ModelPreset> {
        let mut all_presets: Vec<ModelPreset> = Vec::new();
        let mut seen_slugs: HashSet<String> = HashSet::new();

        // Process remote models from each provider
        for (provider_key, mut models) in remote_models_map {
            models.sort_by(|a, b| a.priority.cmp(&b.priority));

            for model in models {
                let mut preset: ModelPreset = model.into();
                let prefixed_model = format!("{}/{}", provider_key, preset.model);

                // Skip duplicates
                if seen_slugs.contains(&prefixed_model) {
                    continue;
                }
                seen_slugs.insert(prefixed_model.clone());

                preset.model = prefixed_model;
                preset.display_name = format!("{}/{}", provider_key, preset.display_name);
                preset.id = format!("{}/{}", provider_key, preset.id);
                all_presets.push(preset);
            }
        }

        // Add local/builtin presets for current provider if not already present
        let provider_key = &config.model_provider_id;
        for preset in self.local_models.clone() {
            let prefixed_model = format!("{}/{}", provider_key, preset.model);
            if seen_slugs.contains(&prefixed_model) {
                continue;
            }
            seen_slugs.insert(prefixed_model.clone());

            let mut new_preset = preset;
            new_preset.model = prefixed_model;
            new_preset.display_name = format!("{}/{}", provider_key, new_preset.display_name);
            new_preset.id = format!("{}/{}", provider_key, new_preset.id);
            new_preset.is_default = false; // Local presets are not default when we have remote
            all_presets.push(new_preset);
        }

        // Note: Models are already sorted by priority within each provider.
        // We don't re-sort here to preserve the priority ordering.

        // Filter for visibility
        all_presets = self.filter_visible_models(all_presets);

        // Mark first as default if none is marked
        if !all_presets.iter().any(|p| p.is_default) {
            if let Some(first) = all_presets.first_mut() {
                first.is_default = true;
            }
        }

        all_presets
    }

    fn filter_visible_models(&self, models: Vec<ModelPreset>) -> Vec<ModelPreset> {
        let chatgpt_mode = self.auth_manager.get_auth_mode() == Some(AuthMode::ChatGPT);
        models
            .into_iter()
            .filter(|model| model.show_in_picker && (chatgpt_mode || model.supported_in_api))
            .collect()
    }

    fn cache_path_for_provider(&self, provider_key: &str) -> PathBuf {
        self.codex_home
            .join(format!("models_cache_{}.json", provider_key))
    }
}

/// Parse a prefixed model name like "openai/gpt-4" into (Some("openai"), "gpt-4").
/// Returns (None, model) for legacy unprefixed names.
pub fn parse_model_with_provider(model: &str) -> (Option<&str>, &str) {
    match model.split_once('/') {
        Some((provider, slug)) => (Some(provider), slug),
        None => (None, model),
    }
}

/// Convert a client version string to a whole version string (e.g. "1.2.3-alpha.4" -> "1.2.3")
fn format_client_version_to_whole() -> String {
    format_client_version_from_parts(
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
    )
}

fn format_client_version_from_parts(major: &str, minor: &str, patch: &str) -> String {
    const DEV_VERSION: &str = "0.0.0";
    const FALLBACK_VERSION: &str = "99.99.99";

    let normalized = format!("{major}.{minor}.{patch}");

    if normalized == DEV_VERSION {
        FALLBACK_VERSION.to_string()
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::cache::ModelsCache;
    use super::*;
    use crate::CodexAuth;
    use crate::auth::AuthCredentialsStoreMode;
    use crate::config::ConfigBuilder;
    use crate::features::Feature;
    use crate::model_provider_info::WireApi;
    use codex_protocol::openai_models::ModelsResponse;
    use core_test_support::responses::mount_models_once;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::tempdir;
    use wiremock::MockServer;

    fn remote_model(slug: &str, display: &str, priority: i32) -> ModelInfo {
        remote_model_with_visibility(slug, display, priority, "list")
    }

    fn remote_model_with_visibility(
        slug: &str,
        display: &str,
        priority: i32,
        visibility: &str,
    ) -> ModelInfo {
        serde_json::from_value(json!({
            "slug": slug,
            "display_name": display,
            "description": format!("{display} desc"),
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [{"effort": "low", "description": "low"}, {"effort": "medium", "description": "medium"}],
            "shell_type": "shell_command",
            "visibility": visibility,
            "minimal_client_version": [0, 1, 0],
            "supported_in_api": true,
            "priority": priority,
            "upgrade": null,
            "base_instructions": null,
            "supports_reasoning_summaries": false,
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": null,
            "truncation_policy": {"mode": "bytes", "limit": 10_000},
            "supports_parallel_tool_calls": false,
            "context_window": null,
            "reasoning_summary_format": "none",
            "experimental_supported_tools": [],
        }))
        .expect("valid model")
    }

    fn provider_for(base_url: String) -> ModelProviderInfo {
        ModelProviderInfo {
            config_key: "mock".into(),
            name: "mock".into(),
            base_url: Some(base_url),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: Some(0),
            stream_max_retries: Some(0),
            stream_idle_timeout_ms: Some(5_000),
            requires_openai_auth: false,
        }
    }

    #[tokio::test]
    async fn refresh_available_models_sorts_and_marks_default() {
        let server = MockServer::start().await;
        let remote_models = vec![
            remote_model("priority-low", "Low", 1),
            remote_model("priority-high", "High", 0),
        ];
        let models_mock = mount_models_once(
            &server,
            ModelsResponse {
                models: remote_models.clone(),
                etag: String::new(),
            },
        )
        .await;

        let codex_home = tempdir().expect("temp dir");
        let provider = provider_for(server.uri());
        let mut config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("load default test config");
        config.features.enable(Feature::RemoteModels);
        // Use the mock provider in config
        config.model_provider_id = provider.config_key.clone();
        config.model_provider = provider.clone();
        let auth_manager =
            AuthManager::from_auth_for_testing(CodexAuth::create_dummy_chatgpt_auth_for_testing());
        let manager = ModelsManager::with_provider(auth_manager, provider);

        manager
            .refresh_available_models(&config)
            .await
            .expect("refresh succeeds");
        let cached_remote = manager.remote_models(&config).await;
        assert_eq!(cached_remote, remote_models);

        let available = manager.list_models(&config).await;
        // Models now have provider prefix
        let high_model = format!("{}/priority-high", config.model_provider_id);
        let low_model = format!("{}/priority-low", config.model_provider_id);
        let high_idx = available
            .iter()
            .position(|model| model.model == high_model)
            .expect("priority-high should be listed");
        let low_idx = available
            .iter()
            .position(|model| model.model == low_model)
            .expect("priority-low should be listed");
        assert!(
            high_idx < low_idx,
            "higher priority should be listed before lower priority"
        );
        assert!(
            available[high_idx].is_default,
            "highest priority should be default"
        );
        assert!(!available[low_idx].is_default);
        assert_eq!(
            models_mock.requests().len(),
            1,
            "expected a single /models request"
        );
    }

    #[tokio::test]
    async fn refresh_available_models_uses_cache_when_fresh() {
        let server = MockServer::start().await;
        let remote_models = vec![remote_model("cached", "Cached", 5)];
        let models_mock = mount_models_once(
            &server,
            ModelsResponse {
                models: remote_models.clone(),
                etag: String::new(),
            },
        )
        .await;

        let codex_home = tempdir().expect("temp dir");
        let provider = provider_for(server.uri());
        let mut config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("load default test config");
        config.features.enable(Feature::RemoteModels);
        // Use the mock provider in config
        config.model_provider_id = provider.config_key.clone();
        config.model_provider = provider.clone();
        let auth_manager = Arc::new(AuthManager::new(
            codex_home.path().to_path_buf(),
            false,
            AuthCredentialsStoreMode::File,
        ));
        let manager = ModelsManager::with_provider(auth_manager, provider);

        manager
            .refresh_available_models(&config)
            .await
            .expect("first refresh succeeds");
        assert_eq!(
            manager.remote_models(&config).await,
            remote_models,
            "remote cache should store fetched models"
        );

        // Second call should read from cache and avoid the network.
        manager
            .refresh_available_models(&config)
            .await
            .expect("cached refresh succeeds");
        assert_eq!(
            manager.remote_models(&config).await,
            remote_models,
            "cache path should not mutate stored models"
        );
        assert_eq!(
            models_mock.requests().len(),
            1,
            "cache hit should avoid a second /models request"
        );
    }

    #[tokio::test]
    async fn refresh_available_models_refetches_when_cache_stale() {
        let server = MockServer::start().await;
        let initial_models = vec![remote_model("stale", "Stale", 1)];
        let initial_mock = mount_models_once(
            &server,
            ModelsResponse {
                models: initial_models.clone(),
                etag: String::new(),
            },
        )
        .await;

        let codex_home = tempdir().expect("temp dir");
        let provider = provider_for(server.uri());
        let mut config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("load default test config");
        config.features.enable(Feature::RemoteModels);
        // Use the mock provider in config
        config.model_provider_id = provider.config_key.clone();
        config.model_provider = provider.clone();
        let auth_manager = Arc::new(AuthManager::new(
            codex_home.path().to_path_buf(),
            false,
            AuthCredentialsStoreMode::File,
        ));
        let manager = ModelsManager::with_provider(auth_manager, provider.clone());

        manager
            .refresh_available_models(&config)
            .await
            .expect("initial refresh succeeds");

        // Rewrite cache with an old timestamp so it is treated as stale.
        // Cache is now per-provider
        let cache_path = codex_home
            .path()
            .join(format!("models_cache_{}.json", provider.config_key));
        let contents =
            std::fs::read_to_string(&cache_path).expect("cache file should exist after refresh");
        let mut cache: ModelsCache =
            serde_json::from_str(&contents).expect("cache should deserialize");
        cache.fetched_at = Utc::now() - chrono::Duration::hours(1);
        std::fs::write(&cache_path, serde_json::to_string_pretty(&cache).unwrap())
            .expect("cache rewrite succeeds");

        let updated_models = vec![remote_model("fresh", "Fresh", 9)];
        server.reset().await;
        let refreshed_mock = mount_models_once(
            &server,
            ModelsResponse {
                models: updated_models.clone(),
                etag: String::new(),
            },
        )
        .await;

        manager
            .refresh_available_models(&config)
            .await
            .expect("second refresh succeeds");
        assert_eq!(
            manager.remote_models(&config).await,
            updated_models,
            "stale cache should trigger refetch"
        );
        assert_eq!(
            initial_mock.requests().len(),
            1,
            "initial refresh should only hit /models once"
        );
        assert_eq!(
            refreshed_mock.requests().len(),
            1,
            "stale cache refresh should fetch /models once"
        );
    }

    #[tokio::test]
    async fn refresh_available_models_drops_removed_remote_models() {
        let server = MockServer::start().await;
        let initial_models = vec![remote_model("remote-old", "Remote Old", 1)];
        let initial_mock = mount_models_once(
            &server,
            ModelsResponse {
                models: initial_models,
                etag: String::new(),
            },
        )
        .await;

        let codex_home = tempdir().expect("temp dir");
        let provider = provider_for(server.uri());
        let mut config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("load default test config");
        config.features.enable(Feature::RemoteModels);
        // Use the mock provider in config
        config.model_provider_id = provider.config_key.clone();
        config.model_provider = provider.clone();
        let auth_manager =
            AuthManager::from_auth_for_testing(CodexAuth::create_dummy_chatgpt_auth_for_testing());
        let mut manager = ModelsManager::with_provider(auth_manager, provider.clone());
        manager.cache_ttl = Duration::ZERO;

        manager
            .refresh_available_models(&config)
            .await
            .expect("initial refresh succeeds");

        server.reset().await;
        let refreshed_models = vec![remote_model("remote-new", "Remote New", 1)];
        let refreshed_mock = mount_models_once(
            &server,
            ModelsResponse {
                models: refreshed_models,
                etag: String::new(),
            },
        )
        .await;

        manager
            .refresh_available_models(&config)
            .await
            .expect("second refresh succeeds");

        let available = manager
            .try_list_models(&config)
            .expect("models should be available");
        // Models now have provider prefix
        let expected_new = format!("{}/remote-new", provider.config_key);
        let expected_old = format!("{}/remote-old", provider.config_key);
        assert!(
            available.iter().any(|preset| preset.model == expected_new),
            "new remote model should be listed"
        );
        assert!(
            !available.iter().any(|preset| preset.model == expected_old),
            "removed remote model should not be listed"
        );
        assert_eq!(
            initial_mock.requests().len(),
            1,
            "initial refresh should only hit /models once"
        );
        assert_eq!(
            refreshed_mock.requests().len(),
            1,
            "second refresh should only hit /models once"
        );
    }

    #[test]
    fn build_available_models_picks_default_after_hiding_hidden_models() {
        let auth_manager =
            AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
        let provider = provider_for("http://example.test".to_string());
        let mut manager = ModelsManager::with_provider(auth_manager, provider);
        manager.local_models = Vec::new();

        let hidden_model = remote_model_with_visibility("hidden", "Hidden", 0, "hide");
        let visible_model = remote_model_with_visibility("visible", "Visible", 1, "list");

        let mut expected = ModelPreset::from(visible_model.clone());
        expected.is_default = true;

        let available = manager.build_available_models(vec![hidden_model, visible_model]);

        assert_eq!(available, vec![expected]);
    }

    #[test]
    fn bundled_models_json_roundtrips() {
        let file_contents = include_str!("../../models.json");
        let response: ModelsResponse =
            serde_json::from_str(file_contents).expect("bundled models.json should deserialize");

        let serialized =
            serde_json::to_string(&response).expect("bundled models.json should serialize");
        let roundtripped: ModelsResponse =
            serde_json::from_str(&serialized).expect("serialized models.json should deserialize");

        assert_eq!(
            response, roundtripped,
            "bundled models.json should round trip through serde"
        );
        assert!(
            !response.models.is_empty(),
            "bundled models.json should contain at least one model"
        );
    }
}
