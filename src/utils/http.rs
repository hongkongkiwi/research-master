//! HTTP client utilities with rate limiting support.

use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use reqwest::{header, Client, StatusCode};
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::models::{DownloadRequest, DownloadResult};
use crate::sources::SourceError;

/// Default rate limit: requests per second
const DEFAULT_REQUESTS_PER_SECOND: u32 = 5;

/// Environment variable for rate limiting (requests per second)
const RATE_LIMIT_ENV_VAR: &str = "RESEARCH_MASTER_RATE_LIMITS_DEFAULT_REQUESTS_PER_SECOND";

/// Environment variable for HTTP proxy
const HTTP_PROXY_ENV_VAR: &str = "HTTP_PROXY";

/// Environment variable for HTTPS proxy
const HTTPS_PROXY_ENV_VAR: &str = "HTTPS_PROXY";

/// Environment variable for no proxy (comma-separated list of hosts to bypass proxy)
const NO_PROXY_ENV_VAR: &str = "NO_PROXY";

/// Proxy configuration
#[derive(Debug, Clone, Default)]
pub struct ProxyConfig {
    pub http_proxy: Option<String>,
    pub https_proxy: Option<String>,
    pub no_proxy: Option<Vec<String>>,
}

impl ProxyConfig {
    /// Merge CLI-provided proxy settings with this config
    /// CLI settings take precedence over existing config values
    pub fn with_cli_args(
        mut self,
        http_proxy: Option<String>,
        https_proxy: Option<String>,
        no_proxy: Option<String>,
    ) -> Self {
        if http_proxy.is_some() {
            self.http_proxy = http_proxy;
        }
        if https_proxy.is_some() {
            self.https_proxy = https_proxy;
        }
        if let Some(no_proxy_str) = no_proxy {
            self.no_proxy = Some(
                no_proxy_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect(),
            );
        }
        self
    }
}

/// Create proxy configuration from environment variables
pub fn create_proxy_config() -> ProxyConfig {
    let http_proxy = std::env::var(HTTP_PROXY_ENV_VAR).ok();
    let https_proxy = std::env::var(HTTPS_PROXY_ENV_VAR).ok();
    let no_proxy: Option<Vec<String>> = std::env::var(NO_PROXY_ENV_VAR)
        .ok()
        .map(|s| s.split(',').map(|v| v.trim().to_string()).collect());

    if http_proxy.is_some() || https_proxy.is_some() {
        tracing::info!(
            "Proxy configured: HTTP={:?}, HTTPS={:?}, NO_PROXY={:?}",
            http_proxy,
            https_proxy,
            no_proxy
        );
    }

    ProxyConfig {
        http_proxy,
        https_proxy,
        no_proxy,
    }
}

/// Create proxy configuration from CLI arguments
/// CLI args take precedence over environment variables
pub fn create_proxy_config_from_cli(
    http_proxy: Option<String>,
    https_proxy: Option<String>,
    no_proxy: Option<String>,
) -> ProxyConfig {
    let env_config = create_proxy_config();
    env_config.with_cli_args(http_proxy, https_proxy, no_proxy)
}

/// Apply CLI proxy arguments to environment variables
/// This allows sources to pick up the proxy settings via their normal env var reading
pub fn apply_cli_proxy_args(
    http_proxy: Option<String>,
    https_proxy: Option<String>,
    no_proxy: Option<String>,
) {
    if let Some(http) = http_proxy {
        std::env::set_var(HTTP_PROXY_ENV_VAR, http);
    }
    if let Some(https) = https_proxy {
        std::env::set_var(HTTPS_PROXY_ENV_VAR, https);
    }
    if let Some(no_proxy_val) = no_proxy {
        std::env::set_var(NO_PROXY_ENV_VAR, no_proxy_val);
    }
}

/// Check if a URL should bypass the proxy
fn should_bypass_proxy(url: &str, no_proxy: &Option<Vec<String>>) -> bool {
    let Some(hosts) = no_proxy else {
        return false;
    };

    if hosts.iter().any(|h| h == "*") {
        return true;
    }

    // Parse URL to extract host
    if let Ok(url) = reqwest::Url::parse(url) {
        let host = url.host_str().map(|h| h.to_lowercase());
        if let Some(host) = host {
            // Check exact match or domain suffix match
            for no_proxy_host in hosts {
                if host == no_proxy_host.to_lowercase() {
                    return true;
                }
                // Check if the host ends with the no_proxy domain
                if host.ends_with(&format!(".{}", no_proxy_host.to_lowercase())) {
                    return true;
                }
            }
        }
    }

    false
}

/// Shared HTTP client with sensible defaults and rate limiting
#[derive(Debug, Clone)]
pub struct HttpClient {
    client: Arc<Client>,
    rate_limiter: Option<Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>>,
    no_proxy: Option<Vec<String>>,
}

