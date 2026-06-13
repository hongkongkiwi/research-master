//! CORE research source implementation.
//!
//! Uses the CORE API for searching and retrieving research papers.
//! API documentation: <https://core.ac.uk/services/api>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const CORE_API_BASE: &str = "https://api.core.ac.uk/v3";

/// CORE research source
///
/// Uses the CORE API for searching and retrieving research papers.
/// API requires a free API key from https://core.ac.uk/register
#[derive(Debug, Clone)]
pub struct CoreSource {
    client: Arc<HttpClient>,
    api_key: Option<String>,
}

impl CoreSource {
    pub fn new() -> Result<Self, SourceError> {
        let api_key = std::env::var("CORE_API_KEY").ok();
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            api_key,
        })
    }
}

impl Default for CoreSource {
    fn default() -> Self {
        Self::new().expect("Failed to create CoreSource")
    }
}

#[async_trait]
impl Source for CoreSource {
    fn id(&self) -> &str {
        "core"
    }

    fn name(&self) -> &str {
        "CORE"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let max_results = query.max_results.min(100);

        let url = format!(
            "{}/works/search?limit={}&q={}",
            CORE_API_BASE,
            max_results,
            urlencoding::encode(&query.query)
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();
        let api_key = self.api_key.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            let api_key = api_key.clone();
            async move {
                let mut request = client.get(&url);
                if let Some(ref key) = api_key {
                    request = request.header("Authorization", format!("Bearer {}", key));
                }

                let response = request
                    .send()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to search CORE: {}", e)))?;

                if !response.status().is_success() {
                    let status = response.status();
                    // CORE API may return 404 for unknown endpoints
                    // Return empty results gracefully
                    if status == reqwest::StatusCode::NOT_FOUND {
                        tracing::debug!("CORE API endpoint not found - returning empty results");
                        return Err(SourceError::Api("CORE API unavailable".to_string()));
                    }
                    // Handle rate limiting
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        tracing::debug!("CORE API rate-limited - returning empty results");
                        return Err(SourceError::Api("CORE rate-limited".to_string()));
                    }
                    let text = response
                        .text()
                        .await
                        .unwrap_or_else(|_| "Failed to read response body".to_string());
                    return Err(SourceError::Api(format!(
                        "CORE API returned status {}: {}",
                        status, text
                    )));
                }

                let json: CoreResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse CORE response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await;

        // Handle API unavailability and rate limiting gracefully
        let response = match response {
            Ok(r) => r,
            Err(SourceError::Api(msg)) if msg.contains("unavailable") => {
                tracing::debug!("CORE API unavailable - returning empty results");
                return Ok(SearchResponse::new(vec![], "CORE", &query.query));
            }
            Err(SourceError::Api(msg)) if msg.contains("rate-limited") => {
                tracing::debug!("CORE rate-limited - returning empty results");
                return Ok(SearchResponse::new(vec![], "CORE", &query.query));
            }
            Err(e) => return Err(e),
        };

        let total = response.total_hits.unwrap_or(0);
        let papers: Result<Vec<Paper>, SourceError> = response
            .results
            .into_iter()
            .map(|item| self.parse_result(&item))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "CORE", &query.query);
        response.total_results = Some(total);
        Ok(response)
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let url = format!(
            "{}/works/{}",
            CORE_API_BASE,
            urlencoding::encode(&clean_doi)
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();
        let api_key = self.api_key.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            let api_key = api_key.clone();
            async move {
                let mut request = client.get(&url);
                if let Some(ref key) = api_key {
                    request = request.header("Authorization", format!("Bearer {}", key));
                }

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in CORE: {}", e))
                })?;

                if !response.status().is_success() {
                    return Err(SourceError::NotFound(format!(
                        "Paper not found in CORE: {}",
                        doi
                    )));
                }

                response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse CORE response: {}", e))
                })
            }
        })
        .await?;

        self.parse_result(&response)
    }
}

impl CoreSource {
    fn parse_result(&self, item: &CoreWork) -> Result<Paper, SourceError> {
        let id = item.id.clone();
        let title = item.title.clone().unwrap_or_default();
        let abstract_text = item.description.clone().unwrap_or_default();
        let doi = item.doi.clone().unwrap_or_default();

        let authors: String = item
            .authors
            .iter()
            .filter_map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join("; ");

        let year = item.publication_date.clone().unwrap_or_default();
        let url = item.url.clone().unwrap_or_else(|| {
            if !doi.is_empty() {
                format!("https://doi.org/{}", doi)
            } else {
                format!("https://core.ac.uk/reader/{}", id)
            }
        });

        Ok(PaperBuilder::new(id, title, url, SourceType::CORE)
            .authors(&authors)
            .published_date(&year)
            .abstract_text(&abstract_text)
            .doi(&doi)
            .build())
    }
}

/// CORE API response
#[derive(Debug, Deserialize)]
struct CoreResponse {
    total_hits: Option<usize>,
    results: Vec<CoreWork>,
}

#[derive(Debug, Deserialize)]
struct CoreWork {
    id: String,
    title: Option<String>,
    description: Option<String>,
    doi: Option<String>,
    publication_date: Option<String>,
    url: Option<String>,
    authors: Vec<CoreAuthor>,
}

#[derive(Debug, Deserialize)]
struct CoreAuthor {
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = CoreSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = CoreSource::new().unwrap();
        assert_eq!(source.id(), "core");
        assert_eq!(source.name(), "CORE");
    }

    #[test]
    fn test_capabilities() {
        let source = CoreSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::DOI_LOOKUP));
        assert_eq!(
            caps,
            SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
        );
    }

    #[test]
    fn test_response_parsing_from_mock_json() {
        let json = r#"{
            "total_hits": 1,
            "results": [{
                "id": "core-1",
                "title": "Mock Paper Title",
                "description": "Mock abstract text.",
                "doi": "10.1234/mock",
                "publication_date": "2024-01-15",
                "url": "https://core.ac.uk/reader/core-1",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
            }]
        }"#;
        let response: CoreResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.total_hits, Some(1));
        assert_eq!(response.results.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = CoreSource::new().unwrap();
        let json = r#"{
            "total_hits": 1,
            "results": [{
                "id": "core-1",
                "title": "Mock Paper Title",
                "description": "Mock abstract text.",
                "doi": "10.1234/mock",
                "publication_date": "2024-01-15",
                "url": "https://core.ac.uk/reader/core-1",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
            }]
        }"#;
        let response: CoreResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response.results[0]).unwrap();
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.source, crate::models::SourceType::CORE);
        assert_eq!(paper.paper_id, "core-1");
        assert_eq!(paper.url, "https://core.ac.uk/reader/core-1");
        assert_eq!(paper.published_date.as_deref(), Some("2024-01-15"));
    }
}
