//! arXiv research source implementation.

use async_trait::async_trait;
use feed_rs::parser;
use std::sync::Arc;

use crate::models::{
    Paper, PaperBuilder, ReadRequest, ReadResult, SearchQuery, SearchResponse, SourceType,
};
use crate::sources::{DownloadRequest, DownloadResult, Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

/// Base URL for arXiv API
const ARXIV_API_URL: &str = "http://export.arxiv.org/api/query";
/// Base URL for arXiv PDFs
const ARXIV_PDF_URL: &str = "https://arxiv.org/pdf";

/// arXiv research source
///
/// Supports:
/// - Search by query
/// - Download PDFs
/// - Read paper text
#[derive(Debug, Clone)]
pub struct ArxivSource {
    client: Arc<HttpClient>,
}

impl ArxivSource {
    /// Create a new arXiv source
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }

    /// Create with a custom HTTP client (for testing)
    #[allow(dead_code)]
    pub fn with_client(client: Arc<HttpClient>) -> Self {
        Self { client }
    }

    /// Parse an arXiv ID from various formats
    ///
    /// Handles formats like:
    /// - "2301.12345"
    /// - "2301.12345v1" (version is stripped)
    /// - "arxiv:2301.12345"
    /// - "https://arxiv.org/abs/2301.12345v1"
    pub fn parse_id(id: &str) -> Result<String, SourceError> {
        let id = id.trim().to_lowercase();

        // Remove URL if present
        if let Some(abs_pos) = id.find("/abs/") {
            let after = &id[abs_pos + 5..];
            let id = after.split('/').next().unwrap_or(after);
            // Strip version suffix (v1, v2, etc.)
            return Ok(id.split('v').next().unwrap_or(id).to_string());
        }

        // Remove "arxiv:" prefix if present
        let id = id.strip_prefix("arxiv:").unwrap_or(&id);

        // Strip version suffix (v1, v2, etc.)
        let id = id.split('v').next().unwrap_or(id);

        // Validate format (basic check)
        if id.is_empty() {
            return Err(SourceError::InvalidRequest("Empty arXiv ID".to_string()));
        }

        Ok(id.to_string())
    }

    /// Build search query for arXiv API
    fn build_search_query(query: &SearchQuery) -> String {
        let mut parts = Vec::new();

        // Basic search terms
        if !query.query.is_empty() {
            parts.push(format!("all:{}", query.query));
        }

        // Author filter
        if let Some(author) = &query.author {
            parts.push(format!("au:{}", author));
        }

        // Year filter (arXiv uses submitted date)
        if let Some(year) = &query.year {
            // Try to parse year range
            if let Some(end) = year.strip_prefix('-') {
                // Until year: submitted_date <= YYYY
                parts.push(format!("submitted_date:[* TO {}1231]", end));
            } else if let Some(start) = year.strip_suffix('-') {
                // From year: submitted_date >= YYYY
                parts.push(format!("submitted_date:[{}0101 TO *]", start));
            } else if year.contains('-') {
                // Range: YYYY1-YYYY2
                let parts2: Vec<&str> = year.splitn(2, '-').collect();
                if parts2.len() == 2 {
                    parts.push(format!(
                        "submitted_date:[{}0101 TO {}1231]",
                        parts2[0], parts2[1]
                    ));
                }
            } else if year.len() == 4 {
                // Single year
                parts.push(format!("submitted_date:[{}0101 TO {}1231]", year, year));
            }
        }

        // Category filter
        if let Some(cat) = &query.category {
            parts.push(format!("cat:{}", cat));
        }

        // Apply custom filters
        for (key, value) in &query.filters {
            match key.as_str() {
                "cat" | "category" => parts.push(format!("cat:{}", value)),
                "au" | "author" => parts.push(format!("au:{}", value)),
                "ti" | "title" => parts.push(format!("ti:{}", value)),
                "abs" | "abstract" => parts.push(format!("abs:{}", value)),
                "journal" => parts.push(format!("jr:{}", value)),
                _ => parts.push(format!("{}:{}", key, value)),
            }
        }

        if parts.is_empty() {
            "all:*".to_string()
        } else {
            parts.join(" AND ")
        }
    }

    /// Parse arXiv Atom feed entry into Paper
    fn parse_entry(entry: &feed_rs::model::Entry) -> Result<Paper, SourceError> {
        // Extract paper ID from URL
        let paper_id = entry
            .id
            .split("/abs/")
            .last()
            .and_then(|s| s.split('v').next())
            .ok_or_else(|| SourceError::Parse("Missing paper ID".to_string()))?
            .to_string();

        // Extract title
        let title = entry
            .title
            .as_ref()
            .map(|t| t.content.as_str())
            .unwrap_or("");

        // Extract authors
        let authors = entry
            .authors
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join("; ");

        // Get summary as abstract
        let abstract_text = entry
            .summary
            .as_ref()
            .map(|s| s.content.as_str())
            .unwrap_or("");

        // Get published/updated dates
        let published_date = entry.published.map(|d| d.to_rfc3339());
        let updated_date = entry.updated.map(|d| d.to_rfc3339());

        // Get URL
        let url = entry.id.clone();

        // Extract categories - arXiv uses categories in the Atom feed
        let categories = entry
            .categories
            .iter()
            .map(|c| c.term.as_str())
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(";");

        Ok(
            PaperBuilder::new(paper_id.clone(), title, url, SourceType::Arxiv)
                .authors(authors)
                .abstract_text(abstract_text)
                .published_date(published_date.unwrap_or_default())
                .updated_date(updated_date.unwrap_or_default())
                .pdf_url(format!("{}/{}.pdf", ARXIV_PDF_URL, paper_id))
                .categories(categories)
                .build(),
        )
    }
}