/// Rate-limited request builder - compatible API with reqwest::RequestBuilder
pub struct RateLimitedRequestBuilder {
    inner: reqwest::RequestBuilder,
    rate_limiter: Option<Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>>,
}

impl RateLimitedRequestBuilder {
    /// Send the request (with rate limiting applied first)
    pub async fn send(self) -> Result<reqwest::Response, reqwest::Error> {
        if let Some(ref limiter) = self.rate_limiter {
            limiter.until_ready().await;
        }
        self.inner.send().await
    }

    /// Add a header (accepts &str for convenience - most common use case)
    pub fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.inner = self.inner.header(key.as_ref(), value.as_ref());
        self
    }

    /// Set headers
    pub fn headers(mut self, headers: header::HeaderMap) -> Self {
        self.inner = self.inner.headers(headers);
        self
    }

    /// Basic auth
    pub fn basic_auth<U, P>(self, username: U, password: Option<P>) -> Self
    where
        U: Into<String> + std::fmt::Display,
        P: Into<String> + std::fmt::Display,
    {
        Self {
            inner: self.inner.basic_auth(username, password),
            rate_limiter: self.rate_limiter,
        }
    }

    /// Bearer auth
    pub fn bearer_auth<T>(self, token: T) -> Self
    where
        T: Into<String> + std::fmt::Display,
    {
        Self {
            inner: self.inner.bearer_auth(token),
            rate_limiter: self.rate_limiter,
        }
    }

    /// Query parameters
    pub fn query<T: serde::Serialize + ?Sized>(mut self, query: &T) -> Self {
        self.inner = self.inner.query(query);
        self
    }

    /// Form data
    pub fn form<T: serde::Serialize + ?Sized>(mut self, form: &T) -> Self {
        self.inner = self.inner.form(form);
        self
    }

    /// JSON body
    pub fn json<T: serde::Serialize + ?Sized>(mut self, json: &T) -> Self {
        self.inner = self.inner.json(json);
        self
    }

    /// Build the request
    pub fn build(self) -> Result<reqwest::Request, reqwest::Error> {
        self.inner.build()
    }
}

/// Environment variable for custom user agent
pub const USER_AGENT_ENV_VAR: &str = "RESEARCH_MASTER_USER_AGENT";

