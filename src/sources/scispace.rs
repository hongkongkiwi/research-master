//! SciSpace research source implementation.
//!
//! Uses the SciSpace (Typeset) API for searching and retrieving research papers.
//! API documentation: <https://typeset.io/api>
//!
//! SciSpace is free and requires no API key for basic search.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const SCISPACE_API_BASE: &str = "https://api.typeset.io/v1";

/// SciSpace research source
///
/// Uses the SciSpace (Typeset) API for searching and retrieving research papers.
/// Free to use with no API key required.
#[derive(Debug, Clone)]
pub struct ScispaceSource {
    client: Arc<HttpClient>,
}

impl ScispaceSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }
}

impl Default for ScispaceSource {
    fn default() -> Self {
        Self::new().expect("Failed to create ScispaceSource")
    }
}

#[async_trait]
impl Source for ScispaceSource {
    fn id(&self) -> &str {
        "scispace"
    }

    fn name(&self) -> &str {
        "SciSpace"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let max_results = query.max_results.min(50);

        let url = format!(
            "{}/papers/search?query={}&limit={}",
            SCISPACE_API_BASE,
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
                    SourceError::Network(format!("Failed to search SciSpace: {}", e))
                })?;

                if !response.status().is_success() {
                    let status = response.status();
                    // SciSpace API may return 404 for unknown endpoints or 403 for rate limiting
                    // Return empty results gracefully for these cases
                    if status == reqwest::StatusCode::NOT_FOUND
                        || status == reqwest::StatusCode::FORBIDDEN
                    {
                        tracing::debug!(
                            "SciSpace API returned {} - likely requires different endpoint, skipping",
                            status
                        );
                        return Err(SourceError::Api("SciSpace endpoint unavailable".to_string()));
                    }
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "SciSpace API returned status {}: {}",
                        status, text
                    )));
                }

                let json: ScispaceResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse SciSpace response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await;

        // Handle API unavailability gracefully
        let response = match response {
            Ok(r) => r,
            Err(SourceError::Api(msg)) if msg.contains("unavailable") => {
                tracing::debug!("SciSpace API unavailable - returning empty results");
                return Ok(SearchResponse::new(vec![], "SciSpace", &query.query));
            }
            Err(e) => return Err(e),
        };

        let total = response.total_results.unwrap_or(0);
        let papers: Result<Vec<Paper>, SourceError> = response
            .papers
            .into_iter()
            .map(|paper| self.parse_result(&paper))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "SciSpace", &query.query);
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
            "{}/papers/doi/{}",
            SCISPACE_API_BASE,
            urlencoding::encode(&clean_doi)
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url);

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in SciSpace: {}", e))
                })?;

                if response.status() == 404 {
                    return Err(SourceError::NotFound(format!(
                        "Paper not found in SciSpace: {}",
                        doi
                    )));
                }

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "SciSpace API returned status: {}",
                        response.status()
                    )));
                }

                let json: ScispacePaper = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse SciSpace response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        self.parse_result(&response)
    }
}

impl ScispaceSource {
    fn parse_result(&self, paper: &ScispacePaper) -> Result<Paper, SourceError> {
        let id = paper.doi.clone().unwrap_or_else(|| paper.id.clone());
        let title = paper.title.clone().unwrap_or_default();
        let abstract_text = paper.abstract_text.clone().unwrap_or_default();

        let doi = paper.doi.clone().unwrap_or_default();

        let authors: String = paper
            .authors
            .iter()
            .filter_map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join("; ");

        let year = paper.publication_year.clone().unwrap_or_default();
        let url = if !doi.is_empty() {
            format!("https://doi.org/{}", doi)
        } else {
            format!("https://typeset.io/papers/{}", paper.id)
        };

        let pdf_url = paper.pdf_url.clone();

        Ok(PaperBuilder::new(id, title, url, SourceType::Scispace)
            .authors(&authors)
            .published_date(&year)
            .abstract_text(&abstract_text)
            .doi(&doi)
            .pdf_url(pdf_url.unwrap_or_default())
            .build())
    }
}

/// SciSpace API response
#[derive(Debug, Deserialize)]
struct ScispaceResponse {
    total_results: Option<usize>,
    papers: Vec<ScispacePaper>,
}

#[derive(Debug, Deserialize)]
struct ScispacePaper {
    id: String,
    doi: Option<String>,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    publication_year: Option<String>,
    authors: Vec<ScispaceAuthor>,
    pdf_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScispaceAuthor {
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = ScispaceSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = ScispaceSource::new().unwrap();
        assert_eq!(source.id(), "scispace");
        assert_eq!(source.name(), "SciSpace");
    }

    #[test]
    fn test_capabilities() {
        let source = ScispaceSource::new().unwrap();
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
            "papers": [{
                "id": "scispace-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "publication_year": "2024",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}],
                "pdf_url": "https://typeset.io/pdf/mock.pdf"
            }]
        }"#;
        let response: ScispaceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.total_results, Some(1));
        assert_eq!(response.papers.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = ScispaceSource::new().unwrap();
        let json = r#"{
            "total_results": 1,
            "papers": [{
                "id": "scispace-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "publication_year": "2024",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}],
                "pdf_url": "https://typeset.io/pdf/mock.pdf"
            }]
        }"#;
        let response: ScispaceResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response.papers[0]).unwrap();
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.source, crate::models::SourceType::Scispace);
        assert_eq!(paper.paper_id, "10.1234/mock");
        assert_eq!(paper.url, "https://doi.org/10.1234/mock");
        assert_eq!(
            paper.pdf_url.as_deref(),
            Some("https://typeset.io/pdf/mock.pdf")
        );
    }
}
