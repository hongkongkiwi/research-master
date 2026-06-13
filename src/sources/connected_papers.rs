//! Connected Papers research source implementation.
//!
//! Uses the Connected Papers API for finding related research papers.
//! API documentation: <https://docs.connectedpapers.com>
//!
//! Connected Papers is free and requires no API key.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const CONNECTED_PAPERS_API_BASE: &str = "https://api.connectedpapers.com/v1";

/// Connected Papers research source
///
/// Uses the Connected Papers API for finding related and similar research papers.
/// Free to use with no API key required.
#[derive(Debug, Clone)]
pub struct ConnectedPapersSource {
    client: Arc<HttpClient>,
}

impl ConnectedPapersSource {
    pub fn new() -> Result<Self, SourceError> {
        // Use 60s timeout for potentially slow API responses
        // User agent respects RESEARCH_MASTER_USER_AGENT env var
        Ok(Self {
            client: Arc::new(HttpClient::with_timeout(
                &crate::utils::get_user_agent(),
                60,
            )?),
        })
    }
}

impl Default for ConnectedPapersSource {
    fn default() -> Self {
        Self::new().expect("Failed to create ConnectedPapersSource")
    }
}

#[async_trait]
impl Source for ConnectedPapersSource {
    fn id(&self) -> &str {
        "connected_papers"
    }

    fn name(&self) -> &str {
        "Connected Papers"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::CITATIONS | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let max_results = query.max_results.min(20);

        // Connected Papers works primarily by finding papers related to a given paper
        // For keyword search, we use their graph search endpoint
        let url = format!(
            "{}/papers/search?query={}&limit={}",
            CONNECTED_PAPERS_API_BASE,
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
                    SourceError::Network(format!("Failed to search Connected Papers: {}", e))
                })?;

                if !response.status().is_success() {
                    let status = response.status();
                    // Connected Papers may return network errors or blocking
                    // Return empty results gracefully
                    if status == reqwest::StatusCode::FORBIDDEN
                        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        tracing::debug!("Connected Papers API blocked or rate-limited - skipping");
                        return Err(SourceError::Api("Connected Papers blocked".to_string()));
                    }
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "Connected Papers API returned status {}: {}",
                        status, text
                    )));
                }

                let json: ConnectedPapersResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse Connected Papers response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await;

        // Handle API blocking gracefully
        let response = match response {
            Ok(r) => r,
            Err(SourceError::Api(msg)) if msg.contains("blocked") => {
                tracing::debug!("Connected Papers API blocked - returning empty results");
                return Ok(SearchResponse::new(
                    vec![],
                    "Connected Papers",
                    &query.query,
                ));
            }
            Err(SourceError::Network(_)) => {
                tracing::debug!("Connected Papers network error - returning empty results");
                return Ok(SearchResponse::new(
                    vec![],
                    "Connected Papers",
                    &query.query,
                ));
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
        let mut response = SearchResponse::new(papers, "Connected Papers", &query.query);
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
            CONNECTED_PAPERS_API_BASE,
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
                    SourceError::Network(format!("Failed to lookup DOI in Connected Papers: {}", e))
                })?;

                if response.status() == 404 {
                    return Err(SourceError::NotFound(format!(
                        "Paper not found in Connected Papers: {}",
                        doi
                    )));
                }

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "Connected Papers API returned status: {}",
                        response.status()
                    )));
                }

                let json: ConnectedPapersPaper = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse Connected Papers response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        self.parse_result(&response)
    }

    async fn get_citations(
        &self,
        request: &crate::models::CitationRequest,
    ) -> Result<SearchResponse, SourceError> {
        let paper_id = request.paper_id.clone();

        let url = format!(
            "{}/papers/doi/{}/citations",
            CONNECTED_PAPERS_API_BASE,
            urlencoding::encode(&paper_id)
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url);

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!(
                        "Failed to get citations from Connected Papers: {}",
                        e
                    ))
                })?;

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "Connected Papers API returned status: {}",
                        response.status()
                    )));
                }

                let json: ConnectedPapersResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse Connected Papers response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        let total = response.total_results.unwrap_or(0);
        let papers: Result<Vec<Paper>, SourceError> = response
            .papers
            .into_iter()
            .map(|paper| self.parse_result(&paper))
            .collect();

        let papers = papers?;
        let mut search_response = SearchResponse::new(papers, "Connected Papers", &paper_id);
        search_response.total_results = Some(total);
        Ok(search_response)
    }

    async fn get_related(
        &self,
        request: &crate::models::CitationRequest,
    ) -> Result<SearchResponse, SourceError> {
        let paper_id = request.paper_id.clone();

        let url = format!(
            "{}/papers/doi/{}/related",
            CONNECTED_PAPERS_API_BASE,
            urlencoding::encode(&paper_id)
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url);

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!(
                        "Failed to get related papers from Connected Papers: {}",
                        e
                    ))
                })?;

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "Connected Papers API returned status: {}",
                        response.status()
                    )));
                }

                let json: ConnectedPapersResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse Connected Papers response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        let total = response.total_results.unwrap_or(0);
        let papers: Result<Vec<Paper>, SourceError> = response
            .papers
            .into_iter()
            .map(|paper| self.parse_result(&paper))
            .collect();

        let papers = papers?;
        let mut search_response = SearchResponse::new(papers, "Connected Papers", &paper_id);
        search_response.total_results = Some(total);
        Ok(search_response)
    }
}

