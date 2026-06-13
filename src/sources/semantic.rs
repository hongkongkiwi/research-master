//! Semantic Scholar research source implementation.

use async_trait::async_trait;
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{
    CitationRequest, DownloadRequest, DownloadResult, ReadRequest, ReadResult, Source,
    SourceCapabilities, SourceError,
};
use crate::utils::{
    api_retry_config, with_retry, CircuitBreaker, HttpClient, RateLimitedRequestBuilder,
};

const SEMANTIC_API_BASE: &str = "https://api.semanticscholar.org/graph/v1";

/// Environment variable for Semantic Scholar rate limit (requests per second)
const SEMANTIC_SCHOLAR_RATE_LIMIT_ENV: &str = "SEMANTIC_SCHOLAR_RATE_LIMIT";

/// Default rate limit for Semantic Scholar (1 request per second without API key)
const DEFAULT_SEMANTIC_RATE_LIMIT: u32 = 1;

/// Environment variable for Semantic Scholar circuit breaker failure threshold
const SEMANTIC_SCHOLAR_CIRCUIT_BREAKER_THRESHOLD_ENV: &str =
    "SEMANTIC_SCHOLAR_CIRCUIT_BREAKER_THRESHOLD";

/// Default circuit breaker failure threshold for Semantic Scholar
const DEFAULT_SEMANTIC_CIRCUIT_BREAKER_THRESHOLD: usize = 10;

/// Semantic Scholar research source
///
/// Uses Semantic Scholar GraphQL/REST API.
#[derive(Debug, Clone)]
pub struct SemanticScholarSource {
    client: Arc<HttpClient>,
    api_key: Option<String>,
    /// Circuit breaker for handling transient failures
    circuit_breaker: Arc<CircuitBreaker>,
}

impl SemanticScholarSource {
    /// Get the rate limit from environment variable or use default
    fn get_rate_limit() -> u32 {
        std::env::var(SEMANTIC_SCHOLAR_RATE_LIMIT_ENV)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(DEFAULT_SEMANTIC_RATE_LIMIT)
    }

    /// Get the circuit breaker failure threshold from environment variable or use default
    fn get_circuit_breaker_threshold() -> usize {
        std::env::var(SEMANTIC_SCHOLAR_CIRCUIT_BREAKER_THRESHOLD_ENV)
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(DEFAULT_SEMANTIC_CIRCUIT_BREAKER_THRESHOLD)
    }

    /// Create a new Semantic Scholar source
    pub fn new() -> Result<Self, SourceError> {
        let rate_limit = Self::get_rate_limit();
        let user_agent = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
        let circuit_threshold = Self::get_circuit_breaker_threshold();

        Ok(Self {
            client: Arc::new(HttpClient::with_rate_limit(user_agent, rate_limit)?),
            api_key: std::env::var("SEMANTIC_SCHOLAR_API_KEY").ok(),
            circuit_breaker: Arc::new(CircuitBreaker::new(
                "semantic",
                circuit_threshold,
                std::time::Duration::from_secs(60),
            )),
        })
    }

    /// Create with an API key (optional, for higher rate limits)
    #[allow(dead_code)]
    pub fn with_api_key(api_key: String) -> Result<Self, SourceError> {
        let rate_limit = Self::get_rate_limit();
        let user_agent = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
        let circuit_threshold = Self::get_circuit_breaker_threshold();

        Ok(Self {
            client: Arc::new(HttpClient::with_rate_limit(user_agent, rate_limit)?),
            api_key: Some(api_key),
            circuit_breaker: Arc::new(CircuitBreaker::new(
                "semantic",
                circuit_threshold,
                std::time::Duration::from_secs(60),
            )),
        })
    }

    /// Build request URL with optional API key
    fn build_url(&self, endpoint: &str) -> String {
        format!("{}{}", SEMANTIC_API_BASE, endpoint)
    }

    /// Add API key to request headers if available
    fn add_api_key_if_present(
        &self,
        builder: RateLimitedRequestBuilder,
    ) -> RateLimitedRequestBuilder {
        if let Some(ref key) = self.api_key {
            builder.header("x-api-key", key)
        } else {
            builder
        }
    }

