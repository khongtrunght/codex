use chrono::Utc;
use codex_core::auth::AuthCredentialsStoreMode;
use codex_core::auth::AuthManager;
use codex_core::auth::ProviderCredential;
use codex_core::auth::ProviderAuth;
use codex_core::auth::AnthropicAuth;
use codex_core::auth::OpenAIAuth;
use codex_core::ModelProviderInfo;
use codex_core::WireApi;
use pretty_assertions::assert_eq;
use tempfile::tempdir;

#[test]
fn test_multi_provider_credential_storage() {
    let codex_home = tempdir().unwrap();
    let manager = AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    );

    // Login with OpenAI
    manager
        .login_provider_with_api_key("openai", "sk-openai-key")
        .unwrap();

    // Login with Anthropic
    manager
        .login_provider_with_api_key("anthropic", "sk-ant-key")
        .unwrap();

    // Verify both are stored
    let openai_provider = ModelProviderInfo::create_openai_provider();
    let anthropic_provider = create_anthropic_provider();

    let openai_auth = manager
        .get_provider_auth("openai", &openai_provider)
        .unwrap();
    assert_eq!(openai_auth.get_token(), Some("sk-openai-key".to_string()));

    let anthropic_auth = manager
        .get_provider_auth("anthropic", &anthropic_provider)
        .unwrap();
    assert_eq!(anthropic_auth.get_token(), Some("sk-ant-key".to_string()));
}

#[test]
fn test_auth_scheme_per_provider() {
    let codex_home = tempdir().unwrap();
    let manager = AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    );

    manager
        .login_provider_with_api_key("openai", "sk-openai")
        .unwrap();
    manager
        .login_provider_with_api_key("anthropic", "sk-ant")
        .unwrap();

    let openai_provider = ModelProviderInfo::create_openai_provider();
    let anthropic_provider = create_anthropic_provider();

    let openai_auth = manager
        .get_provider_auth("openai", &openai_provider)
        .unwrap();
    assert_eq!(openai_auth.auth_scheme(), codex_api::AuthScheme::Bearer);

    // API key uses x-api-key header
    let anthropic_auth = manager
        .get_provider_auth("anthropic", &anthropic_provider)
        .unwrap();
    assert_eq!(anthropic_auth.auth_scheme(), codex_api::AuthScheme::ApiKey);
    assert!(anthropic_auth.extra_headers().is_empty()); // No extra headers for API key
}

#[test]
fn test_anthropic_oauth_uses_bearer_with_beta_header() {
    // OAuth credential uses Bearer scheme (not x-api-key!)
    let oauth_cred = ProviderCredential::OAuth {
        access_token: "ant-access-token".to_string(),
        refresh_token: "ant-refresh-token".to_string(),
        expires_at: None,
        last_refresh: Some(Utc::now()),
        account_id: None,
        exchanged_api_key: None, // Anthropic OAuth doesn't use exchanged_api_key
        extra: None,
    };
    let auth = AnthropicAuth::from_credential(oauth_cred);

    // OAuth uses Bearer scheme
    assert_eq!(auth.auth_scheme(), codex_api::AuthScheme::Bearer);

    // OAuth requires anthropic-beta header
    let extra_headers = auth.extra_headers();
    assert!(!extra_headers.is_empty());
    assert_eq!(
        extra_headers
            .get("anthropic-beta")
            .map(|v| v.to_str().unwrap()),
        Some("oauth-2025-04-20")
    );
}

#[test]
fn test_openai_oauth_with_exchanged_api_key() {
    // OpenAI OAuth with exchanged API key (dual-storage pattern)
    let oauth_cred = ProviderCredential::OAuth {
        access_token: "openai-access-token".to_string(),
        refresh_token: "openai-refresh-token".to_string(),
        expires_at: None,
        last_refresh: Some(Utc::now()),
        account_id: Some("acct-123".to_string()),
        exchanged_api_key: Some("sk-exchanged-key".to_string()), // Has exchanged API key
        extra: None,
    };
    let auth = OpenAIAuth::from_credential(oauth_cred);

    // Should return the exchanged API key, not the access_token
    assert_eq!(auth.get_token(), Some("sk-exchanged-key".to_string()));
    assert_eq!(auth.auth_scheme(), codex_api::AuthScheme::Bearer);
    assert!(auth.has_exchanged_api_key());
}

#[test]
fn test_openai_oauth_without_exchanged_api_key() {
    // OpenAI OAuth without exchanged API key (fallback to access_token)
    let oauth_cred = ProviderCredential::OAuth {
        access_token: "openai-access-token".to_string(),
        refresh_token: "openai-refresh-token".to_string(),
        expires_at: None,
        last_refresh: Some(Utc::now()),
        account_id: None,
        exchanged_api_key: None, // No exchanged API key
        extra: None,
    };
    let auth = OpenAIAuth::from_credential(oauth_cred);

    // Should return access_token when no exchanged_api_key
    assert_eq!(auth.get_token(), Some("openai-access-token".to_string()));
    assert!(!auth.has_exchanged_api_key());
}

