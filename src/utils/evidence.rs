//! Evidence utilities: Scite smart-citation tallies and retraction status.
//!
//! Endpoint shapes verified live 2026-11-02:
//!
//! - Scite tallies, `GET https://api.scite.ai/tallies/{doi}` (raw DOI, path
//!   slashes unencoded):
//!
//! ```text
//! 200 {"total":1937,"supporting":22,"contradicting":1,"mentioning":1905,
//!      "unclassified":9,"doi":"10.1038/nature12373","citingPublications":2163}
//! 404 {"detail":"Tally not found"}
//! ```
//!
//! - OpenAlex single-work lookup (primary retraction source),
//!   `GET https://api.openalex.org/works/doi:{doi}` (DOI percent-encoded,
//!   `%2F` verified):
//!
//! ```text
//! 200 work object with "is_retracted", "retraction_date", "title", "id";
//!    retraction_date may be null even when is_retracted is true
//! 404 HTML body - only the status code is meaningful
//! ```

use serde::{Deserialize, Serialize};

use crate::sources::SourceError;
use crate::utils::{api_retry_config, with_retry, HttpClient, RetryConfig};

/// Scite tallies base URL; the normalized DOI is appended unencoded.
pub const SCITE_TALLIES_URL: &str = "https://api.scite.ai/tallies/";

/// OpenAlex single-work lookup base URL; the percent-encoded DOI is appended.
pub const OPENALEX_WORKS_URL: &str = "https://api.openalex.org/works/doi:";

/// Scite smart-citation tallies for one DOI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SciteTallies {
    pub doi: String,
    pub total: u64,
    pub supporting: u64,
    pub contradicting: u64,
    pub mentioning: u64,
    pub unclassified: u64,
    #[serde(default, rename = "citingPublications")]
    pub citing_publications: Option<u64>,
}

/// Retraction status for one DOI from the OpenAlex single-work lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetractionInfo {
    pub doi: String,
    pub is_retracted: bool,
    #[serde(default, skip_serializing)]
    pub is_known: bool,
    pub retraction_date: Option<String>,
    pub openalex_id: Option<String>,
    pub title: Option<String>,
}

/// Normalize a DOI for lookup: trim, lowercase, strip resolver and `doi:`
/// prefixes. Returns `None` when nothing remains.
pub fn normalize_doi(doi: &str) -> Option<String> {
    let mut s = doi.trim().to_lowercase();
    for prefix in [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "doi:",
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
        }
    }
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn normalize_or_invalid(doi: &str) -> Result<String, SourceError> {
    normalize_doi(doi).ok_or_else(|| SourceError::InvalidRequest(format!("Invalid DOI: '{}'", doi)))
}

/// Fetch Scite smart-citation tallies (supporting/contradicting/mentioning)
/// for a DOI. Unknown DOIs yield [`SourceError::NotFound`].
pub async fn scite_tallies(client: &HttpClient, doi: &str) -> Result<SciteTallies, SourceError> {
    scite_tallies_from(SCITE_TALLIES_URL, client, doi).await
}

/// [`scite_tallies`] against a custom base URL (test seam for mockito).
pub(crate) async fn scite_tallies_from(
    base_url: &str,
    client: &HttpClient,
    doi: &str,
) -> Result<SciteTallies, SourceError> {
    fetch_scite_tallies(base_url, client, doi, api_retry_config()).await
}

async fn fetch_scite_tallies(
    base_url: &str,
    client: &HttpClient,
    doi: &str,
    retry: RetryConfig,
) -> Result<SciteTallies, SourceError> {
    let doi = normalize_or_invalid(doi)?;
    let url = format!("{}{}", base_url, doi);

    with_retry(retry, || {
        let url = url.clone();
        let doi = doi.clone();
        async move {
            let response = client.get(&url).send().await.map_err(|e| {
                SourceError::Network(format!("Scite tallies request failed: {}", e))
            })?;

            let status = response.status();
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(SourceError::NotFound(format!(
                    "No Scite tallies for DOI {}",
                    doi
                )));
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(SourceError::RateLimit);
            }
            if !status.is_success() {
                return Err(map_http_status("Scite tallies API", status));
            }

            response
                .json::<SciteTallies>()
                .await
                .map_err(|e| SourceError::Parse(format!("Failed to parse Scite tallies: {}", e)))
        }
    })
    .await
}

/// Classify non-success, non-404, non-429 statuses: 5xx is transient
/// (Network, so `with_retry` retries it), other 4xx is permanent (Api).
fn map_http_status(api: &str, status: reqwest::StatusCode) -> SourceError {
    if status.is_server_error() {
        SourceError::Network(format!("{} returned {}", api, status))
    } else {
        SourceError::Api(format!("{} returned {}", api, status))
    }
}

