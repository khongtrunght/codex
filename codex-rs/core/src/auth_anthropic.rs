//! Anthropic OAuth authentication implementation using PKCE flow.
//!
//! This module implements OAuth 2.0 with PKCE (Proof Key for Code Exchange) for
//! Anthropic's Claude API, supporting both Claude Pro/Max subscription auth and
//! Console API key creation.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use sha2::Digest;
use sha2::Sha256;

use crate::token_data::AnthropicTokenData;

pub const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const ANTHROPIC_REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";

/// OAuth authentication mode for Anthropic
#[derive(Debug, Clone, Copy)]
pub enum AnthropicOAuthMode {
    /// Claude Pro/Max subscription (uses claude.ai)
    Max,
    /// Console API key creation (uses console.anthropic.com)
    Console,
}

/// PKCE code verifier and challenge pair
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    pub verifier: String,
    pub challenge: String,
}

/// Generate PKCE code verifier and challenge
pub fn generate_pkce() -> PkceChallenge {
    // Generate 32 random bytes for verifier
    let mut rng = rand::rng();
    let verifier_bytes: Vec<u8> = (0..32).map(|_| rng.random()).collect();
    let verifier = URL_SAFE_NO_PAD.encode(&verifier_bytes);

    // Create SHA256 challenge
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    PkceChallenge {
        verifier,
        challenge,
    }
}

/// Build authorization URL for OAuth flow
pub fn build_authorization_url(mode: AnthropicOAuthMode, pkce: &PkceChallenge) -> String {
    let base_url = match mode {
        AnthropicOAuthMode::Max => "https://claude.ai/oauth/authorize",
        AnthropicOAuthMode::Console => "https://console.anthropic.com/oauth/authorize",
    };

    format!(
        "{}?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        base_url,
        ANTHROPIC_CLIENT_ID,
        urlencoding::encode(ANTHROPIC_REDIRECT_URI),
        urlencoding::encode("org:create_api_key user:profile user:inference"),
        pkce.challenge,
        pkce.verifier // Using verifier as state for simplicity
    )
}

/// Exchange authorization code for tokens
pub async fn exchange_code(
    code: &str,
    _state: &str,
    verifier: &str,
    client: &reqwest::Client,
) -> Result<AnthropicTokenData, std::io::Error> {
    // Code comes with format: "{code}#{state}" from redirect
    let parts: Vec<&str> = code.split('#').collect();
    let auth_code = parts
        .first()
        .ok_or_else(|| std::io::Error::other("Invalid code format"))?;
    let code_state = parts.get(1).unwrap_or(&"");

    let response = client
        .post("https://console.anthropic.com/v1/oauth/token")
        .json(&serde_json::json!({
            "code": auth_code,
            "state": code_state,
            "grant_type": "authorization_code",
            "client_id": ANTHROPIC_CLIENT_ID,
            "redirect_uri": ANTHROPIC_REDIRECT_URI,
            "code_verifier": verifier,
        }))
        .send()
        .await
        .map_err(std::io::Error::other)?;

    if !response.status().is_success() {
        return Err(std::io::Error::other(format!(
            "Token exchange failed: {}",
            response.status()
        )));
    }

    let token_response: serde_json::Value = response.json().await.map_err(std::io::Error::other)?;

    Ok(AnthropicTokenData {
        access_token: token_response["access_token"]
            .as_str()
            .ok_or_else(|| std::io::Error::other("Missing access_token"))?
            .to_string(),
        refresh_token: token_response["refresh_token"]
            .as_str()
            .ok_or_else(|| std::io::Error::other("Missing refresh_token"))?
            .to_string(),
        expires_at: chrono::Utc::now().timestamp_millis()
            + (token_response["expires_in"].as_i64().unwrap_or(3600) * 1000),
    })
}

/// Refresh an expired access token
pub async fn refresh_token(
    refresh_token: &str,
    client: &reqwest::Client,
) -> Result<AnthropicTokenData, std::io::Error> {
    let response = client
        .post("https://console.anthropic.com/v1/oauth/token")
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": ANTHROPIC_CLIENT_ID,
        }))
        .send()
        .await
        .map_err(std::io::Error::other)?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unable to read error response".to_string());
        return Err(std::io::Error::other(format!(
            "Token refresh failed: {status} - {error_body}",
        )));
    }

    let token_response: serde_json::Value = response.json().await.map_err(std::io::Error::other)?;

    Ok(AnthropicTokenData {
        access_token: token_response["access_token"]
            .as_str()
            .ok_or_else(|| std::io::Error::other("Missing access_token"))?
            .to_string(),
        refresh_token: token_response["refresh_token"]
            .as_str()
            .ok_or_else(|| std::io::Error::other("Missing refresh_token"))?
            .to_string(),
        expires_at: chrono::Utc::now().timestamp_millis()
            + (token_response["expires_in"].as_i64().unwrap_or(3600) * 1000),
    })
}

/// Create API key using OAuth token (Console mode only)
pub async fn create_api_key(
    access_token: &str,
    client: &reqwest::Client,
) -> Result<String, std::io::Error> {
    let response = client
        .post("https://api.anthropic.com/api/oauth/claude_cli/create_api_key")
        .header("Authorization", format!("Bearer {access_token}"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(std::io::Error::other)?;

    if !response.status().is_success() {
        return Err(std::io::Error::other(format!(
            "API key creation failed: {}",
            response.status()
        )));
    }

    let result: serde_json::Value = response.json().await.map_err(std::io::Error::other)?;

    result["raw_key"]
        .as_str()
        .ok_or_else(|| std::io::Error::other("Missing raw_key in response"))
        .map(std::string::ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_pkce() {
        let pkce = generate_pkce();
        assert!(!pkce.verifier.is_empty());
        assert!(!pkce.challenge.is_empty());
        assert_ne!(pkce.verifier, pkce.challenge);

        // Verify challenge is base64 URL-safe encoded SHA256 of verifier
        let mut hasher = Sha256::new();
        hasher.update(pkce.verifier.as_bytes());
        let expected_challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(pkce.challenge, expected_challenge);
    }

    #[test]
    fn test_build_authorization_url_max() {
        let pkce = PkceChallenge {
            verifier: "test_verifier".to_string(),
            challenge: "test_challenge".to_string(),
        };

        let url = build_authorization_url(AnthropicOAuthMode::Max, &pkce);

        assert!(url.starts_with("https://claude.ai/oauth/authorize"));
        assert!(url.contains("code=true"));
        assert!(url.contains(&format!("client_id={ANTHROPIC_CLIENT_ID}")));
        assert!(url.contains("code_challenge=test_challenge"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn test_build_authorization_url_console() {
        let pkce = PkceChallenge {
            verifier: "test_verifier".to_string(),
            challenge: "test_challenge".to_string(),
        };

        let url = build_authorization_url(AnthropicOAuthMode::Console, &pkce);

        assert!(url.starts_with("https://console.anthropic.com/oauth/authorize"));
        assert!(url.contains("scope=org%3Acreate_api_key"));
    }
}
