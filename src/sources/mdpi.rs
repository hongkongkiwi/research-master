//! MDPI research source implementation.
//!
//! Uses the MDPI API for searching and retrieving research papers.
//! API documentation: <https://developer.mdpi.com>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const MDPI_API_BASE: &str = "https://api.mdpi.com/v1";

/// MDPI research source
///
/// Uses the MDPI API for searching and retrieving research papers.
/// API documentation: <https://www.mdpi.com>
///
/// API key can be obtained from MDPI and set via MDPI_API_KEY environment variable.
#[derive(Debug, Clone)]
pub struct MdpiSource {
    client: Arc<HttpClient>,
    api_key: Option<String>,
}

impl MdpiSource {
    pub fn new() -> Result<Self, SourceError> {
        let api_key = std::env::var("MDPI_API_KEY").ok();
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            api_key,
        })
    }
}

impl Default for MdpiSource {
    fn default() -> Self {
        Self::new().expect("Failed to create MdpiSource")
    }
}

#[async_trait]
impl Source for MdpiSource {
    fn id(&self) -> &str {
        "mdpi"
    }

    fn name(&self) -> &str {
        "MDPI"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let max_results = query.max_results.min(100);

        let url = format!(
            "{}?query={}&page_size={}",
            MDPI_API_BASE,
            urlencoding::encode(&query.query),
            max_results
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

                if let Some(key) = api_key {
                    request = request.header("Authorization", format!("Bearer {}", key));
                }

                let response = request
                    .send()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to search MDPI: {}", e)))?;

                // MDPI API may return 403 if no valid API key is provided
                // In this case, return empty results gracefully
                if response.status() == reqwest::StatusCode::FORBIDDEN {
                    tracing::debug!(
                        "MDPI API returned 403 - likely requires authentication, skipping"
                    );
                    return Err(SourceError::Api("MDPI requires authentication".to_string()));
                }

                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "MDPI API returned status {}: {}",
                        status, text
                    )));
                }

                let json: MdpiResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse MDPI response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await;

        // Check if we got an auth error and return empty results silently
        match &response {
            Err(SourceError::Api(msg))
                if msg.contains("authentication") || msg.contains("requires") =>
            {
                tracing::debug!("MDPI requires authentication - returning empty results");
                return Ok(SearchResponse::new(Vec::new(), "MDPI", &query.query));
            }
            _ => {}
        }

        let response = response?;

        let total = response.total_results.unwrap_or(0);
        let papers: Result<Vec<Paper>, SourceError> = response
            .items
            .into_iter()
            .map(|item| self.parse_result(&item))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "MDPI", &query.query);
        response.total_results = Some(total);
        Ok(response)
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let url = format!("{}/doi/{}", MDPI_API_BASE, urlencoding::encode(&clean_doi));

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url);

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in MDPI: {}", e))
                })?;

                if response.status() == 404 {
                    return Err(SourceError::NotFound(format!(
                        "Paper not found in MDPI: {}",
                        doi
                    )));
                }

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "MDPI API returned status: {}",
                        response.status()
                    )));
                }

                response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse MDPI response: {}", e))
                })
            }
        })
        .await?;

        self.parse_result(&response)
    }
}

impl MdpiSource {
    fn parse_result(&self, item: &MdpiItem) -> Result<Paper, SourceError> {
        let id = item.doi.clone().unwrap_or_else(|| item.id.clone());
        let title = item.title.clone().unwrap_or_default();
        let abstract_text = item.abstract_text.clone().unwrap_or_default();

        let doi = item.doi.clone().unwrap_or_default();

        let authors: String = item
            .authors
            .iter()
            .filter_map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join("; ");

        let year = item.publication_date.clone().unwrap_or_default();
        let url = if !doi.is_empty() {
            format!("https://doi.org/{}", doi)
        } else {
            format!("https://www.mdpi.com/{}", item.id)
        };

        Ok(
            PaperBuilder::new(id, title, url, SourceType::Other("mdpi".to_string()))
                .authors(&authors)
                .published_date(&year)
                .abstract_text(&abstract_text)
                .doi(&doi)
                .build(),
        )
    }
}

/// MDPI API response
#[derive(Debug, Deserialize)]
struct MdpiResponse {
    total_results: Option<usize>,
    items: Vec<MdpiItem>,
}

#[derive(Debug, Deserialize)]
struct MdpiItem {
    id: String,
    doi: Option<String>,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    publication_date: Option<String>,
    authors: Vec<MdpiAuthor>,
}

#[derive(Debug, Deserialize)]
struct MdpiAuthor {
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = MdpiSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = MdpiSource::new().unwrap();
        assert_eq!(source.id(), "mdpi");
        assert_eq!(source.name(), "MDPI");
    }

    #[test]
    fn test_capabilities() {
        let source = MdpiSource::new().unwrap();
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
            "total_results": 1,
            "items": [{
                "id": "mdpi-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "publication_date": "2024-02-01",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
            }]
        }"#;
        let response: MdpiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.total_results, Some(1));
        assert_eq!(response.items.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = MdpiSource::new().unwrap();
        let json = r#"{
            "total_results": 1,
            "items": [{
                "id": "mdpi-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "publication_date": "2024-02-01",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
            }]
        }"#;
        let response: MdpiResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response.items[0]).unwrap();
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.source.id(), "mdpi");
        assert_eq!(paper.paper_id, "10.1234/mock");
        assert_eq!(paper.url, "https://doi.org/10.1234/mock");
        assert_eq!(paper.published_date.as_deref(), Some("2024-02-01"));
    }
}
