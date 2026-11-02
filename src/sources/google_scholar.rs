//! Google Scholar research source implementation.
//!
//! NOTE: This source is DISABLED by default and requires the GOOGLE_SCHOLAR_ENABLED
//! environment variable to be set to "true" to enable it.
//!
//! Google Scholar does not have an official public API. This implementation uses
//! web scraping which may violate Google's Terms of Service. Use at your own risk.
//!
//! This source is primarily intended for research/educational purposes.

use async_trait::async_trait;
use std::sync::Arc;

use crate::models::{Paper, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const GOOGLE_SCHOLAR_URL: &str = "https://scholar.google.com";

/// Environment variable to enable Google Scholar scraping
const GOOGLE_SCHOLAR_ENABLED_VAR: &str = "GOOGLE_SCHOLAR_ENABLED";

/// Google Scholar research source
///
/// WARNING: This source requires GOOGLE_SCHOLAR_ENABLED=true to be set.
/// It uses web scraping which may violate Google's Terms of Service.
/// Use at your own risk.
#[derive(Debug, Clone)]
pub struct GoogleScholarSource {
    client: Arc<HttpClient>,
    enabled: bool,
}

impl GoogleScholarSource {
    pub fn new() -> Result<Self, SourceError> {
        let enabled = std::env::var(GOOGLE_SCHOLAR_ENABLED_VAR).unwrap_or_default() == "true";

        if !enabled {
            tracing::info!(
                "Google Scholar source is disabled. Set {} to enable.",
                GOOGLE_SCHOLAR_ENABLED_VAR
            );
        }

        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            enabled,
        })
    }
}

impl Default for GoogleScholarSource {
    fn default() -> Self {
        Self::new().expect("Failed to create GoogleScholarSource")
    }
}

#[async_trait]
impl Source for GoogleScholarSource {
    fn id(&self) -> &str {
        "google_scholar"
    }

    fn name(&self) -> &str {
        "Google Scholar"
    }

    fn capabilities(&self) -> SourceCapabilities {
        if !self.enabled {
            return SourceCapabilities::empty();
        }
        // Search only: there is no verified DOI lookup. The old get_by_doi
        // returned a fabricated paper for any 2xx from the search page.
        SourceCapabilities::SEARCH
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        if !self.enabled {
            return Err(SourceError::Other(format!(
                "Google Scholar is disabled. Set {} to enable.",
                GOOGLE_SCHOLAR_ENABLED_VAR
            )));
        }

        let max_results = query.max_results.min(10);

        // Note: Direct scraping of Google Scholar is complex due to dynamic content
        // and anti-bot measures. This is a simplified implementation.
        // In practice, you may want to use a service like SerpApi or similar.

        let url = format!(
            "{}?hl=en&q={}&start={}&num={}",
            GOOGLE_SCHOLAR_URL,
            urlencoding::encode(&query.query),
            0,
            max_results
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url).header(
                    "User-Agent",
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
                );

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to search Google Scholar: {}", e))
                })?;

                if !response.status().is_success() {
                    let status = response.status();
                    return Err(SourceError::Api(format!(
                        "Google Scholar returned status: {}",
                        status
                    )));
                }

                let text = response
                    .text()
                    .await
                    .map_err(|e| SourceError::Parse(format!("Failed to read response: {}", e)))?;

                Ok(text)
            }
        })
        .await?;

        let papers = self.parse_results(&response, &query.query)?;
        let papers_len = papers.len();
        let mut search_response = SearchResponse::new(papers, "Google Scholar", &query.query);
        search_response.total_results = Some(papers_len);
        Ok(search_response)
    }
}

impl GoogleScholarSource {
    fn parse_results(&self, _html: &str, _query: &str) -> Result<Vec<Paper>, SourceError> {
        Err(SourceError::NotImplemented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        // Should always succeed (even when disabled)
        let source = GoogleScholarSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_disabled_by_default() {
        let source = GoogleScholarSource::new();
        assert!(!source.unwrap().enabled);
    }
}
