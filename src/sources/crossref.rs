//! CrossRef research source implementation.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const CROSSREF_API_BASE: &str = "https://api.crossref.org";

/// CrossRef research source
///
/// Uses CrossRef REST API for DOI metadata lookup and search.
#[derive(Debug, Clone)]
pub struct CrossRefSource {
    client: Arc<HttpClient>,
}

impl CrossRefSource {
    pub fn new() -> Result<Self, SourceError> {
        let user_agent = format!(
            "{} / {} (mailto:crossref@crossref.org)",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        );
        Ok(Self {
            client: Arc::new(HttpClient::with_user_agent(&user_agent)?),
        })
    }
}

impl Default for CrossRefSource {
    fn default() -> Self {
        Self::new().expect("Failed to create CrossRefSource")
    }
}

#[async_trait]
impl Source for CrossRefSource {
    fn id(&self) -> &str {
        "crossref"
    }

    fn name(&self) -> &str {
        "CrossRef"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let mut url = format!(
            "{}/works?query={}&rows={}",
            CROSSREF_API_BASE,
            urlencoding::encode(&query.query),
            query.max_results
        );

        // Add year filter if specified
        if let Some(year) = &query.year {
            url = format!("{}&filter=from-pub-date-year:{}", url, year);
        }

        // Clone values for retry closure
        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let response = client.get(&url).send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to search CrossRef: {}", e))
                })?;

                if !response.status().is_success() {
                    // Handle rate limiting
                    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        tracing::debug!("CrossRef API rate-limited - returning empty results");
                        return Err(SourceError::Api("CrossRef rate-limited".to_string()));
                    }
                    return Err(SourceError::Api(format!(
                        "CrossRef API returned status: {}",
                        response.status()
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
                        "CrossRef returned non-JSON content-type: {} - rate-limited?",
                        content_type
                    );
                    return Err(SourceError::Api("CrossRef rate-limited".to_string()));
                }

                Ok(response)
            }
        })
        .await;

        // Handle rate limiting gracefully
        let response = match response {
            Ok(r) => r,
            Err(SourceError::Api(msg)) if msg.contains("rate-limited") => {
                tracing::debug!("CrossRef rate-limited - returning empty results");
                return Ok(SearchResponse::new(vec![], "CrossRef", &query.query));
            }
            Err(e) => return Err(e),
        };

        // Capture response text for better error messages
        let response_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read response body".to_string());

        let data: CRResponse = serde_json::from_str(&response_text).map_err(|e| {
            let preview = response_text.chars().take(500).collect::<String>();
            tracing::warn!("CrossRef parse error: {}", preview);
            SourceError::Parse(format!(
                "Failed to parse JSON: {}. Response: {}",
                e, preview
            ))
        })?;

        let papers: Vec<Paper> = data
            .message
            .items
            .into_iter()
            .filter_map(|item| {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                let authors = item
                    .get("author")
                    .and_then(|v| v.as_array())
                    .map(|authors| {
                        authors
                            .iter()
                            .filter_map(|a| a.get("given").and_then(|g| g.as_str()))
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
                    .unwrap_or_default();

                let doi = item
                    .get("doi")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                let url = item
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                let published_date = item
                    .get("published-print")
                    .and_then(|v| v.get("date-parts"))
                    .and_then(|v| v.as_array())
                    .and_then(|dates| dates.first())
                    .and_then(|d| d.as_i64())
                    .map(|y| y.to_string())
                    .unwrap_or_default();

                if title.is_empty() {
                    return None;
                }

                Some(
                    PaperBuilder::new(doi.clone(), title, url, SourceType::CrossRef)
                        .authors(&authors)
                        .doi(&doi)
                        .published_date(&published_date)
                        .build(),
                )
            })
            .collect();

        Ok(SearchResponse::new(papers, "CrossRef", &query.query)
            .total_results(data.message.total_results))
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let url = format!("{}/works/{}", CROSSREF_API_BASE, urlencoding::encode(doi));

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch DOI: {}", e)))?;

        if !response.status().is_success() {
            return Err(SourceError::NotFound("DOI not found".to_string()));
        }

        // Capture response text for better error messages
        let response_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read response body".to_string());

        let data: CRResponse = serde_json::from_str(&response_text).map_err(|e| {
            let preview = response_text.chars().take(500).collect::<String>();
            tracing::warn!("CrossRef DOI parse error: {}", preview);
            SourceError::Parse(format!(
                "Failed to parse JSON: {}. Response: {}",
                e, preview
            ))
        })?;

        let item = data
            .message
            .items
            .first()
            .ok_or_else(|| SourceError::NotFound("DOI not found".to_string()))?;

        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let authors = item
            .get("author")
            .and_then(|v| v.as_array())
            .map(|authors| {
                authors
                    .iter()
                    .filter_map(|a| a.get("given").and_then(|g| g.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();

        let doi = item
            .get("doi")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let url = item
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let published_date = item
            .get("published-print")
            .and_then(|v| v.get("date-parts"))
            .and_then(|v| v.as_array())
            .and_then(|dates| dates.first())
            .and_then(|d| d.as_i64())
            .map(|y| y.to_string())
            .unwrap_or_default();

        Ok(
            PaperBuilder::new(doi.clone(), title, url, SourceType::CrossRef)
                .authors(&authors)
                .doi(&doi)
                .published_date(&published_date)
                .build(),
        )
    }
}

// ===== CrossRef API Types =====

#[derive(Debug, Deserialize)]
struct CRResponse {
    message: CRMessage,
}

#[derive(Debug, Deserialize)]
struct CRMessage {
    #[serde(rename = "total-results")]
    total_results: usize,
    items: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CRDate {
    _date: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crossref_source_creation() {
        let source = CrossRefSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_crossref_capabilities() {
        let source = CrossRefSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::DOI_LOOKUP));
    }

    #[test]
    fn test_crossref_id() {
        let source = CrossRefSource::new().unwrap();
        assert_eq!(source.id(), "crossref");
    }

    #[test]
    fn test_crossref_name() {
        let source = CrossRefSource::new().unwrap();
        assert_eq!(source.name(), "CrossRef");
    }
}