/// Check whether a DOI is retracted via the OpenAlex single-work lookup
/// (the primary retraction source). `retraction_date` may be null even when
/// `is_retracted` is true. Unknown DOIs yield [`SourceError::NotFound`].
pub async fn check_retraction(
    client: &HttpClient,
    doi: &str,
) -> Result<RetractionInfo, SourceError> {
    check_retraction_from(OPENALEX_WORKS_URL, client, doi).await
}

/// [`check_retraction`] against a custom base URL (test seam for mockito).
pub(crate) async fn check_retraction_from(
    base_url: &str,
    client: &HttpClient,
    doi: &str,
) -> Result<RetractionInfo, SourceError> {
    fetch_retraction(base_url, client, doi, api_retry_config()).await
}

async fn fetch_retraction(
    base_url: &str,
    client: &HttpClient,
    doi: &str,
    retry: RetryConfig,
) -> Result<RetractionInfo, SourceError> {
    let doi = normalize_or_invalid(doi)?;
    let url = format!("{}{}", base_url, urlencoding::encode(&doi));

    with_retry(retry, || {
        let url = url.clone();
        let doi = doi.clone();
        async move {
            let response = client.get(&url).send().await.map_err(|e| {
                SourceError::Network(format!("OpenAlex lookup request failed: {}", e))
            })?;

            let status = response.status();
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(SourceError::NotFound(format!(
                    "DOI {} not found in OpenAlex",
                    doi
                )));
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(SourceError::RateLimit);
            }
            if !status.is_success() {
                return Err(map_http_status("OpenAlex API", status));
            }

            let work = response
                .json::<OpenAlexWork>()
                .await
                .map_err(|e| SourceError::Parse(format!("Failed to parse OpenAlex work: {}", e)))?;

            Ok(work.into_retraction_info(doi))
        }
    })
    .await
}

/// Subset of the OpenAlex work object this module needs; unknown fields are
/// ignored by serde.
#[derive(Debug, Deserialize)]
struct OpenAlexWork {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "display_name")]
    display_name: Option<String>,
    #[serde(default)]
    is_retracted: Option<bool>,
    #[serde(default)]
    retraction_date: Option<String>,
}

