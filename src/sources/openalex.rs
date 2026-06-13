//! OpenAlex research source implementation.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{
    CitationRequest, DownloadRequest, DownloadResult, ReadRequest, ReadResult, Source,
    SourceCapabilities, SourceError,
};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const OPENALEX_API_BASE: &str = "https://api.openalex.org";

/// OpenAlex research source
///
/// Uses the OpenAlex REST API.
#[derive(Debug, Clone)]
pub struct OpenAlexSource {
    client: Arc<HttpClient>,
    email: Option<String>,
}

impl OpenAlexSource {
    /// Create a new OpenAlex source
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            email: std::env::var("OPENALEX_EMAIL").ok(),
        })
    }

    /// Create with an email (recommended for better rate limits)
    #[allow(dead_code)]
    pub fn with_email(email: String) -> Result<Self, SourceError> {
        let user_agent = format!(
            "{}/{} (mailto:{})",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            email
        );
        Ok(Self {
            client: Arc::new(HttpClient::with_user_agent(&user_agent)?),
            email: Some(email),
        })
    }

    /// Build request URL
    fn build_url(&self, endpoint: &str) -> String {
        format!("{}{}", OPENALEX_API_BASE, endpoint)
    }

    /// Add email to request URL if available (for polite pool)
    fn add_email_if_present(&self, url: &str) -> String {
        if let Some(ref email) = self.email {
            format!("{}&mailto={}", url, urlencoding::encode(email))
        } else {
            url.to_string()
        }
    }

    /// Parse OpenAlex paper data
    fn parse_paper(data: &OAPaper) -> Paper {
        let authors = data
            .authorships
            .iter()
            .filter_map(|a| a.author.display_name.as_ref())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("; ");

        let published_date = data.publication_year.as_ref().map(|y| y.to_string());

        let doi = data.doi.clone().unwrap_or_default();

        let url = data.id.clone().unwrap_or_default();

        let paper_id = data
            .id
            .clone()
            .unwrap_or_else(|| format!("OpenAlex:{}", doi));

        let pdf_url = data
            .best_open_access_pdf
            .as_ref()
            .and_then(|p| p.url.clone());

        PaperBuilder::new(paper_id, data.title.clone(), url, SourceType::OpenAlex)
            .authors(authors)
            .abstract_text(data.r#abstract.clone().unwrap_or_default())
            .doi(doi)
            .published_date(published_date.unwrap_or_default())
            .pdf_url(pdf_url.unwrap_or_default())
            .citations(data.cited_by_count.unwrap_or(0) as u32)
            .build()
    }
}

impl Default for OpenAlexSource {
    fn default() -> Self {
        Self::new().expect("Failed to create OpenAlexSource")
    }
}

#[async_trait]
impl Source for OpenAlexSource {
    fn id(&self) -> &str {
        "openalex"
    }

