//! Dimensions research source implementation.
//!
//! Uses the Dimensions API for comprehensive research paper discovery.
//! API documentation: <https://docs.dimensions.ai>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{CitationRequest, Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const DIMENSIONS_API_BASE: &str = "https://api.dimensions.ai/graphql";

/// Dimensions research source
///
/// Uses the Dimensions API for searching and retrieving research papers.
/// API works without a key but has lower rate limits.
#[derive(Debug, Clone)]
pub struct DimensionsSource {
    client: Arc<HttpClient>,
    api_key: Option<String>,
}

impl DimensionsSource {
    pub fn new() -> Result<Self, SourceError> {
        let api_key = std::env::var("DIMENSIONS_API_KEY").ok();
        // Use 90s timeout for GraphQL queries that may take longer
        // User agent respects RESEARCH_MASTER_USER_AGENT env var
        Ok(Self {
            client: Arc::new(HttpClient::with_timeout(
                &crate::utils::get_user_agent(),
                90,
            )?),
            api_key,
        })
    }
}

impl Default for DimensionsSource {
    fn default() -> Self {
        Self::new().expect("Failed to create DimensionsSource")
    }
}

#[async_trait]
impl Source for DimensionsSource {
    fn id(&self) -> &str {
        "dimensions"
    }

    fn name(&self) -> &str {
        "Dimensions"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::CITATIONS | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let search_query = format!(
            r#"
            {{
                search(query: "{}", limit: {}) {{
                    id
                    title
                    abstract
                    authors {{
                        first_name
                        last_name
                    }}
                    publication_year
                    journal {{
                        title
                    }}
                    doi
                    type
                    concepts {{
                        name
                    }}
                    related_works {{
                        doi
                        title
                    }}
                }}
            }}
            "#,
            query.query.replace("\"", "\\\""),
            query.max_results.min(100)
        );

        let client = Arc::clone(&self.client);
        let query_for_retry = search_query.clone();
        let api_key = self.api_key.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let query = query_for_retry.clone();
            let api_key = api_key.clone();
            async move {
                let mut request = client.post(DIMENSIONS_API_BASE).json(&serde_json::json!({
                    "query": query
                }));

                if let Some(ref key) = api_key {
                    request = request.header("Authorization", format!("JWT {}", key));
                }

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to search Dimensions: {}", e))
                })?;

                let status = response.status();
                if !status.is_success() {
                    // Dimensions may return network errors or blocking
                    // Return empty results gracefully
                    if status == reqwest::StatusCode::FORBIDDEN
                        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        tracing::debug!("Dimensions API blocked or rate-limited - skipping");
                        return Err(SourceError::Api("Dimensions blocked".to_string()));
                    }
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "Dimensions API returned status {}: {}",
                        status, text
                    )));
                }

                let json: DimensionsResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse Dimensions response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await;

        // Handle API blocking gracefully
        let response = match response {
            Ok(r) => r,
            Err(SourceError::Api(msg)) if msg.contains("blocked") => {
                tracing::debug!("Dimensions API blocked - returning empty results");
                return Ok(SearchResponse::new(vec![], "Dimensions", &query.query));
            }
            Err(SourceError::Network(_)) => {
                tracing::debug!("Dimensions network error - returning empty results");
                return Ok(SearchResponse::new(vec![], "Dimensions", &query.query));
            }
            Err(e) => return Err(e),
        };

        let papers: Result<Vec<Paper>, SourceError> = response
            .data
            .search
            .into_iter()
            .map(|item| self.parse_result(&item))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "Dimensions", &query.query);
        response.total_results = Some(response.papers.len());
        Ok(response)
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let query = format!(
            r#"
            {{
                papers(ids: ["{}"]) {{
                    id
                    title
                    abstract
                    authors {{
                        first_name
                        last_name
                    }}
                    publication_year
                    journal {{
                        title
                    }}
                    doi
                    type
                    concepts {{
                        name
                    }}
                }}
            }}
            "#,
            clean_doi
        );

        let client = Arc::clone(&self.client);
        let query_for_retry = query.clone();
        let api_key = self.api_key.clone();

        let response: DimensionsResponse = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let query = query_for_retry.clone();
            let api_key = api_key.clone();
            async move {
                let mut request = client.post(DIMENSIONS_API_BASE).json(&serde_json::json!({
                    "query": query
                }));

                if let Some(ref key) = api_key {
                    request = request.header("Authorization", format!("JWT {}", key));
                }

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in Dimensions: {}", e))
                })?;

                let status = response.status();
                if !status.is_success() {
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "Dimensions API returned status {}: {}",
                        status, text
                    )));
                }

                response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse Dimensions response: {}", e))
                })
            }
        })
        .await?;

        if let Some(paper) = response.data.papers.into_iter().next() {
            self.parse_result(&paper)
        } else {
            Err(SourceError::NotFound(format!(
                "Paper not found in Dimensions: {}",
                doi
            )))
        }
    }

    async fn get_citations(
        &self,
        _request: &CitationRequest,
    ) -> Result<SearchResponse, SourceError> {
        Err(SourceError::Other(
            "Citations via Dimensions require DOI lookup. Use get_by_doi first.".to_string(),
        ))
    }
}

impl DimensionsSource {
    fn parse_result(&self, item: &DimensionsPaper) -> Result<Paper, SourceError> {
        let authors: String = item
            .authors
            .iter()
            .map(|a| {
                let first = a.first_name.as_deref().unwrap_or("");
                let last = a.last_name.as_deref().unwrap_or("");
                let name = format!("{} {}", first, last);
                name.trim().to_string()
            })
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join("; ");

        let title = item.title.clone().unwrap_or_default();
        let abstract_text = item.abstract_text.clone().unwrap_or_default();
        let doi = item.doi.clone().unwrap_or_default();
        let year = item
            .publication_year
            .map(|y| y.to_string())
            .unwrap_or_default();

        let url = if !doi.is_empty() {
            format!("https://doi.org/{}", doi)
        } else {
            format!("https://app.dimensions.ai/details/{}", item.id)
        };

        Ok(
            PaperBuilder::new(item.id.clone(), title, url, SourceType::Dimensions)
                .authors(&authors)
                .published_date(&year)
                .abstract_text(&abstract_text)
                .doi(&doi)
                .build(),
        )
    }
}

/// Dimensions API response
#[derive(Debug, Deserialize)]
struct DimensionsResponse {
    data: DimensionsData,
}

#[derive(Debug, Deserialize)]
struct DimensionsData {
    search: Vec<DimensionsPaper>,
    papers: Vec<DimensionsPaper>,
}

/// Common paper structure for both search and DOI lookup results
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DimensionsPaper {
    id: String,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    authors: Vec<DimensionsAuthor>,
    publication_year: Option<i32>,
    journal: Option<DimensionsJournal>,
    doi: Option<String>,
    r#type: Option<String>,
    concepts: Vec<DimensionsConcept>,
    #[serde(default)]
    related_works: Option<Vec<DimensionsRelatedWork>>,
}

#[derive(Debug, Deserialize)]
struct DimensionsAuthor {
    first_name: Option<String>,
    last_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DimensionsJournal {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DimensionsConcept {
    name: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DimensionsRelatedWork {
    doi: Option<String>,
    title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = DimensionsSource::new();
        assert!(source.is_ok());
    }
}
