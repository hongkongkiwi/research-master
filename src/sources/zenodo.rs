//! Zenodo research source implementation.
//!
//! Uses the Zenodo API for searching and retrieving research papers.
//! API documentation: <https://developers.zenodo.org>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const ZENODO_API_BASE: &str = "https://zenodo.org/api/records";

/// Zenodo research source
///
/// Uses the Zenodo API for searching and retrieving research papers.
/// Zenodo is free and requires no API key.
#[derive(Debug, Clone)]
pub struct ZenodoSource {
    client: Arc<HttpClient>,
}

impl ZenodoSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }
}

impl Default for ZenodoSource {
    fn default() -> Self {
        Self::new().expect("Failed to create ZenodoSource")
    }
}

#[async_trait]
impl Source for ZenodoSource {
    fn id(&self) -> &str {
        "zenodo"
    }

    fn name(&self) -> &str {
        "Zenodo"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let max_results = query.max_results.min(1000);

        let url = format!(
            "{}?q={}&size={}&type=publication",
            ZENODO_API_BASE,
            urlencoding::encode(&query.query),
            max_results
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url);

                let response = request
                    .send()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to search Zenodo: {}", e)))?;

                if !response.status().is_success() {
                    let status = response.status();
                    // Check if we got rate limited (often returns HTML)
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        tracing::debug!("Zenodo API rate-limited - returning empty results");
                        return Err(SourceError::Api("Zenodo rate-limited".to_string()));
                    }
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "Zenodo API returned status {}: {}",
                        status, text
                    )));
                }

                // Check content-type to ensure we got JSON
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                if !content_type.contains("application/json") {
                    tracing::debug!(
                        "Zenodo returned non-JSON content-type: {} - rate-limited?",
                        content_type
                    );
                    return Err(SourceError::Api("Zenodo rate-limited".to_string()));
                }

                // Capture response text for better error messages
                let response_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read response body".to_string());

                let json: ZenodoResponse = serde_json::from_str(&response_text).map_err(|e| {
                    let preview = response_text.chars().take(500).collect::<String>();
                    tracing::warn!("Zenodo parse error: {}", preview);
                    SourceError::Parse(format!(
                        "Failed to parse Zenodo response: {}. Response: {}",
                        e, preview
                    ))
                })?;

                Ok(json)
            }
        })
        .await;

        // Handle rate limiting gracefully
        let response = match response {
            Ok(r) => r,
            Err(SourceError::Api(msg)) if msg.contains("rate-limited") => {
                tracing::debug!("Zenodo rate-limited - returning empty results");
                return Ok(SearchResponse::new(vec![], "Zenodo", &query.query));
            }
            Err(e) => return Err(e),
        };

        let total = match response.hits.total {
            ZenodoTotal::Struct { value } => value,
            ZenodoTotal::Integer(n) => n,
        };
        let papers: Result<Vec<Paper>, SourceError> = response
            .hits
            .hits
            .into_iter()
            .map(|item| self.parse_result(&item))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "Zenodo", &query.query);
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
            "{}?q=doi:\"{}\"",
            ZENODO_API_BASE,
            urlencoding::encode(&clean_doi)
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response: ZenodoResponse = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url);

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in Zenodo: {}", e))
                })?;

                if !response.status().is_success() {
                    return Err(SourceError::NotFound(format!(
                        "Paper not found in Zenodo: {}",
                        doi
                    )));
                }

                // Capture response text for better error messages
                let response_text = response.text().await.map_err(|e| {
                    SourceError::Network(format!("Failed to read Zenodo response: {}", e))
                })?;

                serde_json::from_str::<ZenodoResponse>(&response_text).map_err(|e| {
                    let preview = response_text.chars().take(500).collect::<String>();
                    tracing::warn!("Zenodo DOI parse error: {}", preview);
                    SourceError::Parse(format!(
                        "Failed to parse Zenodo response: {}. Response: {}",
                        e, preview
                    ))
                })
            }
        })
        .await?;

        if let Some(hit) = response.hits.hits.into_iter().next() {
            self.parse_result(&hit)
        } else {
            Err(SourceError::NotFound(format!(
                "Paper not found in Zenodo: {}",
                doi
            )))
        }
    }
}

