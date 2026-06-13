//! WorldWideScience research source implementation.
//!
//! Uses the WorldWideScience API for searching international scientific literature.
//! API documentation: <https://worldwidescience.org/api>
//!
//! WorldWideScience is free and requires no API key.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const WWS_API_BASE: &str = "https://worldwidescience.org/api";

/// WorldWideScience research source
///
/// Uses the WorldWideScience API for searching international scientific literature.
/// Free to use with no API key required.
#[derive(Debug, Clone)]
pub struct WorldWideScienceSource {
    client: Arc<HttpClient>,
}

impl WorldWideScienceSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }
}

impl Default for WorldWideScienceSource {
    fn default() -> Self {
        Self::new().expect("Failed to create WorldWideScienceSource")
    }
}

#[async_trait]
impl Source for WorldWideScienceSource {
    fn id(&self) -> &str {
        "worldwidescience"
    }

    fn name(&self) -> &str {
        "WorldWideScience"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let max_results = query.max_results.min(100);

        let url = format!(
            "{}/search?q={}&limit={}",
            WWS_API_BASE,
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

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to search WorldWideScience: {}", e))
                })?;

                // WorldWideScience API may return 404 or HTML pages
                // Return empty results gracefully
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    tracing::debug!("WorldWideScience API endpoint not available - skipping");
                    return Err(SourceError::Api(
                        "WorldWideScience API not available".to_string(),
                    ));
                }

                // Check content-type header for JSON response
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                if !content_type.contains("application/json") {
                    tracing::debug!(
                        "WorldWideScience API returned non-JSON content-type: {} - skipping",
                        content_type
                    );
                    return Err(SourceError::Api(
                        "WorldWideScience API not available".to_string(),
                    ));
                }

                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "WorldWideScience API returned status {}: {}",
                        status, text
                    )));
                }

                let json: WwsResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse WorldWideScience response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await;

        // Handle API not available gracefully
        match &response {
            Err(SourceError::Api(msg)) if msg.contains("not available") => {
                tracing::debug!("WorldWideScience API not available - returning empty results");
                return Ok(SearchResponse::new(
                    Vec::new(),
                    "WorldWideScience",
                    &query.query,
                ));
            }
            _ => {}
        }

        let response = response?;

        let total = response.total_hits.unwrap_or(0);
        let papers: Result<Vec<Paper>, SourceError> = response
            .records
            .into_iter()
            .map(|record| self.parse_result(&record))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "WorldWideScience", &query.query);
        response.total_results = Some(total);
        Ok(response)
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let url = format!("{}/doi/{}", WWS_API_BASE, urlencoding::encode(&clean_doi));

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url);

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in WorldWideScience: {}", e))
                })?;

                if response.status() == 404 {
                    return Err(SourceError::NotFound(format!(
                        "Paper not found in WorldWideScience: {}",
                        doi
                    )));
                }

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "WorldWideScience API returned status: {}",
                        response.status()
                    )));
                }

                let json: WwsRecord = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse WorldWideScience response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        self.parse_result(&response)
    }
}

impl WorldWideScienceSource {
    fn parse_result(&self, record: &WwsRecord) -> Result<Paper, SourceError> {
        let id = record.doi.clone().unwrap_or_else(|| record.id.clone());
        let title = record.title.clone().unwrap_or_default();
        let abstract_text = record.abstract_text.clone().unwrap_or_default();

        let doi = record.doi.clone().unwrap_or_default();

        let authors: String = record
            .authors
            .iter()
            .filter_map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join("; ");

        let year = record.publication_year.clone().unwrap_or_default();
        let url = if !doi.is_empty() {
            format!("https://doi.org/{}", doi)
        } else {
            format!("https://worldwidescience.org/records/{}", record.id)
        };

        Ok(
            PaperBuilder::new(id, title, url, SourceType::WorldWideScience)
                .authors(&authors)
                .published_date(&year)
                .abstract_text(&abstract_text)
                .doi(&doi)
                .build(),
        )
    }
}

/// WorldWideScience API response
#[derive(Debug, Deserialize)]
struct WwsResponse {
    total_hits: Option<usize>,
    records: Vec<WwsRecord>,
}

#[derive(Debug, Deserialize)]
struct WwsRecord {
    id: String,
    doi: Option<String>,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    publication_year: Option<String>,
    authors: Vec<WwsAuthor>,
}

#[derive(Debug, Deserialize)]
struct WwsAuthor {
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = WorldWideScienceSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = WorldWideScienceSource::new().unwrap();
        assert_eq!(source.id(), "worldwidescience");
        assert_eq!(source.name(), "WorldWideScience");
    }

    #[test]
    fn test_capabilities() {
        let source = WorldWideScienceSource::new().unwrap();
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
            "records": [{
                "id": "wws-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "publication_year": "2024",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
            }]
        }"#;
        let response: WwsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.total_hits, Some(1));
        assert_eq!(response.records.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = WorldWideScienceSource::new().unwrap();
        let json = r#"{
            "total_hits": 1,
            "records": [{
                "id": "wws-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "publication_year": "2024",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
            }]
        }"#;
        let response: WwsResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response.records[0]).unwrap();
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.source, crate::models::SourceType::WorldWideScience);
        assert_eq!(paper.paper_id, "10.1234/mock");
        assert_eq!(paper.url, "https://doi.org/10.1234/mock");
        assert_eq!(paper.published_date.as_deref(), Some("2024"));
    }
}