/// Get user agent from environment or use default
pub fn get_user_agent() -> String {
    std::env::var(USER_AGENT_ENV_VAR)
        .unwrap_or_else(|_| format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")))
}

impl HttpClient {
    /// Create a new HTTP client with default settings and rate limiting
    pub fn new() -> Result<Self, SourceError> {
        Self::with_user_agent(&get_user_agent())
    }

    /// Create a new HTTP client with a custom user agent
    pub fn with_user_agent(user_agent: &str) -> Result<Self, SourceError> {
        let rate_limiter = Self::create_rate_limiter();
        let proxy = create_proxy_config();

        let mut builder = Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90));

        // Apply proxy if configured
        if let Some(proxy_url) = proxy.http_proxy {
            builder = builder.proxy(reqwest::Proxy::http(&proxy_url)?);
        }
        if let Some(proxy_url) = proxy.https_proxy {
            builder = builder.proxy(reqwest::Proxy::https(&proxy_url)?);
        }

        let client = builder
            .build()
            .map_err(|e| SourceError::Network(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client: Arc::new(client),
            rate_limiter,
            no_proxy: proxy.no_proxy,
        })
    }

    /// Create HTTP client with custom timeout
    pub fn with_timeout(user_agent: &str, timeout_secs: u64) -> Result<Self, SourceError> {
        let rate_limiter = Self::create_rate_limiter();
        let proxy = create_proxy_config();

        let mut builder = Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90));

        // Apply proxy if configured
        if let Some(proxy_url) = proxy.http_proxy {
            builder = builder.proxy(reqwest::Proxy::http(&proxy_url)?);
        }
        if let Some(proxy_url) = proxy.https_proxy {
            builder = builder.proxy(reqwest::Proxy::https(&proxy_url)?);
        }

        let client = builder
            .build()
            .map_err(|e| SourceError::Network(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client: Arc::new(client),
            rate_limiter,
            no_proxy: proxy.no_proxy,
        })
    }

    /// Create a new HTTP client without rate limiting
    pub fn without_rate_limit(user_agent: &str) -> Result<Self, SourceError> {
        let proxy = create_proxy_config();
        let mut builder = Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90));

        if let Some(proxy_url) = proxy.http_proxy {
            builder = builder.proxy(reqwest::Proxy::http(&proxy_url)?);
        }
        if let Some(proxy_url) = proxy.https_proxy {
            builder = builder.proxy(reqwest::Proxy::https(&proxy_url)?);
        }

        let client = builder
            .build()
            .map_err(|e| SourceError::Network(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client: Arc::new(client),
            rate_limiter: None,
            no_proxy: proxy.no_proxy,
        })
    }

    /// Check if a URL should bypass the proxy
    pub fn should_bypass_proxy(&self, url: &str) -> bool {
        should_bypass_proxy(url, &self.no_proxy)
    }

    /// Create a new HTTP client with a custom rate limit
    pub fn with_rate_limit(
        user_agent: &str,
        requests_per_second: u32,
    ) -> Result<Self, SourceError> {
        let rate_limiter = if requests_per_second == 0 {
            None
        } else {
            let nonzero = NonZeroU32::new(requests_per_second)
                .expect("requests_per_second should be > 0 when not 0 branch");
            let quota = Quota::per_second(nonzero);
            Some(Arc::new(RateLimiter::direct(quota)))
        };

        let proxy = create_proxy_config();
        let mut builder = Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90));

        if let Some(proxy_url) = proxy.http_proxy {
            builder = builder.proxy(reqwest::Proxy::http(&proxy_url)?);
        }
        if let Some(proxy_url) = proxy.https_proxy {
            builder = builder.proxy(reqwest::Proxy::https(&proxy_url)?);
        }

        let client = builder
            .build()
            .map_err(|e| SourceError::Network(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client: Arc::new(client),
            rate_limiter,
            no_proxy: proxy.no_proxy,
        })
    }

    /// Create HTTP client with per-source proxy
    pub fn with_proxy(
        user_agent: &str,
        http_proxy: Option<String>,
        https_proxy: Option<String>,
        requests_per_second: u32,
    ) -> Result<Self, SourceError> {
        let rate_limiter = if requests_per_second == 0 {
            None
        } else {
            let nonzero = NonZeroU32::new(requests_per_second)
                .expect("requests_per_second should be > 0 when not 0 branch");
            let quota = Quota::per_second(nonzero);
            Some(Arc::new(RateLimiter::direct(quota)))
        };

        let mut builder = Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90));

        if let Some(proxy_url) = http_proxy {
            builder = builder.proxy(reqwest::Proxy::http(&proxy_url)?);
        }
        if let Some(proxy_url) = https_proxy {
            builder = builder.proxy(reqwest::Proxy::https(&proxy_url)?);
        }

        let client = builder
            .build()
            .map_err(|e| SourceError::Network(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client: Arc::new(client),
            rate_limiter,
            no_proxy: None, // Per-source proxy doesn't use env no_proxy
        })
    }

    /// Create rate limiter from environment variable or default
    fn create_rate_limiter() -> Option<Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>> {
        let requests_per_second = std::env::var(RATE_LIMIT_ENV_VAR)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(DEFAULT_REQUESTS_PER_SECOND);

        if requests_per_second == 0 {
            // Rate limiting disabled
            tracing::info!("Rate limiting disabled");
            return None;
        }

        let nonzero =
            NonZeroU32::new(requests_per_second).expect("requests_per_second should not be zero");
        let quota = Quota::per_second(nonzero);
        let limiter = RateLimiter::direct(quota);

        tracing::info!(
            "Rate limiting enabled: {} requests per second",
            requests_per_second
        );

        Some(Arc::new(limiter))
    }

    /// Create from an existing reqwest Client
    pub fn from_client(client: Arc<Client>) -> Self {
        Self {
            client,
            rate_limiter: Self::create_rate_limiter(),
            no_proxy: None,
        }
    }

    /// Get the underlying client
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Create a rate-limited GET request builder
    pub fn get(&self, url: &str) -> RateLimitedRequestBuilder {
        RateLimitedRequestBuilder {
            inner: self.client.get(url),
            rate_limiter: self.rate_limiter.clone(),
        }
    }

    /// Create a rate-limited POST request builder
    pub fn post(&self, url: &str) -> RateLimitedRequestBuilder {
        RateLimitedRequestBuilder {
            inner: self.client.post(url),
            rate_limiter: self.rate_limiter.clone(),
        }
    }

    /// Download a file from a URL to the specified path
    pub async fn download_to_file(
        &self,
        url: &str,
        request: &DownloadRequest,
        filename: &str,
    ) -> Result<DownloadResult, SourceError> {
        if let Some(ref limiter) = self.rate_limiter {
            limiter.until_ready().await;
        }

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to download: {}", e)))?;

        if !response.status().is_success() {
            return Err(SourceError::NotFound(format!(
                "Failed to download: HTTP {}",
                response.status()
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to read response: {}", e)))?;

        // Create download directory if it doesn't exist
        std::fs::create_dir_all(&request.save_path).map_err(|e| {
            SourceError::Io(std::io::Error::other(format!(
                "Failed to create directory: {}",
                e
            )))
        })?;

        let path = Path::new(&request.save_path).join(filename);

        std::fs::write(&path, bytes.as_ref()).map_err(SourceError::Io)?;

        Ok(DownloadResult::success(
            path.to_string_lossy().to_string(),
            bytes.len() as u64,
        ))
    }

    /// Download a PDF with a sanitized filename
    pub async fn download_pdf(
        &self,
        url: &str,
        request: &DownloadRequest,
        paper_id: &str,
    ) -> Result<DownloadResult, SourceError> {
        let filename = format!("{}.pdf", paper_id.replace('/', "_"));
        self.download_to_file(url, request, &filename).await
    }

    /// Check if a URL returns success status
    pub async fn head(&self, url: &str) -> Result<bool, SourceError> {
        if let Some(ref limiter) = self.rate_limiter {
            limiter.until_ready().await;
        }

        let response = self
            .client
            .head(url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Head request failed: {}", e)))?;
        Ok(response.status() == StatusCode::OK)
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default HTTP client")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_rate_limit_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let previous = std::env::var(RATE_LIMIT_ENV_VAR).ok();

        match value {
            Some(v) => std::env::set_var(RATE_LIMIT_ENV_VAR, v),
            None => std::env::remove_var(RATE_LIMIT_ENV_VAR),
        }

        let result = f();

        match previous {
            Some(v) => std::env::set_var(RATE_LIMIT_ENV_VAR, v),
            _ => std::env::remove_var(RATE_LIMIT_ENV_VAR),
        }

        result
    }

    #[test]
    fn test_create_rate_limiter_with_default() {
        with_rate_limit_env(None, || {
            let limiter = HttpClient::create_rate_limiter();
            assert!(limiter.is_some(), "Default rate limiter should be created");
        });
    }

    #[test]
    fn test_create_rate_limiter_disabled() {
        with_rate_limit_env(Some("0"), || {
            let limiter = HttpClient::create_rate_limiter();
            assert!(
                limiter.is_none(),
                "Rate limiter should be disabled when set to 0"
            );
        });
    }

    #[test]
    fn test_create_rate_limiter_custom() {
        with_rate_limit_env(Some("10"), || {
            let limiter = HttpClient::create_rate_limiter();
            assert!(limiter.is_some(), "Custom rate limiter should be created");
        });
    }

    #[test]
    fn test_create_rate_limiter_invalid() {
        with_rate_limit_env(Some("invalid"), || {
            let limiter = HttpClient::create_rate_limiter();
            // Should fall back to default when invalid value is provided
            assert!(
                limiter.is_some(),
                "Should fall back to default rate limiter"
            );
        });
    }

    #[test]
    fn test_should_bypass_proxy_no_config() {
        // No no_proxy configured
        let result = should_bypass_proxy("https://api.semanticscholar.org", &None);
        assert!(!result, "Should not bypass when no no_proxy configured");
    }

    #[test]
    fn test_should_bypass_proxy_wildcard() {
        let no_proxy = Some(vec!["*".to_string()]);
        let result = should_bypass_proxy("https://api.semanticscholar.org", &no_proxy);
        assert!(result, "Should bypass for wildcard");
    }

    #[test]
    fn test_should_bypass_proxy_exact_match() {
        let no_proxy = Some(vec!["api.semanticscholar.org".to_string()]);
        let result = should_bypass_proxy("https://api.semanticscholar.org", &no_proxy);
        assert!(result, "Should bypass for exact match");
    }

    #[test]
    fn test_should_bypass_proxy_domain_suffix() {
        let no_proxy = Some(vec!["semanticscholar.org".to_string()]);
        let result = should_bypass_proxy("https://api.semanticscholar.org", &no_proxy);
        assert!(result, "Should bypass for domain suffix match");
    }

    #[test]
    fn test_should_bypass_proxy_no_match() {
        let no_proxy = Some(vec!["other-domain.org".to_string()]);
        let result = should_bypass_proxy("https://api.semanticscholar.org", &no_proxy);
        assert!(!result, "Should not bypass when domain doesn't match");
    }

    #[test]
    fn test_should_bypass_proxy_multiple_hosts() {
        let no_proxy = Some(vec![
            "api.semanticscholar.org".to_string(),
            "arxiv.org".to_string(),
        ]);
        assert!(should_bypass_proxy(
            "https://api.semanticscholar.org",
            &no_proxy
        ));
        assert!(should_bypass_proxy("https://arxiv.org", &no_proxy));
        assert!(!should_bypass_proxy("https://openalex.org", &no_proxy));
    }
}