    /// Parse Semantic Scholar paper data
    fn parse_paper(data: &S2Paper) -> Paper {
        let authors = data
            .authors
            .iter()
            .filter_map(|a| a.name.as_ref())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("; ");

        let published_date = data
            .publication_date
            .clone()
            .or_else(|| data.year.as_ref().map(|y| y.to_string()));

        let doi = data
            .doi
            .clone()
            .or_else(|| {
                data.external_ids
                    .as_ref()
                    .and_then(|ids| ids.get("DOI").cloned())
            })
            .unwrap_or_default();

        let url = data.url.clone().unwrap_or_else(|| {
            if !doi.is_empty() {
                format!("https://doi.org/{}", doi)
            } else {
                String::new()
            }
        });

        let paper_id = data
            .paper_id
            .clone()
            .or(data.corpus_id.clone())
            .unwrap_or_default();

        let pdf_url = data
            .open_access_pdf
            .as_ref()
            .and_then(|p| p.url.clone())
            .unwrap_or_default();

        let mut builder = PaperBuilder::new(
            paper_id,
            data.title.clone(),
            url,
            SourceType::SemanticScholar,
        )
        .authors(authors)
        .abstract_text(data.r#abstract.clone().unwrap_or_default())
        .doi(doi)
        .published_date(published_date.unwrap_or_default())
        .pdf_url(pdf_url)
        .citations(data.citation_count.unwrap_or(0) as u32);

        if let Some(venue) = data.venue.as_ref().filter(|venue| !venue.is_empty()) {
            builder = builder.categories(venue.clone());
        }

        builder.build()
    }
}

impl Default for SemanticScholarSource {
    fn default() -> Self {
        Self::new().expect("Failed to create SemanticScholarSource")
    }
}

#[async_trait]
impl Source for SemanticScholarSource {
    fn id(&self) -> &str {
        "semantic"
    }

    fn name(&self) -> &str {
        "Semantic Scholar"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH
            | SourceCapabilities::DOWNLOAD
            | SourceCapabilities::READ
            | SourceCapabilities::CITATIONS
            | SourceCapabilities::DOI_LOOKUP
            | SourceCapabilities::AUTHOR_SEARCH
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let url = format!(
            "/paper/search?query={}&limit={}",
            urlencoding::encode(&query.query),
            query.max_results
        );

        // Check circuit breaker before making request
        if !self.circuit_breaker.can_request() {
            tracing::warn!(
                "Semantic Scholar: circuit is open (too many failures) - skipping request"
            );
            return Ok(SearchResponse::new(
                vec![],
                "Semantic Scholar",
                &query.query,
            ));
        }

        // Clone values for retry closure
        let client = Arc::clone(&self.client);
        let api_key = self.api_key.clone();
        let url_for_retry = url.clone();
        let circuit_breaker = Arc::clone(&self.circuit_breaker);

        let result: Result<S2SearchResponse, SourceError> = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let api_key = api_key.clone();
            let url = url_for_retry.clone();
            let circuit_breaker = Arc::clone(&circuit_breaker);
            async move {
                // Check circuit breaker before each attempt
                if !circuit_breaker.can_request() {
                    return Err(SourceError::Api(
                        "Semantic Scholar circuit is open".to_string(),
                    ));
                }

                let mut request = client.get(&format!("{}{}", SEMANTIC_API_BASE, url));
                if let Some(ref key) = api_key {
                    request = request.header("x-api-key", key);
                }
                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to search Semantic Scholar: {}", e))
                })?;