#[test]
fn test_anthropic_oauth_credential_storage() {
    let codex_home = tempdir().unwrap();
    let manager = AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    );

    // Store Anthropic OAuth credential (last_refresh is inside the credential)
    let oauth_cred = ProviderCredential::OAuth {
        access_token: "ant-access-token".to_string(),
        refresh_token: "ant-refresh-token".to_string(),
        expires_at: Some(Utc::now() + chrono::Duration::hours(24)),
        last_refresh: Some(Utc::now()),
        account_id: None,
        exchanged_api_key: None, // Anthropic OAuth doesn't use exchanged_api_key
        extra: None,
    };

    manager
        .credential_loader()
        .save_credential("anthropic", oauth_cred)
        .unwrap();

    // Verify it loads correctly
    let anthropic_provider = create_anthropic_provider();
    let anthropic_auth = manager
        .get_provider_auth("anthropic", &anthropic_provider)
        .unwrap();

    // OAuth credentials use Bearer scheme (not x-api-key!)
    // Reference: index.mjs lines 148-153 show OAuth uses Authorization: Bearer
    assert_eq!(anthropic_auth.auth_scheme(), codex_api::AuthScheme::Bearer);
    assert_eq!(
        anthropic_auth.get_token(),
        Some("ant-access-token".to_string())
    );
    assert!(!anthropic_auth.needs_refresh()); // Recently refreshed, shouldn't need refresh

    // OAuth requires anthropic-beta header
    let extra_headers = anthropic_auth.extra_headers();
    assert!(!extra_headers.is_empty());
    assert!(extra_headers.contains_key("anthropic-beta"));
}

#[test]
fn test_anthropic_oauth_needs_refresh() {
    // OAuth credential that expires soon - should need refresh
    let expiring_cred = ProviderCredential::OAuth {
        access_token: "token".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: Some(Utc::now() + chrono::Duration::minutes(30)), // Expires in 30 min
        last_refresh: Some(Utc::now()),
        account_id: None,
        exchanged_api_key: None,
        extra: None,
    };
    let auth = AnthropicAuth::from_credential(expiring_cred);
    assert!(auth.needs_refresh()); // Should need refresh (within 1 hour buffer)

    // OAuth credential that doesn't expire soon
    let valid_cred = ProviderCredential::OAuth {
        access_token: "token".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: Some(Utc::now() + chrono::Duration::hours(24)), // Expires in 24 hours
        last_refresh: Some(Utc::now()),
        account_id: None,
        exchanged_api_key: None,
        extra: None,
    };
    let auth = AnthropicAuth::from_credential(valid_cred);
    assert!(!auth.needs_refresh()); // Should NOT need refresh

    // OAuth credential without expires_at but recent last_refresh
    let recent_refresh_cred = ProviderCredential::OAuth {
        access_token: "token".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: None,
        last_refresh: Some(Utc::now()), // Just refreshed
        account_id: None,
        exchanged_api_key: None,
        extra: None,
    };
    let auth = AnthropicAuth::from_credential(recent_refresh_cred);
    assert!(!auth.needs_refresh()); // Should NOT need refresh

    // OAuth credential without expires_at and no last_refresh
    let never_refreshed_cred = ProviderCredential::OAuth {
        access_token: "token".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: None,
        last_refresh: None, // Never refreshed
        account_id: None,
        exchanged_api_key: None,
        extra: None,
    };
    let auth = AnthropicAuth::from_credential(never_refreshed_cred);
    assert!(auth.needs_refresh()); // Should need refresh

    // API key credential - never needs refresh
    let api_cred = ProviderCredential::Api {
        key: "sk-ant-xxx".to_string(),
    };
    let auth = AnthropicAuth::from_credential(api_cred);
    assert!(!auth.needs_refresh());
}

#[test]
fn test_legacy_auth_json_migration() {
    let codex_home = tempdir().unwrap();

    // Write legacy format auth.json
    let legacy_auth = serde_json::json!({
        "OPENAI_API_KEY": "sk-legacy-key",
        "tokens": null,
        "last_refresh": null
    });
    std::fs::write(
        codex_home.path().join("auth.json"),
        serde_json::to_string_pretty(&legacy_auth).unwrap(),
    )
    .unwrap();

    // Load with AuthManager - should auto-migrate
    let manager = AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    );

    let openai_provider = ModelProviderInfo::create_openai_provider();
    let openai_auth = manager
        .get_provider_auth("openai", &openai_provider)
        .unwrap();
    assert_eq!(openai_auth.get_token(), Some("sk-legacy-key".to_string()));
}