    fn name(&self) -> &str {
        "OpenAlex"
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
        let mut url = format!(
            "/works?search={}&per-page={}",
            urlencoding::encode(&query.query),
            query.max_results
        );

        // Add year filter if specified
        if let Some(year) = &query.year {
            if let Some(start) = year.strip_suffix('-') {
                // From year: from_year onwards
                url = format!("{}&filter=publication_year:{}", url, start);
            } else if let Some(end) = year.strip_prefix('-') {
                // Until year
                url = format!("{}&filter=publication_year:|-{}", url, end);
            } else if year.contains('-') {
                // Range
                url = format!("{}&filter=publication_year:{}", url, year);
            } else if year.len() == 4 {
                // Single year
                url = format!("{}&filter=publication_year:{}", url, year);
            }
        }

        url = self.add_email_if_present(&url);

        // Clone values for retry closure
        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let data: WorksResponse = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let response = client
                    .get(&format!("{}{}", OPENALEX_API_BASE, url))
                    .send()
                    .await
                    .map_err(|e| {
                        SourceError::Network(format!("Failed to search OpenAlex: {}", e))
                    })?;

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "OpenAlex API returned status: {}",
                        response.status()
                    )));
                }

                response
                    .json()
                    .await
                    .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))
            }
        })
        .await?;

        let papers: Result<Vec<Paper>, SourceError> = data
            .results
            .into_iter()
            .map(|p| Ok(Self::parse_paper(&p)))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "OpenAlex", &query.query);
        response.total_results = Some(data.meta.count);
        Ok(response)
    }

    async fn search_by_author(
        &self,
        author: &str,
        max_results: usize,
        year: Option<&str>,
    ) -> Result<SearchResponse, SourceError> {
        let url = format!("/authors?search={}&per-page=1", urlencoding::encode(author));

        let response = self
            .client
            .get(&self.build_url(&url))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to search author: {}", e)))?;

        let author_data: AuthorsResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let author_id = author_data
            .results
            .first()
            .and_then(|a| a.id.clone())
            .ok_or_else(|| SourceError::NotFound("Author not found".to_string()))?;

        // Get papers by author
        let mut papers_url = format!("/authors/{}/works?per-page={}", author_id, max_results);

        // Add year filter if provided (OpenAlex uses "from_publication_date" and "to_publication_date")
        if let Some(y) = year {
            // Handle year ranges like "2018-2022", "2010-", "-2015"
            if y.contains('-') {
                let parts: Vec<&str> = y.split('-').collect();
                if parts.len() == 2 {
                    if !parts[0].is_empty() {
                        papers_url.push_str(&format!("&from_publication_date={}-01-01", parts[0]));
                    }
                    if !parts[1].is_empty() {
                        papers_url.push_str(&format!("&to_publication_date={}-12-31", parts[1]));
                    }
                }
            } else if let Ok(year_num) = y.parse::<i32>() {
                // Single year - use from/to publication date for exact year
                papers_url.push_str(&format!(
                    "&from_publication_date={}-01-01&to_publication_date={}-12-31",
                    year_num, year_num
                ));
            }
        }

        let papers_response = self
            .client
            .get(&self.build_url(&papers_url))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch author papers: {}", e)))?;

        let papers_data: WorksResponse = papers_response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let papers: Result<Vec<Paper>, SourceError> = papers_data
            .results
            .into_iter()
            .map(|p| Ok(Self::parse_paper(&p)))
            .collect();

        Ok(SearchResponse::new(papers?, "OpenAlex", author))
    }

    async fn download(&self, request: &DownloadRequest) -> Result<DownloadResult, SourceError> {
        // Try to get the paper first to find PDF URL
        let url = format!("/works/{}", request.paper_id);

        let response = self
            .client
            .get(&self.build_url(&url))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch paper: {}", e)))?;

        if !response.status().is_success() {
            return Err(SourceError::NotFound(format!(
                "Paper not found: {}",
                request.paper_id
            )));
        }

        let data: WorkResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let pdf_url = data
            .best_open_access_pdf
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
            "/works?filter=cites:{}&per-page={}",
            urlencoding::encode(&request.paper_id),
            request.max_results
        );

        let response = self
            .client
            .get(&self.build_url(&url))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch citations: {}", e)))?;

        let data: WorksResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let papers: Result<Vec<Paper>, SourceError> = data
            .results
            .into_iter()
            .map(|p| Ok(Self::parse_paper(&p)))
            .collect();

        Ok(SearchResponse::new(papers?, "OpenAlex", &request.paper_id))
    }

    async fn get_references(
        &self,
        request: &CitationRequest,
    ) -> Result<SearchResponse, SourceError> {
        let url = format!(
            "/works?filter=referenceds:{}&per-page={}",
            urlencoding::encode(&request.paper_id),
            request.max_results
        );

        let response = self
            .client
            .get(&self.build_url(&url))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch references: {}", e)))?;

        let data: WorksResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let papers: Result<Vec<Paper>, SourceError> = data
            .results
            .into_iter()
            .map(|p| Ok(Self::parse_paper(&p)))
            .collect();

        Ok(SearchResponse::new(papers?, "OpenAlex", &request.paper_id))
    }

    async fn get_related(&self, request: &CitationRequest) -> Result<SearchResponse, SourceError> {
        let url = format!(
            "/works?filter=related:{}&per-page={}",
            urlencoding::encode(&request.paper_id),
            request.max_results
        );

        let response = self
            .client
            .get(&self.build_url(&url))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch related: {}", e)))?;

        let data: WorksResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let papers: Result<Vec<Paper>, SourceError> = data
            .results
            .into_iter()
            .map(|p| Ok(Self::parse_paper(&p)))
            .collect();

        Ok(SearchResponse::new(papers?, "OpenAlex", &request.paper_id))
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let url = format!("/works/doi:{}", urlencoding::encode(doi));

        let response = self
            .client
            .get(&self.build_url(&url))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch DOI: {}", e)))?;

        if !response.status().is_success() {
            return Err(SourceError::NotFound("DOI not found".to_string()));
        }

        let data: WorkResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        Ok(Self::parse_paper(&data))
    }

    async fn get_by_id(&self, id: &str) -> Result<Paper, SourceError> {
        let url = format!("/works/{}", urlencoding::encode(id));

        let response = self
            .client
            .get(&self.build_url(&url))
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch paper: {}", e)))?;

        if !response.status().is_success() {
            return Err(SourceError::NotFound(format!("Paper '{}' not found", id)));
        }

        let data: WorkResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        Ok(Self::parse_paper(&data))
    }
}

// ===== OpenAlex API Types =====

#[derive(Debug, Deserialize)]
struct OAPaper {
    id: Option<String>,
    title: String,
    publication_year: Option<i32>,
    #[serde(rename = "cited_by_count")]
    cited_by_count: Option<i32>,
    doi: Option<String>,
    r#abstract: Option<String>,
    best_open_access_pdf: Option<OAPdf>,
    authorships: Vec<OAAuthorship>,
}

