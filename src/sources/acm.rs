//! ACM research source implementation.
//!
//! Uses the ACM Digital Library API for searching and retrieving research papers.
//! API documentation: <https://dl.acm.org/api>
//!
//! Requires an API key from the ACM Developer Portal.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const ACM_API_BASE: &str = "https://dl.acm.org/api";

/// ACM research source
///
/// Uses the ACM Digital Library API for searching and retrieving research papers.
/// Requires an API key from developer.acm.org
#[derive(Debug, Clone)]
pub struct AcmSource {
    client: Arc<HttpClient>,
    api_key: Option<String>,
}

impl AcmSource {
    pub fn new() -> Result<Self, SourceError> {
        let api_key = std::env::var("ACM_API_KEY").ok();
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            api_key,
        })
    }
}

impl Default for AcmSource {
    fn default() -> Self {
        Self::new().expect("Failed to create AcmSource")
    }
}

#[async_trait]
impl Source for AcmSource {
    fn id(&self) -> &str {
        "acm"
    }

    fn name(&self) -> &str {
        "ACM Digital Library"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        if self.api_key.is_none() {
            tracing::debug!("ACM_API_KEY not set - skipping search");
            return Ok(SearchResponse::new(Vec::new(), "ACM", &query.query));
        }

        let max_results = query.max_results.min(100);

        let url = format!(
            "{}?q={}&limit={}",
            ACM_API_BASE,
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
                    .map_err(|e| SourceError::Network(format!("Failed to search ACM: {}", e)))?;

                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "ACM API returned status {}: {}",
                        status, text
                    )));
                }

                let json: AcmResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse ACM response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        let total = response.total_hits.unwrap_or(0);
        let papers: Result<Vec<Paper>, SourceError> = response
            .records
            .into_iter()
            .map(|record| self.parse_result(&record))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "ACM", &query.query);
        response.total_results = Some(total);
        Ok(response)
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let url = format!("{}/doi/{}", ACM_API_BASE, urlencoding::encode(&clean_doi));

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
                    SourceError::Network(format!("Failed to lookup DOI in ACM: {}", e))
                })?;

                if response.status() == 404 {
                    return Err(SourceError::NotFound(format!(
                        "Paper not found in ACM: {}",
                        doi
                    )));
                }

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "ACM API returned status: {}",
                        response.status()
                    )));
                }

                let json: AcmRecord = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse ACM response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        self.parse_result(&response)
    }
}

impl AcmSource {
    fn parse_result(&self, record: &AcmRecord) -> Result<Paper, SourceError> {
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

        let year = record
            .publication_year
            .map(|y| y.to_string())
            .unwrap_or_default();
        let url = format!("https://dl.acm.org/doi/{}", doi);

        let pdf_url = record.pdf_url.clone();

        Ok(PaperBuilder::new(id, title, url, SourceType::Acm)
            .authors(&authors)
            .published_date(&year)
            .abstract_text(&abstract_text)
            .doi(&doi)
            .pdf_url(pdf_url.unwrap_or_default())
            .build())
    }
}

/// ACM API response
#[derive(Debug, Deserialize)]
struct AcmResponse {
    total_hits: Option<usize>,
    records: Vec<AcmRecord>,
}

#[derive(Debug, Deserialize)]
struct AcmRecord {
    id: String,
    doi: Option<String>,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    publication_year: Option<i32>,
    authors: Vec<AcmAuthor>,
    pdf_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AcmAuthor {
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = AcmSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = AcmSource::new().unwrap();
        assert_eq!(source.id(), "acm");
        assert_eq!(source.name(), "ACM Digital Library");
    }

    #[test]
    fn test_capabilities() {
        let source = AcmSource::new().unwrap();
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
                "id": "acm-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "publication_year": 2024,
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}],
                "pdf_url": "https://dl.acm.org/doi/pdf/10.1234/mock"
            }]
        }"#;
        let response: AcmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.total_hits, Some(1));
        assert_eq!(response.records.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = AcmSource::new().unwrap();
        let json = r#"{
            "total_hits": 1,
            "records": [{
                "id": "acm-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "publication_year": 2024,
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}],
                "pdf_url": "https://dl.acm.org/doi/pdf/10.1234/mock"
            }]
        }"#;
        let response: AcmResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response.records[0]).unwrap();
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.source, crate::models::SourceType::Acm);
        assert_eq!(paper.paper_id, "10.1234/mock");
        assert_eq!(paper.published_date.as_deref(), Some("2024"));
        assert_eq!(
            paper.pdf_url.as_deref(),
            Some("https://dl.acm.org/doi/pdf/10.1234/mock")
        );
    }
}
