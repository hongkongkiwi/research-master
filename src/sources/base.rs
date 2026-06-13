//! BASE (Bielefeld Academic Search Engine) research source implementation.
//!
//! Uses the BASE API for searching academic resources from repositories worldwide.
//! API documentation: <https://www.base-search.net/about/en/developers.php>
//!
//! BASE requires a free API key for higher rate limits, but works with basic access.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const BASE_API_BASE: &str = "https://api.base-search.net/cgi-bin/BaseHttpSearchInterface.fcgi";

/// BASE research source
///
/// Uses the BASE API for searching academic resources from repositories worldwide.
/// Free API key available at base-search.net
#[derive(Debug, Clone)]
pub struct BaseSource {
    client: Arc<HttpClient>,
    api_key: Option<String>,
}

impl BaseSource {
    pub fn new() -> Result<Self, SourceError> {
        let api_key = std::env::var("BASE_API_KEY").ok();
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            api_key,
        })
    }
}

impl Default for BaseSource {
    fn default() -> Self {
        Self::new().expect("Failed to create BaseSource")
    }
}

#[async_trait]
impl Source for BaseSource {
    fn id(&self) -> &str {
        "base"
    }

    fn name(&self) -> &str {
        "BASE"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let max_results = query.max_results.min(100);

        let url = format!(
            "{}?q={}&output=json&n={}",
            BASE_API_BASE,
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
                    request = request.header("Authorization", format!("Basic {}", key));
                }

                let response = request
                    .send()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to search BASE: {}", e)))?;

                if !response.status().is_success() {
                    let status = response.status();
                    // Check for rate limiting
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        tracing::debug!("BASE API rate-limited - returning empty results");
                        return Err(SourceError::Api("BASE rate-limited".to_string()));
                    }
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "BASE API returned status {}: {}",
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
                        "BASE returned non-JSON content-type: {} - rate-limited?",
                        content_type
                    );
                    return Err(SourceError::Api("BASE rate-limited".to_string()));
                }

                let json: BaseResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse BASE response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await;

        // Handle rate limiting gracefully
        let response = match response {
            Ok(r) => r,
            Err(SourceError::Api(msg)) if msg.contains("rate-limited") => {
                tracing::debug!("BASE rate-limited - returning empty results");
                return Ok(SearchResponse::new(vec![], "BASE", &query.query));
            }
            Err(e) => return Err(e),
        };

        let total = response.result_num_found.unwrap_or(0);
        let papers: Result<Vec<Paper>, SourceError> = response
            .documents
            .into_iter()
            .map(|doc| self.parse_result(&doc))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "BASE", &query.query);
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
            "{}?q=doi:\"{}\"&output=json&n=1",
            BASE_API_BASE,
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
                    request = request.header("Authorization", format!("Basic {}", key));
                }

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in BASE: {}", e))
                })?;

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "BASE API returned status: {}",
                        response.status()
                    )));
                }

                let json: BaseResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse BASE response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        let doc =
            response.documents.into_iter().next().ok_or_else(|| {
                SourceError::NotFound(format!("Paper not found in BASE: {}", doi))
            })?;

        self.parse_result(&doc)
    }
}

impl BaseSource {
    fn parse_result(&self, doc: &BaseDocument) -> Result<Paper, SourceError> {
        let id = doc.id.clone();
        let title = doc.title.clone().unwrap_or_default();
        let abstract_text = doc.abstract_text.clone().unwrap_or_default();

        let doi = doc.doi.clone().unwrap_or_default();

        let authors: String = doc
            .authors
            .iter()
            .filter_map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join("; ");

        let year = doc.year.clone().unwrap_or_default();
        let url = if !doi.is_empty() {
            format!("https://doi.org/{}", doi)
        } else {
            doc.link.clone().unwrap_or_default()
        };

        let _publisher = doc.publisher.clone().unwrap_or_default();

        Ok(PaperBuilder::new(id, title, url, SourceType::Base)
            .authors(&authors)
            .published_date(&year)
            .abstract_text(&abstract_text)
            .doi(&doi)
            .build())
    }
}

/// BASE API response
#[derive(Debug, Deserialize)]
struct BaseResponse {
    result_num_found: Option<usize>,
    documents: Vec<BaseDocument>,
}

#[derive(Debug, Deserialize)]
struct BaseDocument {
    id: String,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    authors: Vec<BaseAuthor>,
    doi: Option<String>,
    year: Option<String>,
    link: Option<String>,
    publisher: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BaseAuthor {
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = BaseSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = BaseSource::new().unwrap();
        assert_eq!(source.id(), "base");
        assert_eq!(source.name(), "BASE");
    }

    #[test]
    fn test_capabilities() {
        let source = BaseSource::new().unwrap();
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
            "documents": [{
                "id": "base-1",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}],
                "doi": "10.1234/mock",
                "year": "2024",
                "link": "https://example.org/base-1",
                "publisher": "Mock Publisher"
            }]
        }"#;
        let response: BaseResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.result_num_found, Some(1));
        assert_eq!(response.documents.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = BaseSource::new().unwrap();
        let json = r#"{
            "result_num_found": 1,
            "documents": [{
                "id": "base-1",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}],
                "doi": "10.1234/mock",
                "year": "2024",
                "link": "https://example.org/base-1",
                "publisher": "Mock Publisher"
            }]
        }"#;
        let response: BaseResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response.documents[0]).unwrap();
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.source, crate::models::SourceType::Base);
        assert_eq!(paper.paper_id, "base-1");
        assert_eq!(paper.url, "https://doi.org/10.1234/mock");
        assert_eq!(paper.published_date.as_deref(), Some("2024"));
    }
}