#[derive(Debug, Deserialize)]
struct OAPdf {
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAAuthorship {
    author: OAAuthor,
}

#[derive(Debug, Deserialize)]
struct OAAuthor {
    #[serde(rename = "display_name")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorksResponse {
    results: Vec<OAPaper>,
    meta: Meta,
}

#[derive(Debug, Deserialize)]
struct Meta {
    count: usize,
}

type WorkResponse = OAPaper;

#[derive(Debug, Deserialize)]
struct AuthorsResponse {
    results: Vec<OAAuthorData>,
}

#[derive(Debug, Deserialize)]
struct OAAuthorData {
    id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openalex_source_creation() {
        let source = OpenAlexSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_openalex_capabilities() {
        let source = OpenAlexSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::DOWNLOAD));
        assert!(caps.contains(SourceCapabilities::READ));
        assert!(caps.contains(SourceCapabilities::CITATIONS));
        assert!(caps.contains(SourceCapabilities::DOI_LOOKUP));
        assert!(caps.contains(SourceCapabilities::AUTHOR_SEARCH));
    }

    #[test]
    fn test_openalex_id() {
        let source = OpenAlexSource::new().unwrap();
        assert_eq!(source.id(), "openalex");
    }

    #[test]
    fn test_openalex_name() {
        let source = OpenAlexSource::new().unwrap();
        assert_eq!(source.name(), "OpenAlex");
    }

    #[test]
    fn test_parse_search_response() {
        // Mock OpenAlex search API response
        let mock_response = r#"
        {
            "results": [
                {
                    "id": "https://openalex.org/W1234567890",
                    "title": "Deep Learning for Computer Vision",
                    "publication_year": 2023,
                    "cited_by_count": 50,
                    "doi": "https://doi.org/10.1234/cvpr2023",
                    "abstract": "A comprehensive survey of deep learning techniques.",
                    "authorships": [
                        {"author": {"display_name": "Alice Chen"}},
                        {"author": {"display_name": "Bob Smith"}}
                    ]
                }
            ],
            "meta": {"count": 1, "db_response_time_ms": 10}
        }
        "#;

        // Parse the mock response
        let parse_result: Result<WorksResponse, serde_json::Error> =
            serde_json::from_str(mock_response);
        assert!(parse_result.is_ok(), "Mock response should be valid JSON");

        // Verify parsed data
        let response = parse_result.unwrap();
        assert_eq!(response.results.len(), 1);

        let paper = &response.results[0];
        assert_eq!(paper.title, "Deep Learning for Computer Vision");
        assert_eq!(paper.publication_year, Some(2023));
        assert_eq!(paper.cited_by_count, Some(50));
        assert_eq!(paper.authorships.len(), 2);
    }

    #[test]
    fn test_parse_paper_details() {
        // Mock OpenAlex paper details response
        let mock_response = r#"
        {
            "id": "https://openalex.org/W9876543210",
            "title": "Natural Language Processing Advances",
            "publication_year": 2024,
            "cited_by_count": 100,
            "doi": "https://doi.org/10.5678/acl2024",
            "abstract": "New advances in NLP and transformers.",
            "authorships": [
                {"author": {"display_name": "Carol Davis"}}
            ]
        }
        "#;

        let parse_result: Result<WorkResponse, serde_json::Error> =
            serde_json::from_str(mock_response);
        assert!(parse_result.is_ok(), "Should parse valid JSON");

        let paper = parse_result.unwrap();
        assert_eq!(paper.title, "Natural Language Processing Advances");
        assert_eq!(paper.publication_year, Some(2024));
        assert_eq!(paper.cited_by_count, Some(100));
        assert_eq!(paper.authorships.len(), 1);
    }

    #[test]
    fn test_parse_citations_response() {
        // Mock citations response (uses same structure as search)
        let mock_response = r#"
        {
            "results": [
                {
                    "id": "https://openalex.org/W111",
                    "title": "Citing Paper 1",
                    "publication_year": 2024,
                    "cited_by_count": 10,
                    "authorships": [{"author": {"display_name": "Author One"}}]
                },
                {
                    "id": "https://openalex.org/W222",
                    "title": "Citing Paper 2",
                    "publication_year": 2023,
                    "cited_by_count": 5,
                    "authorships": [{"author": {"display_name": "Author Two"}}]
                }
            ],
            "meta": {"count": 2}
        }
        "#;

        let parse_result: Result<WorksResponse, serde_json::Error> =
            serde_json::from_str(mock_response);
        assert!(parse_result.is_ok(), "Should parse valid citations JSON");

        let citations = parse_result.unwrap();
        assert_eq!(citations.results.len(), 2);
        assert_eq!(citations.results[0].title, "Citing Paper 1");
        assert_eq!(citations.results[1].title, "Citing Paper 2");
    }

    #[test]
    fn test_parse_author_search_response() {
        // Mock author search response
        let mock_response = r#"
        {
            "results": [
                {"id": "https://openalex.org/A1234567890"},
                {"id": "https://openalex.org/A0987654321"}
            ],
            "meta": {"count": 2}
        }
        "#;

        let parse_result: Result<AuthorsResponse, serde_json::Error> =
            serde_json::from_str(mock_response);
        assert!(parse_result.is_ok(), "Should parse valid authors JSON");

        let authors = parse_result.unwrap();
        assert_eq!(authors.results.len(), 2);
        assert!(authors.results[0].id.is_some());
        assert!(authors.results[1].id.is_some());
    }
}