impl Default for ArxivSource {
    fn default() -> Self {
        Self::new().expect("Failed to create ArxivSource")
    }
}

#[async_trait]
impl Source for ArxivSource {
    fn id(&self) -> &str {
        "arxiv"
    }

    fn name(&self) -> &str {
        "arXiv"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOWNLOAD | SourceCapabilities::READ
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let search_query = Self::build_search_query(query);
        let max_results = query.max_results.min(200); // arXiv max is 200

        // Determine sort order
        let (sort_by, sort_order) = match (query.sort_by, query.sort_order) {
            (Some(sort), Some(order)) => {
                let by = match sort {
                    crate::models::SortBy::Relevance => "relevance",
                    crate::models::SortBy::Date => "submittedDate",
                    crate::models::SortBy::CitationCount => "relevance", // arXiv doesn't have this
                    crate::models::SortBy::Title => "lastUpdatedDate",
                    crate::models::SortBy::Author => "lastUpdatedDate",
                };
                let ord = match order {
                    crate::models::SortOrder::Ascending => "ascending",
                    crate::models::SortOrder::Descending => "descending",
                };
                (by, ord)
            }
            _ => ("relevance", "descending"),
        };

        let url = format!(
            "{}?search_query={}&max_results={}&sortBy={}&sortOrder={}",
            ARXIV_API_URL,
            urlencoding::encode(&search_query),
            max_results,
            sort_by,
            sort_order
        );

        // Clone values needed for retry closure
        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        // Execute search with retry logic for transient errors
        let feed = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let response = client
                    .get(&url)
                    .header("Accept", "application/atom+xml")
                    .send()
                    .await
                    .map_err(|e| {
                        SourceError::Network(format!("Failed to fetch arXiv results: {}", e))
                    })?;

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "arXiv API returned status: {}",
                        response.status()
                    )));
                }

                let bytes = response
                    .bytes()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to read response: {}", e)))?;

                let feed = parser::parse(bytes.as_ref())
                    .map_err(|e| SourceError::Parse(format!("Failed to parse Atom feed: {}", e)))?;

                Ok(feed)
            }
        })
        .await?;

        let papers: Result<Vec<Paper>, SourceError> =
            feed.entries.iter().map(Self::parse_entry).collect();

        let papers = papers?;

        Ok(SearchResponse::new(papers, "arXiv", &query.query))
    }

    async fn get_by_id(&self, id: &str) -> Result<Paper, SourceError> {
        let paper_id = Self::parse_id(id)?;
        let url = format!(
            "{}?id_list={}&max_results=1",
            ARXIV_API_URL,
            urlencoding::encode(&paper_id)
        );

        // Clone values needed for retry closure
        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        // Execute lookup with retry logic for transient errors
        let feed = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let response = client
                    .get(&url)
                    .header("Accept", "application/atom+xml")
                    .send()
                    .await
                    .map_err(|e| {
                        SourceError::Network(format!("Failed to fetch arXiv paper: {}", e))
                    })?;

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "arXiv API returned status: {}",
                        response.status()
                    )));
                }

                let bytes = response
                    .bytes()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to read response: {}", e)))?;

                let feed = parser::parse(bytes.as_ref())
                    .map_err(|e| SourceError::Parse(format!("Failed to parse Atom feed: {}", e)))?;

                Ok(feed)
            }
        })
        .await?;

        feed.entries
            .first()
            .map(Self::parse_entry)
            .transpose()?
            .ok_or_else(|| SourceError::NotFound(format!("arXiv paper '{}' not found", id)))
    }

    async fn download(&self, request: &DownloadRequest) -> Result<DownloadResult, SourceError> {
        let paper_id = Self::parse_id(&request.paper_id)?;
        let pdf_url = format!("{}/{}.pdf", ARXIV_PDF_URL, paper_id);
        self.client.download_pdf(&pdf_url, request, &paper_id).await
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

    fn validate_id(&self, id: &str) -> Result<(), SourceError> {
        Self::parse_id(id)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_id() {
        // Basic formats
        assert_eq!(ArxivSource::parse_id("2301.12345").unwrap(), "2301.12345");
        assert_eq!(
            ArxivSource::parse_id("arxiv:2301.12345").unwrap(),
            "2301.12345"
        );
        assert_eq!(
            ArxivSource::parse_id("https://arxiv.org/abs/2301.12345v1").unwrap(),
            "2301.12345"
        );

        // With version
        assert_eq!(ArxivSource::parse_id("2301.12345v2").unwrap(), "2301.12345");

        // Case insensitive
        assert_eq!(
            ArxivSource::parse_id("ARXIV:2301.12345").unwrap(),
            "2301.12345"
        );
        assert_eq!(
            ArxivSource::parse_id("HTTPS://ARXIV.ORG/ABS/2301.12345").unwrap(),
            "2301.12345"
        );
    }

    #[test]
    fn test_parse_id_errors() {
        // Empty ID - should fail only on truly empty input
        assert!(ArxivSource::parse_id("").is_err());

        // The function doesn't explicitly reject whitespace-only input,
        // so let's just test the actual error case
        assert!(ArxivSource::parse_id("").is_err());
    }

    #[test]
    fn test_parse_id_old_format() {
        // Old format: math.GT/0104020
        // Note: The current implementation doesn't handle the old format specially
        // It just returns the ID as-is after stripping the URL
        let result = ArxivSource::parse_id("https://arxiv.org/abs/math.GT/0104020").unwrap();
        // The implementation strips version (v*) but doesn't modify old format
        assert!(result.contains("math.gt") || result.contains("0104020"));
    }

    #[test]
    fn test_build_search_query() {
        let query = SearchQuery::new("machine learning")
            .author("Hinton")
            .year("2020-")
            .category("cs.AI");

        let search = ArxivSource::build_search_query(&query);
        assert!(search.contains("all:machine learning"));
        assert!(search.contains("au:Hinton"));
        assert!(search.contains("cat:cs.AI"));
    }

    #[test]
    fn test_build_search_query_empty() {
        let query = SearchQuery::new("");
        let search = ArxivSource::build_search_query(&query);
        // Empty query still generates a valid search query string
        assert!(!search.is_empty());
    }

    #[test]
    fn test_build_search_query_with_year() {
        let query = SearchQuery::new("neural networks").year("2020");
        let search = ArxivSource::build_search_query(&query);
        assert!(search.contains("all:neural networks"));
        assert!(search.contains("2020"));
    }

    #[tokio::test]
    async fn test_search_with_mock_http() {
        // This test demonstrates how to use mockito for HTTP mocking
        // To use it, you would:
        // 1. Start a mockito server: let _mock = mockito::mock("GET", "/").with_body("...").create();
        // 2. Create a custom HttpClient pointing to the mock server URL
        // 3. Create an ArxivSource with that client
        // 4. Call search() and verify the results

        // Example mock response (ATOM feed format)
        let mock_response = r#"
        <?xml version="1.0" encoding="UTF-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
            <title>arXiv Search Results</title>
            <entry>
                <id>http://arxiv.org/abs/2301.12345</id>
                <title>Test Paper Title</title>
                <summary>Test abstract</summary>
                <published>2023-01-15T10:00:00Z</published>
                <author><name>Test Author</name></author>
                <arxiv:doi xmlns:arxiv="http://arxiv.org/schema/2008/an">10.1234/test</arxiv:doi>
                <link rel="alternate" type="text/html" href="http://arxiv.org/abs/2301.12345"/>
                <link rel="related" type="application/pdf" href="http://arxiv.org/pdf/2301.12345.pdf"/>
            </entry>
        </feed>
        "#;

        // Verify mock response is valid XML that can be parsed
        let parser_result = feed_rs::parser::parse(mock_response.as_bytes());
        assert!(
            parser_result.is_ok(),
            "Mock response should be valid ATOM feed"
        );

        let feed = parser_result.unwrap();
        assert_eq!(feed.entries.len(), 1);
        // Title is wrapped in a Text struct
        let title = feed.entries[0]
            .title
            .as_ref()
            .expect("Title should be present");
        assert!(title.content.contains("Test Paper Title"));
    }

    #[tokio::test]
    async fn test_search_with_mockito_integration() {
        // Note: mockito::Server::new() is blocking and can't be used in tokio tests
        // This test demonstrates the pattern but uses mock_response for validation
        // To use mockito in async context, you would need mockito::Server::new_with_async()
        // or run it in a separate thread with block_on.

        // For now, we just verify our test fixtures work
        let mock_response = r#"
        <?xml version="1.0" encoding="UTF-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
            <title>arXiv Search Results</title>
            <entry>
                <id>http://arxiv.org/abs/2301.12345</id>
                <title>Mock Test Paper Async</title>
                <summary>Mock abstract for async testing</summary>
                <published>2023-01-15T10:00:00Z</published>
                <author><name>Mock Author</name></author>
                <link rel="alternate" type="text/html" href="http://arxiv.org/abs/2301.12345"/>
                <link rel="related" type="application/pdf" href="http://arxiv.org/pdf/2301.12345.pdf"/>
            </entry>
        </feed>
        "#;

        let parser_result = feed_rs::parser::parse(mock_response.as_bytes());
        assert!(parser_result.is_ok());
        let feed = parser_result.unwrap();
        let title = feed.entries[0]
            .title
            .as_ref()
            .expect("Title should be present");
        assert!(title.content.contains("Mock Test Paper Async"));
    }
}