impl OpenAlexWork {
    fn into_retraction_info(self, doi: String) -> RetractionInfo {
        RetractionInfo {
            doi,
            is_known: self.is_retracted.is_some(),
            is_retracted: self.is_retracted.unwrap_or(false),
            retraction_date: self.retraction_date,
            openalex_id: self.id,
            title: self.title.or(self.display_name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Build a client with proxy env vars cleared so requests reach the
    /// local mockito server instead of any ambient proxy.
    fn test_client() -> HttpClient {
        let guard = env_lock().lock().expect("env lock poisoned");
        const VARS: [&str; 6] = [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ];
        let previous: Vec<(&str, Option<String>)> =
            VARS.iter().map(|v| (*v, std::env::var(v).ok())).collect();
        for var in VARS {
            std::env::remove_var(var);
        }
        let client =
            HttpClient::without_rate_limit("research-master-tests").expect("test HTTP client");
        for (var, value) in previous {
            match value {
                Some(v) => std::env::set_var(var, v),
                None => std::env::remove_var(var),
            }
        }
        drop(guard);
        client
    }

    fn fast_retry() -> RetryConfig {
        RetryConfig {
            max_attempts: 2,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            backoff_multiplier: 2.0,
            max_total_time: Duration::from_secs(10),
        }
    }

    #[test]
    fn test_missing_retraction_status_is_unknown() {
        let work: OpenAlexWork = serde_json::from_str(r#"{"id":"W1"}"#).unwrap();
        let info = work.into_retraction_info("10.1/example".to_string());
        assert!(!info.is_known);
        assert!(!info.is_retracted);
    }

    #[test]
    fn test_normalize_doi_variants() {
        assert_eq!(
            normalize_doi("10.1038/Nature12373"),
            Some("10.1038/nature12373".to_string())
        );
        assert_eq!(
            normalize_doi("  https://doi.org/10.1038/nature12373 "),
            Some("10.1038/nature12373".to_string())
        );
        assert_eq!(
            normalize_doi("doi:10.1038/nature12373"),
            Some("10.1038/nature12373".to_string())
        );
        assert_eq!(normalize_doi("   "), None);
        assert_eq!(normalize_doi(""), None);
    }

    const SCITE_LIVE_BODY: &str = r#"{"total":1937,"supporting":22,"contradicting":1,"mentioning":1905,"unclassified":9,"doi":"10.1038/nature12373","citingPublications":2163}"#;

    #[tokio::test]
    async fn test_scite_tallies_parses_live_shape() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/tallies/10.1038/nature12373")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SCITE_LIVE_BODY)
            .expect(1)
            .create_async()
            .await;

        let url = format!("{}/tallies/", server.url());
        let tallies =
            fetch_scite_tallies(&url, &test_client(), "10.1038/nature12373", fast_retry())
                .await
                .expect("tallies");

        assert_eq!(tallies.doi, "10.1038/nature12373");
        assert_eq!(tallies.total, 1937);
        assert_eq!(tallies.supporting, 22);
        assert_eq!(tallies.contradicting, 1);
        assert_eq!(tallies.mentioning, 1905);
        assert_eq!(tallies.unclassified, 9);
        assert_eq!(tallies.citing_publications, Some(2163));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_scite_tallies_unknown_doi_is_permanent_not_found() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/tallies/10.9999/bogus")
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(r#"{"detail":"Tally not found"}"#)
            .expect(1)
            .create_async()
            .await;

        let url = format!("{}/tallies/", server.url());
        let err = fetch_scite_tallies(&url, &test_client(), "10.9999/bogus", fast_retry())
            .await
            .expect_err("404 must not be retried");

        assert!(matches!(err, SourceError::NotFound(_)), "got {:?}", err);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_scite_tallies_retries_server_error_then_succeeds() {
        let mut server = mockito::Server::new_async().await;
        // mockito matches mocks in creation order: first request hits the
        // 500, the retry hits the 200.
        let flaky = server
            .mock("GET", "/tallies/10.1038/nature12373")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;
        let ok = server
            .mock("GET", "/tallies/10.1038/nature12373")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SCITE_LIVE_BODY)
            .expect(1)
            .create_async()
            .await;

        let url = format!("{}/tallies/", server.url());
        let tallies =
            fetch_scite_tallies(&url, &test_client(), "10.1038/nature12373", fast_retry())
                .await
                .expect("tallies after one 500");
        assert_eq!(tallies.total, 1937);

        flaky.assert_async().await;
        ok.assert_async().await;
    }

    #[tokio::test]
    async fn test_scite_tallies_uppercase_doi_normalized() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/tallies/10.1038/nature12373")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SCITE_LIVE_BODY)
            .expect(1)
            .create_async()
            .await;

        let url = format!("{}/tallies/", server.url());
        let result =
            fetch_scite_tallies(&url, &test_client(), "10.1038/NATURE12373", fast_retry()).await;
        assert!(result.is_ok(), "got {:?}", result);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_check_retraction_not_retracted_shape() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/works/doi:10.1038%2Fnature12373")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"id":"https://openalex.org/W2159974629",
                    "doi":"https://doi.org/10.1038/nature12373",
                    "title":"Nanometre-scale thermometry in a living cell",
                    "is_retracted":false,"retraction_date":null}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let url = format!("{}/works/doi:", server.url());
        let info = fetch_retraction(&url, &test_client(), "10.1038/nature12373", fast_retry())
            .await
            .expect("retraction info");

        assert_eq!(info.doi, "10.1038/nature12373");
        assert!(!info.is_retracted);
        assert_eq!(info.retraction_date, None);
        assert_eq!(
            info.openalex_id.as_deref(),
            Some("https://openalex.org/W2159974629")
        );
        assert_eq!(
            info.title.as_deref(),
            Some("Nanometre-scale thermometry in a living cell")
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_check_retraction_retracted_shape() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/works/doi:10.1016%2Fs0140-6736%2897%2911096-0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"id":"https://openalex.org/W2117847125",
                    "doi":"https://doi.org/10.1016/s0140-6736(97)11096-0",
                    "title":"RETRACTED: Ileal-lymphoid-nodular hyperplasia, non-specific colitis, and pervasive developmental disorder in children",
                    "is_retracted":true,"retraction_date":null}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let url = format!("{}/works/doi:", server.url());
        let info = fetch_retraction(
            &url,
            &test_client(),
            "10.1016/s0140-6736(97)11096-0",
            fast_retry(),
        )
        .await
        .expect("retraction info");

        assert!(info.is_retracted);
        assert_eq!(info.retraction_date, None);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_check_retraction_unknown_doi_404_html() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/works/doi:10.9999%2Fbogus")
            .with_status(404)
            .with_body("<!doctype html><html lang=en><title>404 Not Found</title></html>")
            .expect(1)
            .create_async()
            .await;

        let url = format!("{}/works/doi:", server.url());
        let err = fetch_retraction(&url, &test_client(), "10.9999/bogus", fast_retry())
            .await
            .expect_err("404 must surface as NotFound");

        assert!(matches!(err, SourceError::NotFound(_)), "got {:?}", err);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_invalid_doi_returns_invalid_request() {
        let client = test_client();
        let err = scite_tallies_from("http://127.0.0.1:1/tallies/", &client, "   ")
            .await
            .expect_err("blank DOI must be rejected");
        assert!(
            matches!(err, SourceError::InvalidRequest(_)),
            "got {:?}",
            err
        );
    }
}
