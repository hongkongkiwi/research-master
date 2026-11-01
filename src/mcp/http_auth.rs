//! Bearer-token auth middleware for the streamable HTTP transport.
//!
//! stdio is the default transport and needs no auth. When `mcp --http` runs
//! with `RESEARCH_MASTER_HTTP_TOKEN` set, every request must present
//! `Authorization: Bearer <token>` or it is rejected before the protocol
//! layer. Without the env var the transport stays open, which is only
//! appropriate on loopback.

use http::header::AUTHORIZATION;
use pmcp::server::http_middleware::{
    ServerHttpContext, ServerHttpMiddleware, ServerHttpMiddlewareChain, ServerHttpRequest,
};
use pmcp::{Error, Result};

/// Rejects requests whose Authorization header does not carry the expected
/// bearer token.
#[derive(Debug)]
pub struct BearerTokenMiddleware {
    expected: String,
}

impl BearerTokenMiddleware {
    pub fn new(expected: impl Into<String>) -> Self {
        Self {
            expected: expected.into(),
        }
    }

    /// Build a chain with this middleware when the env token is configured.
    pub fn chain_from_env() -> Option<ServerHttpMiddlewareChain> {
        let token = std::env::var("RESEARCH_MASTER_HTTP_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty())?;
        let mut chain = ServerHttpMiddlewareChain::new();
        chain.add(std::sync::Arc::new(Self::new(token)));
        Some(chain)
    }
}

#[async_trait::async_trait]
impl ServerHttpMiddleware for BearerTokenMiddleware {
    async fn on_request(
        &self,
        request: &mut ServerHttpRequest,
        _context: &ServerHttpContext,
    ) -> Result<()> {
        let authorized = request
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value == format!("Bearer {}", self.expected)
                    || value == format!("bearer {}", self.expected)
            });
        if authorized {
            Ok(())
        } else {
            Err(Error::invalid_params(
                "missing or invalid bearer token (set Authorization: Bearer <RESEARCH_MASTER_HTTP_TOKEN>)",
            ))
        }
    }

    fn priority(&self) -> i32 {
        // Auth runs before logging (90) and other middleware.
        10
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderValue, Method};

    fn request_with_auth(value: Option<&str>) -> ServerHttpRequest {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        }
        ServerHttpRequest::new(
            Method::POST,
            "http://localhost:3000/mcp".parse().unwrap(),
            headers,
            Vec::new(),
        )
    }

    fn middleware() -> BearerTokenMiddleware {
        BearerTokenMiddleware::new("secret-token")
    }

    #[tokio::test]
    async fn accepts_matching_bearer_token() {
        let mut request = request_with_auth(Some("Bearer secret-token"));
        let context = ServerHttpContext::new("req-1".to_string());
        assert!(middleware()
            .on_request(&mut request, &context)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn rejects_missing_wrong_and_mismatched_tokens() {
        let context = ServerHttpContext::new("req-1".to_string());

        let mut request = request_with_auth(None);
        assert!(middleware()
            .on_request(&mut request, &context)
            .await
            .is_err());

        let mut request = request_with_auth(Some("Bearer wrong"));
        assert!(middleware()
            .on_request(&mut request, &context)
            .await
            .is_err());

        let mut request = request_with_auth(Some("secret-token"));
        assert!(middleware()
            .on_request(&mut request, &context)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn open_when_no_token_configured() {
        assert!(BearerTokenMiddleware::chain_from_env().is_none());
    }
}