                if !response.status().is_success() {
                    let status = response.status();
                    // Handle rate limiting
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        circuit_breaker.record_failure();
                        tracing::warn!(
                            "Semantic Scholar API rate-limited - circuit breaker recorded failure"
                        );
                        return Err(SourceError::Api(
                            "Semantic Scholar rate-limited".to_string(),
                        ));
                    }
                    // Check for service unavailable
                    if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                        circuit_breaker.record_failure();
                        tracing::warn!(
                            "Semantic Scholar API unavailable - circuit breaker recorded failure"
                        );
                        return Err(SourceError::Api("Semantic Scholar unavailable".to_string()));
                    }
                    // Record failure for other errors
                    circuit_breaker.record_failure();
                    return Err(SourceError::Api(format!(
                        "Semantic Scholar API returned status: {}",
                        response.status()
                    )));
                }

                // Record success
                circuit_breaker.record_success();

                // Capture response text first for better error messages
                let response_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read response body".to_string());

                serde_json::from_str::<S2SearchResponse>(&response_text).map_err(|e| {
                    // Log the first 500 chars of response for debugging
                    let preview = response_text.chars().take(500).collect::<String>();
                    tracing::warn!(
                        "Semantic Scholar parse error (first 500 chars): {}",
                        preview
                    );
                    SourceError::Parse(format!(
                        "Failed to parse JSON: {}. Response preview: {}",
                        e, preview
                    ))
                })
            }
        })
        .await;

        // Handle rate limiting gracefully
        let data = match result {
            Ok(d) => d,
            Err(SourceError::Api(msg)) if msg.contains("rate-limited") => {
                tracing::warn!("Semantic Scholar: rate-limited - returning empty results");
                return Ok(SearchResponse::new(
                    vec![],
                    "Semantic Scholar",
                    &query.query,
                ));
            }
            Err(SourceError::Api(msg))
                if msg.contains("unavailable") || msg.contains("circuit") =>
            {
                tracing::warn!("Semantic Scholar: unavailable - returning empty results");
                return Ok(SearchResponse::new(
                    vec![],
                    "Semantic Scholar",
                    &query.query,
                ));
            }
            Err(e) => return Err(e),
        };

        let papers: Result<Vec<Paper>, SourceError> = data
            .data
            .into_iter()
            .map(|item| Ok(Self::parse_paper(&item)))
            .collect();

        Ok(SearchResponse::new(
            papers?,
            "Semantic Scholar",
            &query.query,
        ))
    }

    async fn search_by_author(
        &self,
        author: &str,
        max_results: usize,
        year: Option<&str>,
    ) -> Result<SearchResponse, SourceError> {
        // Clone values for retry closure
        let client = Arc::clone(&self.client);
        let api_key = self.api_key.clone();

        // First, search for the author
        let author_url = format!(
            "/author/search?query={}&limit=1",
            urlencoding::encode(author)
        );

        let author_data: AuthorSearchResponse = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let api_key = api_key.clone();
            let url = author_url.clone();
            async move {
                let mut request = client.get(&format!("{}{}", SEMANTIC_API_BASE, url));
                if let Some(ref key) = api_key {
                    request = request.header("x-api-key", key);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to search author: {}", e)))?;

                let response_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read response body".to_string());

                serde_json::from_str::<AuthorSearchResponse>(&response_text).map_err(|e| {
                    let preview = response_text.chars().take(500).collect::<String>();
                    tracing::warn!("Semantic Scholar author search parse error: {}", preview);
                    SourceError::Parse(format!(
                        "Failed to parse JSON: {}. Response: {}",
                        e, preview
                    ))
                })
            }
        })
        .await?;

        let author_id = author_data
            .data
            .first()
            .and_then(|a| a.author_id.clone())
            .ok_or_else(|| SourceError::NotFound("Author not found".to_string()))?;

        // Then get papers by that author
        let mut papers_url = format!("/author/{}/papers?limit={}", author_id, max_results);

        // Add year filter if provided
        if let Some(y) = year {
            papers_url.push_str(&format!("&year={}", y));
        }

        let papers_data: PapersResponse = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let api_key = api_key.clone();
            let url = papers_url.clone();
            async move {
                let mut request = client.get(&format!("{}{}", SEMANTIC_API_BASE, url));
                if let Some(ref key) = api_key {
                    request = request.header("x-api-key", key);
                }
                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to fetch author papers: {}", e))
                })?;

                let response_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read response body".to_string());

                serde_json::from_str::<PapersResponse>(&response_text).map_err(|e| {
                    let preview = response_text.chars().take(500).collect::<String>();
                    tracing::warn!("Semantic Scholar papers parse error: {}", preview);
                    SourceError::Parse(format!(
                        "Failed to parse JSON: {}. Response: {}",
                        e, preview
                    ))
                })
            }
        })
        .await?;

        let papers: Result<Vec<Paper>, SourceError> = papers_data
            .data
            .into_iter()
            .map(|item| Ok(Self::parse_paper(&item)))
            .collect();

        Ok(SearchResponse::new(papers?, "Semantic Scholar", author))
    }

    async fn download(&self, request: &DownloadRequest) -> Result<DownloadResult, SourceError> {
        // Try to get the paper first to find PDF URL
        let url = format!("/paper/{}", urlencoding::encode(&request.paper_id));

        let response = self
            .add_api_key_if_present(self.client.get(&self.build_url(&url)))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch paper: {}", e)))?;

        if !response.status().is_success() {
            return Err(SourceError::NotFound(format!(
                "Paper not found: {}",
                request.paper_id
            )));
        }

        let response_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read response body".to_string());

        let data: PaperResponse = serde_json::from_str(&response_text).map_err(|e| {
            let preview = response_text.chars().take(500).collect::<String>();
            tracing::warn!("Semantic Scholar paper details parse error: {}", preview);
            SourceError::Parse(format!(
                "Failed to parse JSON: {}. Response: {}",
                e, preview
            ))
        })?;

        let pdf_url = data
            .data
            .open_access_pdf
            .and_then(|p| p.url.clone())
            .ok_or_else(|| SourceError::NotFound("No PDF available".to_string()))?;

        let pdf_response = self
            .client
            .get(&pdf_url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to download PDF: {}", e)))?;

        if !pdf_response.status().is_success() {
            return Err(SourceError::NotFound("PDF not available".to_string()));
        }

        let bytes = pdf_response
            .bytes()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to read PDF: {}", e)))?;

        std::fs::create_dir_all(&request.save_path).map_err(|e| {
            SourceError::Io(std::io::Error::other(format!(
                "Failed to create directory: {}",
                e
            )))
        })?;

        let filename = format!("{}.pdf", request.paper_id.replace('/', "_"));
        let path = std::path::Path::new(&request.save_path).join(&filename);

        std::fs::write(&path, bytes.as_ref()).map_err(SourceError::Io)?;

        Ok(DownloadResult::success(
            path.to_string_lossy().to_string(),
            bytes.len() as u64,
        ))
    }

    async fn read(&self, request: &ReadRequest) -> Result<ReadResult, SourceError> {
        let download_request = DownloadRequest::new(&request.paper_id, &request.save_path);
        let download_result = self.download(&download_request).await?;

        let pdf_path = std::path::Path::new(&download_result.path);
        match crate::utils::extract_text(pdf_path) {
            Ok((text, _method)) => {
                let pages = (text.len() / 3000).max(1);
                Ok(ReadResult::success(text).pages(pages))
            }
            Err(e) => Ok(ReadResult::error(format!(
                "PDF downloaded but text extraction failed: {}",
                e
            ))),
        }
    }

    async fn get_citations(
        &self,
        request: &CitationRequest,
    ) -> Result<SearchResponse, SourceError> {
        let url = format!(
            "/paper/{}/citations?limit={}",
            urlencoding::encode(&request.paper_id),
            request.max_results
        );

        let response = self
            .add_api_key_if_present(self.client.get(&self.build_url(&url)))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch citations: {}", e)))?;

        let response_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read response body".to_string());

        let data: CitationsResponse = serde_json::from_str(&response_text).map_err(|e| {
            let preview = response_text.chars().take(500).collect::<String>();
            tracing::warn!("Semantic Scholar citations parse error: {}", preview);
            SourceError::Parse(format!(
                "Failed to parse JSON: {}. Response: {}",
                e, preview
            ))
        })?;

        let papers: Result<Vec<Paper>, SourceError> = data
            .data
            .into_iter()
            .map(|item| Ok(Self::parse_paper(&item)))
            .collect();

        Ok(SearchResponse::new(
            papers?,
            "Semantic Scholar",
            &request.paper_id,
        ))
    }

    async fn get_references(
        &self,
        request: &CitationRequest,
    ) -> Result<SearchResponse, SourceError> {
        let url = format!(
            "/paper/{}/references?limit={}",
            urlencoding::encode(&request.paper_id),
            request.max_results
        );

        let response = self
            .add_api_key_if_present(self.client.get(&self.build_url(&url)))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch references: {}", e)))?;

        let response_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read response body".to_string());

        let data: ReferencesResponse = serde_json::from_str(&response_text).map_err(|e| {
            let preview = response_text.chars().take(500).collect::<String>();
            tracing::warn!("Semantic Scholar references parse error: {}", preview);
            SourceError::Parse(format!(
                "Failed to parse JSON: {}. Response: {}",
                e, preview
            ))
        })?;

        let papers: Result<Vec<Paper>, SourceError> = data
            .data
            .into_iter()
            .map(|item| Ok(Self::parse_paper(&item)))
            .collect();

        Ok(SearchResponse::new(
            papers?,
            "Semantic Scholar",
            &request.paper_id,
        ))
    }

    async fn get_related(&self, request: &CitationRequest) -> Result<SearchResponse, SourceError> {
        let url = format!(
            "/paper/{}/related?limit={}",
            urlencoding::encode(&request.paper_id),
            request.max_results
        );

        let response = self
            .add_api_key_if_present(self.client.get(&self.build_url(&url)))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch related: {}", e)))?;

        let response_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read response body".to_string());

        let data: RelatedResponse = serde_json::from_str(&response_text).map_err(|e| {
            let preview = response_text.chars().take(500).collect::<String>();
            tracing::warn!("Semantic Scholar related parse error: {}", preview);
            SourceError::Parse(format!(
                "Failed to parse JSON: {}. Response: {}",
                e, preview
            ))
        })?;

        let papers: Result<Vec<Paper>, SourceError> = data
            .data
            .into_iter()
            .map(|item| Ok(Self::parse_paper(&item)))
            .collect();

        Ok(SearchResponse::new(
            papers?,
            "Semantic Scholar",
            &request.paper_id,
        ))
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        // Search by DOI using the search API
        let url = format!("/paper/search?query={}&limit=1", urlencoding::encode(doi));

        let response = self
            .add_api_key_if_present(self.client.get(&self.build_url(&url)))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to search DOI: {}", e)))?;

        let response_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read response body".to_string());

        let data: S2SearchResponse = serde_json::from_str(&response_text).map_err(|e| {
            let preview = response_text.chars().take(500).collect::<String>();
            tracing::warn!("Semantic Scholar DOI parse error: {}", preview);
            SourceError::Parse(format!(
                "Failed to parse JSON: {}. Response: {}",
                e, preview
            ))
        })?;

        data.data
            .first()
            .map(Self::parse_paper)
            .ok_or_else(|| SourceError::NotFound("DOI not found".to_string()))
    }

    async fn get_by_id(&self, id: &str) -> Result<Paper, SourceError> {
        let url = format!(
            "/paper/{}?fields=title,url,abstract,authors,year,externalIds,openAccessPdf,citationCount,publicationDate,venue",
            urlencoding::encode(id)
        );

        let data: PaperResponse = with_retry(api_retry_config(), || {
            let url = url.clone();
            async move {
                let response = self
                    .add_api_key_if_present(self.client.get(&self.build_url(&url)))
                    .send()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to fetch paper: {}", e)))?;

                if !response.status().is_success() {
                    return Err(SourceError::NotFound(format!("Paper not found: {}", id)));
                }

                let response_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Failed to read response body".to_string());

                serde_json::from_str::<PaperResponse>(&response_text).map_err(|e| {
                    let preview = response_text.chars().take(500).collect::<String>();
                    tracing::warn!("Semantic Scholar paper details parse error: {}", preview);
                    SourceError::Parse(format!(
                        "Failed to parse JSON: {}. Response: {}",
                        e, preview
                    ))
                })
            }
        })
        .await?;

        let mut paper = Self::parse_paper(&data.data);
        if paper.paper_id.is_empty() {
            paper.paper_id = id.to_string();
        }

        Ok(paper)
    }
}