impl ConnectedPapersSource {
    fn parse_result(&self, paper: &ConnectedPapersPaper) -> Result<Paper, SourceError> {
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

        let year = paper.year.clone().unwrap_or_default();
        let url = if !doi.is_empty() {
            format!("https://doi.org/{}", doi)
        } else {
            format!("https://www.connectedpapers.com/main/{}", paper.id)
        };

        Ok(
            PaperBuilder::new(id, title, url, SourceType::ConnectedPapers)
                .authors(&authors)
                .published_date(&year)
                .abstract_text(&abstract_text)
                .doi(&doi)
                .build(),
        )
    }
}

/// Connected Papers API response
#[derive(Debug, Deserialize)]
struct ConnectedPapersResponse {
    total_results: Option<usize>,
    papers: Vec<ConnectedPapersPaper>,
}

#[derive(Debug, Deserialize)]
struct ConnectedPapersPaper {
    id: String,
    doi: Option<String>,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    year: Option<String>,
    authors: Vec<ConnectedPapersAuthor>,
}

#[derive(Debug, Deserialize)]
struct ConnectedPapersAuthor {
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = ConnectedPapersSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = ConnectedPapersSource::new().unwrap();
        assert_eq!(source.id(), "connected_papers");
        assert_eq!(source.name(), "Connected Papers");
    }

    #[test]
    fn test_capabilities() {
        let source = ConnectedPapersSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::CITATIONS));
        assert!(caps.contains(SourceCapabilities::DOI_LOOKUP));
        assert_eq!(
            caps,
            SourceCapabilities::SEARCH
                | SourceCapabilities::CITATIONS
                | SourceCapabilities::DOI_LOOKUP
        );
    }

    #[test]
    fn test_response_parsing_from_mock_json() {
        let json = r#"{
            "total_results": 1,
            "papers": [{
                "id": "cp-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "year": "2024",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
            }]
        }"#;
        let response: ConnectedPapersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.total_results, Some(1));
        assert_eq!(response.papers.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = ConnectedPapersSource::new().unwrap();
        let json = r#"{
            "total_results": 1,
            "papers": [{
                "id": "cp-1",
                "doi": "10.1234/mock",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "year": "2024",
                "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}]
            }]
        }"#;
        let response: ConnectedPapersResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response.papers[0]).unwrap();
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.source, crate::models::SourceType::ConnectedPapers);
        assert_eq!(paper.paper_id, "10.1234/mock");
        assert_eq!(paper.url, "https://doi.org/10.1234/mock");
        assert_eq!(paper.published_date.as_deref(), Some("2024"));
    }
}
