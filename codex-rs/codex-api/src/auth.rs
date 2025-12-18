use codex_client::Request;
use http::HeaderMap;

/// Authentication scheme used by a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AuthScheme {
    /// Use `Authorization: Bearer {token}` header (default for OpenAI-compatible APIs)
    #[default]
    Bearer,
    /// Use `x-api-key: {token}` header (for Anthropic)
    ApiKey,
}

/// Provides bearer and account identity information for API requests.
///
/// Implementations should be cheap and non-blocking; any asynchronous
/// refresh or I/O should be handled by higher layers before requests
/// reach this interface.
pub trait AuthProvider: Send + Sync {
    fn bearer_token(&self) -> Option<String>;
    fn account_id(&self) -> Option<String> {
        None
    }
    /// Returns the authentication scheme to use. Defaults to Bearer.
    fn auth_scheme(&self) -> AuthScheme {
        AuthScheme::Bearer
    }
    /// Additional headers to add to requests (e.g., anthropic-beta for OAuth).
    /// Returns empty HeaderMap if no extra headers needed.
    fn extra_headers(&self) -> HeaderMap {
        HeaderMap::new()
    }
}

pub(crate) fn add_auth_headers<A: AuthProvider>(auth: &A, mut req: Request) -> Request {
    if let Some(token) = auth.bearer_token() {
        match auth.auth_scheme() {
            AuthScheme::Bearer => {
                if let Ok(header) = format!("Bearer {token}").parse() {
                    let _ = req.headers.insert(http::header::AUTHORIZATION, header);
                }
            }
            AuthScheme::ApiKey => {
                if let Ok(header) = token.parse() {
                    let _ = req.headers.insert("x-api-key", header);
                }
            }
        }
    }
    if let Some(account_id) = auth.account_id()
        && let Ok(header) = account_id.parse()
    {
        let _ = req.headers.insert("ChatGPT-Account-ID", header);
    }
    // Extend with any provider-specific headers (e.g., anthropic-beta for OAuth)
    req.headers.extend(auth.extra_headers());
    req
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;
    use http::Method;

    struct TestAuth {
        token: Option<String>,
        account_id: Option<String>,
        scheme: AuthScheme,
        extra_headers: HeaderMap,
    }

    impl AuthProvider for TestAuth {
        fn bearer_token(&self) -> Option<String> {
            self.token.clone()
        }
        fn account_id(&self) -> Option<String> {
            self.account_id.clone()
        }
        fn auth_scheme(&self) -> AuthScheme {
            self.scheme
        }
        fn extra_headers(&self) -> HeaderMap {
            self.extra_headers.clone()
        }
    }

    fn make_request() -> Request {
        Request {
            method: Method::POST,
            url: "https://example.com/v1/test".to_string(),
            headers: HeaderMap::new(),
            body: None,
            timeout: None,
        }
    }

    #[test]
    fn bearer_auth_adds_authorization_header() {
        let auth = TestAuth {
            token: Some("test-token".to_string()),
            account_id: None,
            scheme: AuthScheme::Bearer,
            extra_headers: HeaderMap::new(),
        };
        let req = add_auth_headers(&auth, make_request());

        let auth_header = req.headers.get(http::header::AUTHORIZATION).unwrap();
        assert_eq!(auth_header, "Bearer test-token");
        assert!(req.headers.get("x-api-key").is_none());
    }

    #[test]
    fn api_key_auth_adds_x_api_key_header() {
        let auth = TestAuth {
            token: Some("sk-ant-test-key".to_string()),
            account_id: None,
            scheme: AuthScheme::ApiKey,
            extra_headers: HeaderMap::new(),
        };
        let req = add_auth_headers(&auth, make_request());

        let api_key_header = req.headers.get("x-api-key").unwrap();
        assert_eq!(api_key_header, "sk-ant-test-key");
        assert!(req.headers.get(http::header::AUTHORIZATION).is_none());
    }

    #[test]
    fn no_token_adds_no_auth_headers() {
        let auth = TestAuth {
            token: None,
            account_id: None,
            scheme: AuthScheme::Bearer,
            extra_headers: HeaderMap::new(),
        };
        let req = add_auth_headers(&auth, make_request());

        assert!(req.headers.get(http::header::AUTHORIZATION).is_none());
        assert!(req.headers.get("x-api-key").is_none());
    }

    #[test]
    fn account_id_adds_chatgpt_header() {
        let auth = TestAuth {
            token: Some("test-token".to_string()),
            account_id: Some("acct-123".to_string()),
            scheme: AuthScheme::Bearer,
            extra_headers: HeaderMap::new(),
        };
        let req = add_auth_headers(&auth, make_request());

        let account_header = req.headers.get("ChatGPT-Account-ID").unwrap();
        assert_eq!(account_header, "acct-123");
    }

    #[test]
    fn extra_headers_are_added_to_request() {
        let mut extra = HeaderMap::new();
        extra.insert("anthropic-beta", "oauth-2025-04-20".parse().unwrap());
        let auth = TestAuth {
            token: Some("access-token".to_string()),
            account_id: None,
            scheme: AuthScheme::Bearer,
            extra_headers: extra,
        };
        let req = add_auth_headers(&auth, make_request());

        let beta_header = req.headers.get("anthropic-beta").unwrap();
        assert_eq!(beta_header, "oauth-2025-04-20");
        // Also verify auth header is present
        let auth_header = req.headers.get(http::header::AUTHORIZATION).unwrap();
        assert_eq!(auth_header, "Bearer access-token");
    }
}
