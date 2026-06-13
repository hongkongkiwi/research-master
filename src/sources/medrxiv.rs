//! medRxiv research source implementation.
//!
//! Uses the medRxiv API for searching and downloading preprints.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{DownloadRequest, DownloadResult, Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const MEDRXIV_API_URL: &str = "https://api.medrxiv.org";
const MEDRXIV_SERVER: &str = "medrxiv";
const MEDRXIV_NAME: &str = "medRxiv";

/// medRxiv research source
#[derive(Debug, Clone)]
pub struct MedrxivSource {
    client: Arc<HttpClient>,
}

impl MedrxivSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }

    /// Get papers from medRxiv (cursor-based pagination)
    async fn get_papers(
        &self,
        cursor: &str,
        _max_results: usize,
    ) -> Result<Vec<Paper>, SourceError> {
        // Use a date range that covers recent papers (last 5 years to today)
        let today = chrono::Local::now();
        let five_years_ago = today - chrono::Duration::days(365 * 5);
        let from_date = five_years_ago.format("%Y-%m-%d").to_string();
        let to_date = today.format("%Y-%m-%d").to_string();

        let url = format!(
            "{}/details/{}/{}/{}/{}",
            MEDRXIV_API_URL, MEDRXIV_SERVER, from_date, to_date, cursor
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let response = client.get(&url).send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to fetch from {}: {}", MEDRXIV_NAME, e))
                })?;

                if !response.status().is_success() {
                    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        tracing::debug!("{} rate-limited - returning empty results", MEDRXIV_NAME);
                        return Err(SourceError::Api("medRxiv rate-limited".to_string()));
                    }
                    return Err(SourceError::Api(format!(
                        "{} API returned status: {}",
                        MEDRXIV_NAME,
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
                        "{} returned non-JSON content-type: {} - rate-limited?",
                        MEDRXIV_NAME,
                        content_type
                    );
                    return Err(SourceError::Api("medRxiv rate-limited".to_string()));
                }

                Ok(response)
            }
        })
        .await;

        // Handle rate limiting gracefully
        let response = match response {
            Ok(r) => r,
            Err(SourceError::Api(msg)) if msg.contains("rate-limited") => {
                tracing::debug!("medRxiv rate-limited - returning empty results");
                return Ok(vec![]);
            }
            Err(e) => return Err(e),
        };

        let json: ApiResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let mut papers = Vec::new();

        for paper in json.collection {
            let authors = paper.authors.clone().unwrap_or_default();
            let categories = paper.category.unwrap_or_default().clone();
            let published_date = paper.date.clone();
            let doi = paper.doi.clone().unwrap_or_default();
            let url = paper
                .server_url
                .clone()
                .unwrap_or_else(|| format!("https://doi.org/{}", doi));

            papers.push(
                PaperBuilder::new(doi.clone(), paper.title, url, SourceType::MedRxiv)
                    .authors(authors)
                    .abstract_text(paper.r#abstract.unwrap_or_default())
                    .doi(doi.clone())
                    .published_date(published_date)
                    .categories(categories)
                    .pdf_url(Self::pdf_url(&doi))
                    .build(),
            );
        }

        Ok(papers)
    }

    /// Parse a DOI to get the paper ID
    fn parse_doi(&self, doi: &str) -> Result<String, SourceError> {
        // medRxiv DOIs look like: 10.1101/2023.123.456789
        let trimmed = doi.trim();

        if trimmed.is_empty() {
            return Err(SourceError::InvalidRequest("Empty DOI".to_string()));
        }

        Ok(trimmed.to_string())
    }

    fn pdf_url(doi: &str) -> String {
        format!("https://www.medrxiv.org/content/{}.full.pdf", doi)
    }
}

impl Default for MedrxivSource {
    fn default() -> Self {
        Self::new().expect("Failed to create MedrxivSource")
    }
}

#[async_trait]
impl Source for MedrxivSource {
    fn id(&self) -> &str {
        "medrxiv"
    }

    fn name(&self) -> &str {
        MEDRXIV_NAME
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOWNLOAD
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let mut cursor = "0".to_string();
        let mut all_papers = Vec::new();

        // medRxiv API is cursor-based, fetch until we have enough
        while all_papers.len() < query.max_results {
            let remaining = query.max_results - all_papers.len();
            let batch_size = remaining.clamp(10, 100);
            let papers = self.get_papers(&cursor, batch_size).await?;

            if papers.is_empty() {
                break;
            }

            let count = papers.len();
            all_papers.extend(papers);

            // Update cursor (rough estimate - API doesn't return actual cursor)
            cursor = all_papers.len().to_string();

            if count < batch_size {
                // Got less than requested, probably no more results
                break;
            }
        }

        // Filter by query if specified (simple text search)
        let filtered = if !query.query.is_empty() {
            let query_lower = query.query.to_lowercase();
            all_papers
                .into_iter()
                .filter(|p| {
                    p.title.to_lowercase().contains(&query_lower)
                        || p.r#abstract.to_lowercase().contains(&query_lower)
                        || p.authors.to_lowercase().contains(&query_lower)
                })
                .collect()
        } else {
            all_papers
        };

        let papers = filtered.into_iter().take(query.max_results).collect();

        Ok(SearchResponse::new(papers, MEDRXIV_NAME, &query.query))
    }

    async fn download(&self, request: &DownloadRequest) -> Result<DownloadResult, SourceError> {
        let doi = self.parse_doi(&request.paper_id)?;
        let pdf_url = Self::pdf_url(&doi);

        let response = self
            .client
            .get(&pdf_url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to download PDF: {}", e)))?;

        if !response.status().is_success() {
            return Err(SourceError::NotFound(format!("Paper not found: {}", doi)));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to read PDF: {}", e)))?;

        // Create download directory if it doesn't exist
        std::fs::create_dir_all(&request.save_path).map_err(|e| {
            SourceError::Io(std::io::Error::other(format!(
                "Failed to create directory: {}",
                e
            )))
        })?;

        let filename = format!("{}.pdf", doi.replace('/', "_"));
        let path = std::path::Path::new(&request.save_path).join(&filename);

        std::fs::write(&path, bytes.as_ref()).map_err(SourceError::Io)?;

        Ok(DownloadResult::success(
            path.to_string_lossy().to_string(),
            bytes.len() as u64,
        ))
    }
}

/// API response structure for medRxiv
#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[serde(rename = "collection")]
    collection: Vec<Article>,
    #[allow(dead_code)]
    #[serde(default)]
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct Article {
    #[serde(default)]
    title: String,
    #[serde(default)]
    authors: Option<String>,
    #[serde(default)]
    r#abstract: Option<String>,
    #[serde(default)]
    date: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    doi: Option<String>,
    #[serde(default)]
    server_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Message {
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_medrxiv_source_creation() {
        let source = MedrxivSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_medrxiv_capabilities() {
        let source = MedrxivSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::DOWNLOAD));
        assert!(!caps.contains(SourceCapabilities::READ));
    }
}