// ===== Semantic Scholar API Types =====

#[derive(Debug, Deserialize)]
struct S2Paper {
    #[serde(rename = "paperId")]
    paper_id: Option<String>,
    #[serde(rename = "corpusId")]
    corpus_id: Option<String>,
    title: String,
    r#abstract: Option<String>,
    year: Option<i32>,
    #[serde(rename = "citationCount")]
    citation_count: Option<i32>,
    #[serde(default)]
    authors: Vec<S2Author>,
    doi: Option<String>,
    #[serde(rename = "externalIds")]
    external_ids: Option<HashMap<String, String>>,
    url: Option<String>,
    #[serde(rename = "openAccessPdf")]
    open_access_pdf: Option<S2OpenAccessPdf>,
    #[serde(rename = "publicationDate")]
    publication_date: Option<String>,
    venue: Option<String>,
}

#[derive(Debug, Deserialize)]
struct S2OpenAccessPdf {
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct S2Author {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct S2AuthorData {
    #[serde(rename = "authorId")]
    author_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct S2SearchResponse {
    data: Vec<S2Paper>,
}

#[derive(Debug, Deserialize)]
struct AuthorSearchResponse {
    data: Vec<S2AuthorData>,
}

#[derive(Debug, Deserialize)]
struct PapersResponse {
    data: Vec<S2Paper>,
}

#[derive(Debug)]
struct PaperResponse {
    data: S2Paper,
}

impl<'de> Deserialize<'de> for PaperResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WrappedPaperResponse {
            data: S2Paper,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum PaperResponseHelper {
            Wrapped(WrappedPaperResponse),
            Direct(S2Paper),
        }

        match PaperResponseHelper::deserialize(deserializer)? {
            PaperResponseHelper::Wrapped(response) => Ok(Self {
                data: response.data,
            }),
            PaperResponseHelper::Direct(data) => Ok(Self { data }),
        }
    }
}

#[derive(Debug, Deserialize)]
struct CitationsResponse {
    data: Vec<S2Paper>,
}

#[derive(Debug, Deserialize)]
struct ReferencesResponse {
    data: Vec<S2Paper>,
}

#[derive(Debug, Deserialize)]
struct RelatedResponse {
    data: Vec<S2Paper>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_source_creation() {
        let source = SemanticScholarSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_semantic_capabilities() {
        let source = SemanticScholarSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::DOWNLOAD));
        assert!(caps.contains(SourceCapabilities::READ));
        assert!(caps.contains(SourceCapabilities::CITATIONS));
        assert!(caps.contains(SourceCapabilities::DOI_LOOKUP));
        assert!(caps.contains(SourceCapabilities::AUTHOR_SEARCH));
    }

    #[test]
    fn test_semantic_id() {
        let source = SemanticScholarSource::new().unwrap();
        assert_eq!(source.id(), "semantic");
    }

    #[test]
    fn test_semantic_name() {
        let source = SemanticScholarSource::new().unwrap();
        assert_eq!(source.name(), "Semantic Scholar");
    }

    #[test]
    fn test_parse_search_response() {
        // Mock Semantic Scholar search API response
        let mock_response = r#"
        {
            "data": [
                {
                    "paperId": "paper123",
                    "corpusId": "12345",
                    "title": "Machine Learning for Image Recognition",
                    "abstract": "A novel approach to image recognition using deep learning.",
                    "year": 2023,
                    "authors": [{"name": "Jane Smith"}, {"name": "John Doe"}],
                    "url": "https://www.semanticscholar.org/paper/paper123",
                    "citationCount": 42
                }
            ]
        }
        "#;

        // Parse the mock response
        let parse_result: Result<S2SearchResponse, serde_json::Error> =
            serde_json::from_str(mock_response);
        assert!(parse_result.is_ok(), "Mock response should be valid JSON");

        // Verify parsed data
        let response = parse_result.unwrap();
        assert_eq!(response.data.len(), 1);

        let paper = &response.data[0];
        assert_eq!(paper.paper_id, Some("paper123".to_string()));
        assert_eq!(paper.title, "Machine Learning for Image Recognition");
        assert_eq!(paper.year, Some(2023));
        assert_eq!(paper.authors.len(), 2);
    }

    #[test]
    fn test_parse_paper_details() {
        // Mock Semantic Scholar paper details response
        let mock_response = r#"
        {
            "paperId": "abc123",
            "corpusId": "67890",
            "title": "Advances in Natural Language Processing",
            "abstract": "This paper presents new advances in NLP techniques.",
            "year": 2024,
            "authors": [{"name": "Alice Johnson"}, {"name": "Bob Williams"}],
            "url": "https://www.semanticscholar.org/paper/abc123",
            "citationCount": 100
        }
        "#;

        let parse_result: Result<S2Paper, serde_json::Error> = serde_json::from_str(mock_response);
        assert!(parse_result.is_ok(), "Should parse valid JSON");

        let paper = parse_result.unwrap();
        assert_eq!(paper.paper_id, Some("abc123".to_string()));
        assert_eq!(paper.title, "Advances in Natural Language Processing");
        assert_eq!(paper.year, Some(2024));
        assert_eq!(paper.authors.len(), 2);
    }

    #[test]
    fn test_parse_references_response() {
        // Mock references response
        let mock_response = r#"
        {
            "data": [
                {
                    "paperId": "ref1",
                    "corpusId": "11111",
                    "title": "Referenced Paper 1",
                    "year": 2022,
                    "authors": [{"name": "Ref Author"}]
                },
                {
                    "paperId": "ref2",
                    "corpusId": "22222",
                    "title": "Referenced Paper 2",
                    "year": 2021,
                    "authors": [{"name": "Another Author"}]
                }
            ]
        }
        "#;

        let parse_result: Result<ReferencesResponse, serde_json::Error> =
            serde_json::from_str(mock_response);
        assert!(parse_result.is_ok(), "Should parse valid references JSON");

        let refs = parse_result.unwrap();
        assert_eq!(refs.data.len(), 2);
        assert_eq!(refs.data[0].paper_id, Some("ref1".to_string()));
        assert_eq!(refs.data[1].paper_id, Some("ref2".to_string()));
    }
}