#[test]
fn test_new_format_api_key_works() {
    let codex_home = tempdir().unwrap();

    // Write new format auth.json with credentials map
    let new_auth = serde_json::json!({
        "credentials": {
            "openai": {
                "type": "api",
                "key": "sk-new-format-key"
            }
        }
    });
    std::fs::write(
        codex_home.path().join("auth.json"),
        serde_json::to_string_pretty(&new_auth).unwrap(),
    )
    .unwrap();

    // Load with AuthManager
    let manager = AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    );

    let openai_provider = ModelProviderInfo::create_openai_provider();
    let openai_auth = manager
        .get_provider_auth("openai", &openai_provider)
        .unwrap();

    assert_eq!(
        openai_auth.get_token(),
        Some("sk-new-format-key".to_string())
    );
    assert_eq!(openai_auth.auth_scheme(), codex_api::AuthScheme::Bearer);
}

#[test]
fn test_provider_logout() {
    let codex_home = tempdir().unwrap();
    let manager = AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    );

    // Login both providers
    manager
        .login_provider_with_api_key("openai", "sk-openai-key")
        .unwrap();
    manager
        .login_provider_with_api_key("anthropic", "sk-ant-key")
        .unwrap();

    // Logout Anthropic only
    manager.logout_provider("anthropic").unwrap();

    // OpenAI should still work
    let openai_provider = ModelProviderInfo::create_openai_provider();
    let openai_auth = manager
        .get_provider_auth("openai", &openai_provider)
        .unwrap();
    assert_eq!(openai_auth.get_token(), Some("sk-openai-key".to_string()));

    // Anthropic should be gone
    let anthropic_provider = create_anthropic_provider();
    assert!(manager
        .get_provider_auth("anthropic", &anthropic_provider)
        .is_none());
}

#[test]
fn test_provider_credential_caching() {
    let codex_home = tempdir().unwrap();
    let manager = AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    );

    manager
        .login_provider_with_api_key("openai", "sk-openai-key")
        .unwrap();

    let openai_provider = ModelProviderInfo::create_openai_provider();

    // Get auth twice - should return cached value
    let auth1 = manager
        .get_provider_auth("openai", &openai_provider)
        .unwrap();
    let auth2 = manager
        .get_provider_auth("openai", &openai_provider)
        .unwrap();

    // Both should return the same token
    assert_eq!(auth1.get_token(), auth2.get_token());
}

#[test]
fn test_multi_account_same_provider_type() {
    let codex_home = tempdir().unwrap();
    let manager = AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    );

    // Store credentials under different config keys for same provider type
    manager
        .login_provider_with_api_key("anthropic-personal", "sk-ant-personal")
        .unwrap();
    manager
        .login_provider_with_api_key("anthropic-work", "sk-ant-work")
        .unwrap();

    let personal_provider = create_anthropic_provider_with_config_key("anthropic-personal");
    let work_provider = create_anthropic_provider_with_config_key("anthropic-work");

    let personal_auth = manager
        .get_provider_auth("anthropic-personal", &personal_provider)
        .unwrap();
    let work_auth = manager
        .get_provider_auth("anthropic-work", &work_provider)
        .unwrap();

    assert_eq!(
        personal_auth.get_token(),
        Some("sk-ant-personal".to_string())
    );
    assert_eq!(work_auth.get_token(), Some("sk-ant-work".to_string()));
}

#[test]
fn test_no_auth_provider_returns_none() {
    let codex_home = tempdir().unwrap();
    let manager = AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    );

    // Create an Ollama provider (no auth needed)
    let ollama_provider = create_ollama_provider();

    // Should return None since no credentials and no env var
    let auth = manager.get_provider_auth("ollama", &ollama_provider);
    assert!(auth.is_none());
}

fn create_anthropic_provider() -> ModelProviderInfo {
    create_anthropic_provider_with_config_key("anthropic")
}

fn create_anthropic_provider_with_config_key(config_key: &str) -> ModelProviderInfo {
    ModelProviderInfo {
        name: "Anthropic".into(),
        config_key: config_key.into(),
        base_url: Some("https://api.anthropic.com/v1".into()),
        env_key: Some("ANTHROPIC_API_KEY".into()),
        env_key_instructions: None,
        experimental_bearer_token: None,
        wire_api: WireApi::Anthropic,
        query_params: None,
        http_headers: None,
        env_http_headers: None,
        request_max_retries: None,
        stream_max_retries: None,
        stream_idle_timeout_ms: None,
        requires_openai_auth: false,
    }
}

fn create_ollama_provider() -> ModelProviderInfo {
    ModelProviderInfo {
        name: "Ollama".into(),
        config_key: "ollama".into(),
        base_url: Some("http://localhost:11434/v1".into()),
        env_key: None, // No auth needed
        env_key_instructions: None,
        experimental_bearer_token: None,
        wire_api: WireApi::Chat,
        query_params: None,
        http_headers: None,
        env_http_headers: None,
        request_max_retries: None,
        stream_max_retries: None,
        stream_idle_timeout_ms: None,
        requires_openai_auth: false,
    }
}
