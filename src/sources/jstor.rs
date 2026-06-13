//! JSTOR research source implementation.
//!
//! Uses the JSTOR API for searching and retrieving research papers.
//! API documentation: <https://www.jstor.org/api>
//!
//! Requires a JSTOR API key. Get one from: <https://www.jstor.org/developer>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const JSTOR_API_BASE: &str = "https://api.jstor.org";

/// JSTOR research source
///
/// Uses the JSTOR API for searching and retrieving research papers.
/// Requires an API key from developer.jstor.org
#[derive(Debug, Clone)]
pub struct JstorSource {
    client: Arc<HttpClient>,
    api_key: Option<String>,
}

impl JstorSource {
    pub fn new() -> Result<Self, SourceError> {
        let api_key = std::env::var("JSTOR_API_KEY").ok();
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            api_key,
        })
    }
}

impl Default for JstorSource {
    fn default() -> Self {
        Self::new().expect("Failed to create JstorSource")
    }
}

#[async_trait]
impl Source for JstorSource {
    fn id(&self) -> &str {
        "jstor"
    }

    fn name(&self) -> &str {
        "JSTOR"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        if self.api_key.is_none() {
            tracing::debug!("JSTOR_API_KEY not set - skipping search");
            return Ok(SearchResponse::new(Vec::new(), "JSTOR", &query.query));
        }

        let max_results = query.max_results.min(100);

        let url = format!(
            "{}/metadata?q={}&limit={}",
            JSTOR_API_BASE,
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
                    .map_err(|e| SourceError::Network(format!("Failed to search JSTOR: {}", e)))?;

                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "JSTOR API returned status {}: {}",
                        status, text
                    )));
                }

                let json: JstorResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse JSTOR response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        let total = response.result_num_found.unwrap_or(0);
        let papers: Result<Vec<Paper>, SourceError> = response
            .items
            .into_iter()
            .map(|item| self.parse_result(&item))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "JSTOR", &query.query);
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
            "{}/metadata/doi/{}",
            JSTOR_API_BASE,
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

                if let Some(key) = api_key {
                    request = request.header("Authorization", format!("Bearer {}", key));
                }

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in JSTOR: {}", e))
                })?;

                if response.status() == 404 {
                    return Err(SourceError::NotFound(format!(
                        "Paper not found in JSTOR: {}",
                        doi
                    )));
                }

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "JSTOR API returned status: {}",
                        response.status()
                    )));
                }

                let json: JstorItem = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse JSTOR response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        self.parse_result(&response)
    }
}

impl JstorSource {
    fn parse_result(&self, item: &JstorItem) -> Result<Paper, SourceError> {
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

        let year = item
            .publication_year
            .map(|y| y.to_string())
            .unwrap_or_default();
        let url = if !doi.is_empty() {
            format!("https://doi.org/{}", doi)
        } else {
            format!("https://www.jstor.org/stable/{}", item.id)
        };

        Ok(PaperBuilder::new(id, title, url, SourceType::Jstor)
            .authors(&authors)
            .published_date(&year)
            .abstract_text(&abstract_text)
            .doi(&doi)
            .build())
    }
}

/// JSTOR API response
#[derive(Debug, Deserialize)]
struct JstorResponse {
    result_num_found: Option<usize>,
    items: Vec<JstorItem>,
}

#[derive(Debug, Deserialize)]
struct JstorItem {
    id: String,
    doi: Option<String>,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    publication_year: Option<i32>,
    authors: Vec<JstorAuthor>,
}

#[derive(Debug, Deserialize)]
struct JstorAuthor {
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = JstorSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = JstorSource::new().unwrap();
        assert_eq!(source.id(), "jstor");
        assert_eq!(source.name(), "JSTOR");
    }

    #[test]
    fn test_capabilities() {
        let source = JstorSource::new().unwrap();
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
            "result_num_found": 1,
            "items": [{
                "id": "jstor-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "publication_year": 2024,
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
            }]
        }"#;
        let response: JstorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.result_num_found, Some(1));
        assert_eq!(response.items.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = JstorSource::new().unwrap();
        let json = r#"{
            "result_num_found": 1,
            "items": [{
                "id": "jstor-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "publication_year": 2024,
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
            }]
        }"#;
        let response: JstorResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response.items[0]).unwrap();
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.source, crate::models::SourceType::Jstor);
        assert_eq!(paper.paper_id, "10.1234/mock");
        assert_eq!(paper.url, "https://doi.org/10.1234/mock");
        assert_eq!(paper.published_date.as_deref(), Some("2024"));
    }
}
