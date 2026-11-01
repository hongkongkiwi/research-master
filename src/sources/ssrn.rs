//! SSRN research source implementation.

use async_trait::async_trait;
use scraper::{ElementRef, Html, Selector};
use std::sync::Arc;

use crate::models::{
    Paper, PaperBuilder, ReadRequest, ReadResult, SearchQuery, SearchResponse, SourceType,
};
use crate::sources::{DownloadRequest, DownloadResult, Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

#[allow(dead_code)]
const SSRN_BASE_URL: &str = "https://papers.ssrn.com";
const SSRN_ABSTRACT_URL: &str = "https://papers.ssrn.com/abstract";

/// SSRN research source
///
/// SSRN doesn't have a public API, so we use web scraping with proper rate limiting.
#[derive(Debug, Clone)]
pub struct SsrnSource {
    client: Arc<HttpClient>,
}

impl SsrnSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }

    /// Extract paper ID from URL
    fn extract_paper_id(&self, url: &str) -> Option<String> {
        // Try to find numeric ID in URL
        let re = regex::Regex::new(r"(\d{5,})").ok()?;
        re.captures(url)?.get(1).map(|m| m.as_str().to_string())
    }

    /// Parse paper entry from HTML element
    fn parse_paper_entry(&self, elem: &ElementRef) -> Option<Paper> {
        // Try to find title link within this element
        let title_selector = Selector::parse("a.title, a[href*='abstract']").ok()?;
        let title_elem = elem.select(&title_selector).next()?;

        let title = title_elem.text().collect::<String>().trim().to_string();
        if title.is_empty() {
            return None;
        }

        let href = title_elem.value().attr("href")?;
        let paper_id = self.extract_paper_id(href).unwrap_or_default();

        // Extract authors
        let authors_selector = Selector::parse("span.authors, .author-name").ok()?;
        let authors = elem
            .select(&authors_selector)
            .next()
            .map(|a| {
                a.text()
                    .collect::<String>()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();

        // Extract abstract
        let abstract_selector = Selector::parse("div.abstract, .abstract-text").ok()?;
        let abstract_text = elem
            .select(&abstract_selector)
            .next()
            .map(|a| a.text().collect::<String>())
            .unwrap_or_default();

        // Extract topics/categories
        let topic_selector = Selector::parse("a.topic, .category").ok()?;
        let categories: Vec<String> = elem
            .select(&topic_selector)
            .map(|t| t.text().collect::<String>().trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();

        let url = format!("{}/{}.html", SSRN_ABSTRACT_URL, paper_id);

        // PDF URL (if available)
        let pdf_selector = Selector::parse("a.download, a[href*='download']").ok();
        let pdf_url = pdf_selector
            .and_then(|s| elem.select(&s).next())
            .and_then(|a| a.value().attr("href"))
            .map(|s| s.to_string())
            .unwrap_or_default();

        Some(
            PaperBuilder::new(paper_id.clone(), title, url, SourceType::SSRN)
                .authors(authors)
                .abstract_text(truncate_chars(&abstract_text, 3000))
                .categories(categories.join(", "))
                .pdf_url(pdf_url)
                .build(),
        )
    }
}

/// Shorten `text` to at most `max` characters without splitting a character.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

impl Default for SsrnSource {
    fn default() -> Self {
        Self::new().expect("Failed to create SsrnSource")
    }
}

#[async_trait]
impl Source for SsrnSource {
    fn id(&self) -> &str {
        "ssrn"
    }

    fn name(&self) -> &str {
        "SSRN"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOWNLOAD | SourceCapabilities::READ
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let url = format!(
            "{}/search.cfm?query={}&pg=1",
            SSRN_BASE_URL,
            urlencoding::encode(&query.query)
        );

        // Clone values for retry closure
        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let response = client
                    .get(&url)
                    .header("Accept", "text/html")
                    .send()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to search SSRN: {}", e)))?;

                // SSRN (Elsevier) may block automated requests with 403
                // Also protected by Cloudflare which returns 403 for automated requests
                if response.status() == reqwest::StatusCode::FORBIDDEN {
                    // Read the response body to check for blocking
                    let text = response.text().await.unwrap_or_default();
                    let text_lower = text.to_lowercase();
                    // Check for various blocking indicators
                    if text_lower.contains("automated script")
                        || text_lower.contains("elsevier")
                        || text_lower.contains("content_protection")
                        || text_lower.contains("search engine")
                        || text_lower.contains("cloudflare")
                        || text_lower.contains("attention required")
                    {
                        tracing::debug!("SSRN blocked automated requests (Cloudflare/Elsevier anti-bot), skipping");
                        return Err(SourceError::Api("SSRN blocked automated requests".to_string()));
                    }
                    // If it's a 403 but not blocking detected
                    tracing::debug!("SSRN returned 403 - skipping");
                    return Err(SourceError::Api("SSRN returned 403".to_string()));
                }

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "SSRN returned status: {}",
                        response.status()
                    )));
                }

                Ok(response)
            }
        })
        .await;

        // A Cloudflare/anti-bot block is a failure; propagate it rather than
        // masking it as "no results".
        let response = response?;
        let html_content = response
            .text()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to read HTML: {}", e)))?;

        let document = Html::parse_document(&html_content);

        // Try to find paper entries
        let paper_selector = Selector::parse("div.paper-card, tr.data").ok();
        let mut papers = Vec::new();

        if let Some(selector) = paper_selector {
            for entry in document.select(&selector) {
                if let Some(paper) = self.parse_paper_entry(&entry) {
                    papers.push(paper);
                }
            }
        }

        // If no papers found, try alternative parsing
        if papers.is_empty() {
            // Try finding individual paper links
            let link_selector = Selector::parse("a[href*='abstract']").ok();
            if let Some(selector) = link_selector {
                for link in document.select(&selector).take(query.max_results) {
                    if let Some(href) = link.value().attr("href") {
                        if let Some(paper_id) = self.extract_paper_id(href) {
                            let title = link.text().collect::<String>().trim().to_string();
                            if !title.is_empty() {
                                let url = format!("{}/{}.html", SSRN_ABSTRACT_URL, paper_id);
                                papers.push(
                                    PaperBuilder::new(
                                        paper_id.clone(),
                                        title,
                                        url,
                                        SourceType::SSRN,
                                    )
                                    .build(),
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(SearchResponse::new(papers, "SSRN", &query.query))
    }

    async fn download(&self, request: &DownloadRequest) -> Result<DownloadResult, SourceError> {
        // Get the paper page to find download link
        let url = format!("{}/{}.html", SSRN_ABSTRACT_URL, request.paper_id);

        let response = self
            .client
            .get(&url)
            .header("Accept", "text/html")
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch paper: {}", e)))?;

        if !response.status().is_success() {
            return Err(SourceError::NotFound(format!(
                "Paper not found: {}",
                request.paper_id
            )));
        }

        let html_content = response
            .text()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to read HTML: {}", e)))?;

        // Extract download URL before the await point
        let download_url = {
            let document = Html::parse_document(&html_content);
            let download_selector = Selector::parse("a[href*='download'], a.download").ok();
            if let Some(selector) = download_selector {
                document
                    .select(&selector)
                    .next()
                    .and_then(|a| a.value().attr("href"))
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            } else {
                return Err(SourceError::NotFound("No download link found".to_string()));
            }
        };

        if download_url.is_empty() {
            return Err(SourceError::NotFound("No PDF available".to_string()));
        }

        let pdf_response = self
            .client
            .get(&download_url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to download PDF: {}", e)))?;

        if !pdf_response.status().is_success() {
            return Err(SourceError::NotFound("PDF download failed".to_string()));
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

        let filename = format!("ssrn_{}.pdf", request.paper_id);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssrn_source_creation() {
        let source = SsrnSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_ssrn_capabilities() {
        let source = SsrnSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::DOWNLOAD));
        assert!(caps.contains(SourceCapabilities::READ));
    }

    #[test]
    fn test_ssrn_id() {
        let source = SsrnSource::new().unwrap();
        assert_eq!(source.id(), "ssrn");
    }

    #[test]
    fn test_ssrn_name() {
        let source = SsrnSource::new().unwrap();
        assert_eq!(source.name(), "SSRN");
    }
}
