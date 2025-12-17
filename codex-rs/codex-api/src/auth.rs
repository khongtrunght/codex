use codex_client::Request;

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
        };
        let req = add_auth_headers(&auth, make_request());

        let account_header = req.headers.get("ChatGPT-Account-ID").unwrap();
        assert_eq!(account_header, "acct-123");
    }
}
