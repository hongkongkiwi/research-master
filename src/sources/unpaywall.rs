//! Unpaywall research source implementation.
//!
//! Uses the Unpaywall API for checking open access status of papers.
//! API documentation: <https://unpaywall.org/api/v2>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const UNPAYWALL_API_BASE: &str = "https://api.unpaywall.org/v2";

/// Unpaywall research source
///
/// Uses the Unpaywall API for checking open access status of papers.
/// API requires an email address (free, no key needed).
#[derive(Debug, Clone)]
pub struct UnpaywallSource {
    client: Arc<HttpClient>,
    email: Option<String>,
}

impl UnpaywallSource {
    pub fn new() -> Result<Self, SourceError> {
        let email = std::env::var("UNPAYWALL_EMAIL").ok();
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            email,
        })
    }
}

impl Default for UnpaywallSource {
    fn default() -> Self {
        Self::new().expect("Failed to create UnpaywallSource")
    }
}

#[async_trait]
impl Source for UnpaywallSource {
    fn id(&self) -> &str {
        "unpaywall"
    }

    fn name(&self) -> &str {
        "Unpaywall"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::DOI_LOOKUP
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let email = self
            .email
            .as_deref()
            .unwrap_or("research-master@example.com");

        let url = format!(
            "{}/{}?email={}",
            UNPAYWALL_API_BASE,
            urlencoding::encode(&clean_doi),
            urlencoding::encode(email)
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let request = client.get(&url);

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in Unpaywall: {}", e))
                })?;

                if response.status() == 404 {
                    return Err(SourceError::NotFound(format!(
                        "Paper not found in Unpaywall: {}",
                        doi
                    )));
                }

                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "Unpaywall API returned status {}: {}",
                        status, text
                    )));
                }

                response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse Unpaywall response: {}", e))
                })
            }
        })
        .await?;

        self.parse_result(&response, doi)
    }
}

impl UnpaywallSource {
    fn parse_result(&self, item: &UnpaywallResponse, doi: &str) -> Result<Paper, SourceError> {
        let title = item.title.clone().unwrap_or_default();
        let abstract_text = item.abstract_text.clone().unwrap_or_default();

        let year = item.published_date.clone().unwrap_or_default();
        let url = format!("https://doi.org/{}", doi);

        let authors: String = item
            .authors
            .iter()
            .filter_map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join("; ");

        let pdf_url = item
            .best_oa_location
            .as_ref()
            .and_then(|loc| loc.url_for_pdf.clone());

        Ok(PaperBuilder::new(
            doi.to_string(),
            title,
            url,
            SourceType::Other("unpaywall".to_string()),
        )
        .authors(&authors)
        .published_date(&year)
        .abstract_text(&abstract_text)
        .doi(doi)
        .pdf_url(pdf_url.unwrap_or_default())
        .build())
    }
}

/// Unpaywall API response
#[derive(Debug, Deserialize)]
struct UnpaywallResponse {
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    published_date: Option<String>,
    authors: Vec<UnpaywallAuthor>,
    best_oa_location: Option<UnpaywallLocation>,
}

#[derive(Debug, Deserialize)]
struct UnpaywallAuthor {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UnpaywallLocation {
    url_for_pdf: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = UnpaywallSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = UnpaywallSource::new().unwrap();
        assert_eq!(source.id(), "unpaywall");
        assert_eq!(source.name(), "Unpaywall");
    }

    #[test]
    fn test_capabilities() {
        let source = UnpaywallSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::DOI_LOOKUP));
        assert_eq!(caps, SourceCapabilities::DOI_LOOKUP);
    }

    #[test]
    fn test_response_parsing_from_mock_json() {
        let json = r#"{
            "title": "Mock Paper Title",
            "abstract": "Mock abstract text.",
            "published_date": "2024-04-01",
            "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}],
            "best_oa_location": {"url_for_pdf": "https://example.org/mock.pdf"}
        }"#;
        let response: UnpaywallResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.title.as_deref(), Some("Mock Paper Title"));
        assert_eq!(response.authors.len(), 2);
        assert_eq!(
            response
                .best_oa_location
                .as_ref()
                .and_then(|loc| loc.url_for_pdf.as_deref()),
            Some("https://example.org/mock.pdf")
        );
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = UnpaywallSource::new().unwrap();
        let json = r#"{
            "title": "Mock Paper Title",
            "abstract": "Mock abstract text.",
            "published_date": "2024-04-01",
            "authors": [{"name": "Ada Lovelace"}, {"name": "Alan Turing"}],
            "best_oa_location": {"url_for_pdf": "https://example.org/mock.pdf"}
        }"#;
        let response: UnpaywallResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response, "10.1234/mock").unwrap();
        assert_eq!(paper.paper_id, "10.1234/mock");
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.url, "https://doi.org/10.1234/mock");
        assert_eq!(
            paper.pdf_url.as_deref(),
            Some("https://example.org/mock.pdf")
        );
        assert_eq!(paper.source.id(), "unpaywall");
    }
}
