//! IEEE Xplore research source implementation.
//!
//! Uses the IEEE Xplore API for searching and retrieving research papers.
//! API documentation: <https://developer.ieee.org/>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const IEEE_XPLORE_API_BASE: &str = "https://ieeexploreapi.ieee.org/api/v1/search/articles";

/// IEEE Xplore research source
///
/// Uses the IEEE Xplore API for searching and retrieving research papers.
/// API requires a free API key from https://developer.ieee.org/
#[derive(Debug, Clone)]
pub struct IeeeXploreSource {
    client: Arc<HttpClient>,
    api_key: Option<String>,
}

impl IeeeXploreSource {
    pub fn new() -> Result<Self, SourceError> {
        let api_key = std::env::var("IEEE_XPLORE_API_KEY").ok();
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            api_key,
        })
    }
}

impl Default for IeeeXploreSource {
    fn default() -> Self {
        Self::new().expect("Failed to create IeeeXploreSource")
    }
}

#[async_trait]
impl Source for IeeeXploreSource {
    fn id(&self) -> &str {
        "ieee_xplore"
    }

    fn name(&self) -> &str {
        "IEEE Xplore"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        if self.api_key.is_none() {
            tracing::debug!("IEEE_XPLORE_API_KEY not set - skipping search");
            return Ok(SearchResponse::new(Vec::new(), "IEEE Xplore", &query.query));
        }

        let max_results = query.max_results.min(100).min(100);
        let mut url = format!(
            "{}?max_records={}&apikey={}",
            IEEE_XPLORE_API_BASE,
            max_results,
            self.api_key.as_deref().unwrap_or_default()
        );

        url.push_str(&format!(
            "&article_number={}",
            urlencoding::encode(&query.query)
        ));

        if let Some(year_filter) = &query.year {
            url.push_str(&format!("&publication_year={}", year_filter));
        }

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url);

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to search IEEE Xplore: {}", e))
                })?;

                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "IEEE Xplore API returned status {}: {}",
                        status, text
                    )));
                }

                let json: IeeeXploreResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse IEEE Xplore response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        let total = response.total_records.unwrap_or(0);
        let papers: Result<Vec<Paper>, SourceError> = response
            .articles
            .into_iter()
            .map(|item| self.parse_result(&item))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "IEEE Xplore", &query.query);
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
            "{}?article_number={}&apikey={}",
            IEEE_XPLORE_API_BASE,
            urlencoding::encode(&clean_doi),
            self.api_key.as_deref().unwrap_or_default()
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response: IeeeXploreResponse = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url);

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in IEEE Xplore: {}", e))
                })?;

                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "IEEE Xplore API returned status {}: {}",
                        status, text
                    )));
                }

                response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse IEEE Xplore response: {}", e))
                })
            }
        })
        .await?;

        if let Some(article) = response.articles.into_iter().next() {
            self.parse_result(&article)
        } else {
            Err(SourceError::NotFound(format!(
                "Paper not found in IEEE Xplore: {}",
                doi
            )))
        }
    }
}

impl IeeeXploreSource {
    fn parse_result(&self, item: &IeeeXploreArticle) -> Result<Paper, SourceError> {
        let id = item.article_number.clone();
        let title = item.title.clone().unwrap_or_default();
        let abstract_text = item.abstract_text.clone().unwrap_or_default();
        let doi = item.doi.clone().unwrap_or_default();

        let authors: String = item
            .authors
            .iter()
            .filter_map(|a| a.full_name.clone())
            .collect::<Vec<_>>()
            .join("; ");

        let year = item.publication_date.clone().unwrap_or_default();
        let url = if !doi.is_empty() {
            format!("https://doi.org/{}", doi)
        } else {
            format!("https://ieeexplore.ieee.org/document/{}", id)
        };

        Ok(PaperBuilder::new(id, title, url, SourceType::IeeeXplore)
            .authors(&authors)
            .published_date(&year)
            .abstract_text(&abstract_text)
            .doi(&doi)
            .build())
    }
}

/// IEEE Xplore API response
#[derive(Debug, Deserialize)]
struct IeeeXploreResponse {
    total_records: Option<usize>,
    articles: Vec<IeeeXploreArticle>,
}

#[derive(Debug, Deserialize)]
struct IeeeXploreArticle {
    article_number: String,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    doi: Option<String>,
    publication_date: Option<String>,
    authors: Vec<IeeeXploreAuthor>,
}

#[derive(Debug, Deserialize)]
struct IeeeXploreAuthor {
    full_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = IeeeXploreSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = IeeeXploreSource::new().unwrap();
        assert_eq!(source.id(), "ieee_xplore");
        assert_eq!(source.name(), "IEEE Xplore");
    }

    #[test]
    fn test_capabilities() {
        let source = IeeeXploreSource::new().unwrap();
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
            "total_records": 1,
            "articles": [{
                "article_number": "9876543",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "doi": "10.1234/mock",
                "publication_date": "2024",
                "authors": [{"full_name": "Ada Lovelace"}, {"full_name": "Alan Turing"}]
            }]
        }"#;
        let response: IeeeXploreResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.total_records, Some(1));
        assert_eq!(response.articles.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = IeeeXploreSource::new().unwrap();
        let json = r#"{
            "total_records": 1,
            "articles": [{
                "article_number": "9876543",
                "title": "Mock Paper Title",
                "abstract": "Mock abstract text.",
                "doi": "10.1234/mock",
                "publication_date": "2024",
                "authors": [{"full_name": "Ada Lovelace"}, {"full_name": "Alan Turing"}]
            }]
        }"#;
        let response: IeeeXploreResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response.articles[0]).unwrap();
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.source, crate::models::SourceType::IeeeXplore);
        assert_eq!(paper.paper_id, "9876543");
        assert_eq!(paper.url, "https://doi.org/10.1234/mock");
        assert_eq!(paper.published_date.as_deref(), Some("2024"));
    }
}