impl ZenodoSource {
    fn parse_result(&self, item: &ZenodoHit) -> Result<Paper, SourceError> {
        let id = item.id.to_string();
        let title = item.metadata.title.clone().unwrap_or_default();
        let abstract_text = item.metadata.description.clone().unwrap_or_default();

        let doi = item.metadata.doi.clone().unwrap_or_default();

        let authors: String = item
            .metadata
            .creators
            .iter()
            .filter_map(|c| c.name.clone())
            .collect::<Vec<_>>()
            .join("; ");

        let year = item.metadata.publication_date.clone().unwrap_or_default();
        let url = item.links.html.clone().unwrap_or_else(|| {
            if !doi.is_empty() {
                format!("https://doi.org/{}", doi)
            } else {
                format!("https://zenodo.org/record/{}", id)
            }
        });

        Ok(
            PaperBuilder::new(id, title, url, SourceType::Other("zenodo".to_string()))
                .authors(&authors)
                .published_date(&year)
                .abstract_text(&abstract_text)
                .doi(&doi)
                .build(),
        )
    }
}

/// Zenodo API response
#[derive(Debug, Deserialize)]
struct ZenodoResponse {
    hits: ZenodoHits,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ZenodoTotal {
    Struct { value: usize },
    Integer(usize),
}

impl Default for ZenodoTotal {
    fn default() -> Self {
        ZenodoTotal::Integer(0)
    }
}

#[derive(Debug, Deserialize)]
struct ZenodoHits {
    #[serde(default)]
    total: ZenodoTotal,
    hits: Vec<ZenodoHit>,
}

#[derive(Debug, Deserialize)]
struct ZenodoHit {
    id: usize,
    metadata: ZenodoMetadata,
    links: ZenodoLinks,
}

#[derive(Debug, Deserialize)]
struct ZenodoMetadata {
    title: Option<String>,
    description: Option<String>,
    doi: Option<String>,
    publication_date: Option<String>,
    creators: Vec<ZenodoCreator>,
}

#[derive(Debug, Deserialize)]
struct ZenodoCreator {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZenodoLinks {
    html: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_json_with_struct_total() -> &'static str {
        r#"{
            "hits": {
                "total": {"value": 1},
                "hits": [{
                    "id": 12345,
                    "metadata": {
                        "title": "Mock Paper Title",
                        "description": "Mock abstract text.",
                        "doi": "10.1234/mock",
                        "publication_date": "2024-06-01",
                        "creators": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
                    },
                    "links": {"html": "https://zenodo.org/records/12345"}
                }]
            }
        }"#
    }

    #[test]
    fn test_source_creation() {
        let source = ZenodoSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = ZenodoSource::new().unwrap();
        assert_eq!(source.id(), "zenodo");
        assert_eq!(source.name(), "Zenodo");
    }

    #[test]
    fn test_capabilities() {
        let source = ZenodoSource::new().unwrap();
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
        let response: ZenodoResponse = serde_json::from_str(mock_json_with_struct_total()).unwrap();
        match response.hits.total {
            ZenodoTotal::Struct { value } => assert_eq!(value, 1),
            ZenodoTotal::Integer(value) => panic!("expected struct total, got {value}"),
        }
        assert_eq!(response.hits.hits.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = ZenodoSource::new().unwrap();
        let response: ZenodoResponse = serde_json::from_str(mock_json_with_struct_total()).unwrap();
        let paper = source.parse_result(&response.hits.hits[0]).unwrap();
        assert_eq!(paper.paper_id, "12345");
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.url, "https://zenodo.org/records/12345");
        assert_eq!(paper.published_date.as_deref(), Some("2024-06-01"));
        assert_eq!(paper.source.id(), "zenodo");
    }

    #[test]
    fn test_zenodo_total_parses_integer_variant() {
        let json = r#"{"hits": {"total": 2, "hits": []}}"#;
        let response: ZenodoResponse = serde_json::from_str(json).unwrap();
        match response.hits.total {
            ZenodoTotal::Integer(value) => assert_eq!(value, 2),
            ZenodoTotal::Struct { value } => panic!("expected integer total, got {value}"),
        }
    }
}
