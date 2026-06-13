//! CiteSeerX research source implementation.
//!
//! Uses the CiteSeerX OpenSearch API for searching and retrieving computer science papers.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const CITESEERX_API_BASE: &str = "https://citeseerx.ist.psu.edu/api";
const CITESEERX_NAME: &str = "CiteSeerX";

/// CiteSeerX research source
///
/// Uses the CiteSeerX OpenSearch JSON API.
#[derive(Debug, Clone)]
pub struct CiteseerxSource {
    client: Arc<HttpClient>,
}

impl CiteseerxSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }

    fn parse_document(&self, doc: &CiteseerxDocument) -> Paper {
        let id = doc.id.clone().unwrap_or_default();
        let title = doc.title.clone().unwrap_or_default();
        let doi = doc.doi.clone().unwrap_or_default();
        let year = doc.year_as_string();
        let authors = doc.authors.clone().unwrap_or_default().join("; ");
        let abstract_text = doc.r#abstract.clone().unwrap_or_default();
        let url = doc.url.clone().unwrap_or_else(|| {
            if !doi.is_empty() {
                format!("https://citeseerx.ist.psu.edu/document?doi={}", doi)
            } else {
                format!("https://citeseerx.ist.psu.edu/document?doi={}", id)
            }
        });

        let mut builder = PaperBuilder::new(id, title, url, SourceType::CiteSeerX)
            .authors(authors)
            .abstract_text(abstract_text)
            .published_date(year);

        if !doi.is_empty() {
            builder = builder.doi(doi);
        }

        if let Some(venue) = &doc.venue {
            if !venue.is_empty() {
                builder = builder.categories(venue.clone());
            }
        }

        builder.build()
    }

    async fn fetch_response(
        &self,
        url: String,
        error_context: &str,
    ) -> Result<CiteseerxResponse, SourceError> {
        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();
        let error_context = error_context.to_string();

        with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            let error_context = error_context.clone();
            async move {
                let response = client.get(&url).send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to {} CiteSeerX: {}", error_context, e))
                })?;

                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "CiteSeerX API returned status {}: {}",
                        status, text
                    )));
                }

                response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse CiteSeerX response: {}", e))
                })
            }
        })
        .await
    }
}

impl Default for CiteseerxSource {
    fn default() -> Self {
        Self::new().expect("Failed to create CiteseerxSource")
    }
}

#[async_trait]
impl Source for CiteseerxSource {
    fn id(&self) -> &str {
        "citeseerx"
    }

    fn name(&self) -> &str {
        CITESEERX_NAME
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let max_results = query.max_results.min(10);
        let url = format!(
            "{}/search?q={}&t=json&n={}&sort=rel",
            CITESEERX_API_BASE,
            urlencoding::encode(&query.query),
            max_results
        );

        let response = self.fetch_response(url, "search").await?;
        let total = response.response.num_found;
        let papers = response
            .response
            .docs
            .iter()
            .map(|doc| self.parse_document(doc))
            .collect();

        let mut search_response = SearchResponse::new(papers, CITESEERX_NAME, &query.query);
        search_response.total_results = total;
        Ok(search_response)
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let url = format!(
            "{}/documents?doi={}",
            CITESEERX_API_BASE,
            urlencoding::encode(&clean_doi)
        );

        let response = self.fetch_response(url, "lookup DOI in").await?;
        response
            .response
            .docs
            .first()
            .map(|doc| self.parse_document(doc))
            .ok_or_else(|| SourceError::NotFound(format!("DOI not found in CiteSeerX: {}", doi)))
    }

    async fn get_by_id(&self, id: &str) -> Result<Paper, SourceError> {
        let url = format!(
            "{}/documents?ids={}",
            CITESEERX_API_BASE,
            urlencoding::encode(id)
        );

        let response = self.fetch_response(url, "fetch document from").await?;
        response
            .response
            .docs
            .first()
            .map(|doc| self.parse_document(doc))
            .ok_or_else(|| SourceError::NotFound(format!("Paper not found in CiteSeerX: {}", id)))
    }
}

/// CiteSeerX OpenSearch API response
#[derive(Debug, Deserialize)]
struct CiteseerxResponse {
    response: CiteseerxResponseBody,
}

#[derive(Debug, Deserialize)]
struct CiteseerxResponseBody {
    #[serde(rename = "numFound")]
    num_found: Option<usize>,
    docs: Vec<CiteseerxDocument>,
}

#[derive(Debug, Deserialize)]
struct CiteseerxDocument {
    id: Option<String>,
    title: Option<String>,
    authors: Option<Vec<String>>,
    #[serde(rename = "abstract")]
    r#abstract: Option<String>,
    doi: Option<String>,
    year: Option<serde_json::Value>,
    url: Option<String>,
    venue: Option<String>,
}

impl CiteseerxDocument {
    fn year_as_string(&self) -> String {
        match &self.year {
            Some(serde_json::Value::String(year)) => year.clone(),
            Some(serde_json::Value::Number(year)) => year.to_string(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_json() -> &'static str {
        r#"{
            "response": {
                "numFound": 1,
                "docs": [{
                    "id": "csx-1",
                    "title": "Mock Paper Title",
                    "authors": ["Ada Lovelace", "Alan Turing"],
                    "abstract": "Mock abstract text.",
                    "doi": "10.1234/mock",
                    "year": 2024,
                    "url": "https://citeseerx.ist.psu.edu/document?doi=10.1234/mock",
                    "venue": "MockConf"
                }]
            }
        }"#
    }

    #[test]
    fn test_source_creation() {
        let source = CiteseerxSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = CiteseerxSource::new().unwrap();
        assert_eq!(source.id(), "citeseerx");
        assert_eq!(source.name(), "CiteSeerX");
    }

    #[test]
    fn test_capabilities() {
        let source = CiteseerxSource::new().unwrap();
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
        let response: CiteseerxResponse = serde_json::from_str(mock_json()).unwrap();
        assert_eq!(response.response.num_found, Some(1));
        assert_eq!(response.response.docs.len(), 1);
        assert_eq!(response.response.docs[0].year_as_string(), "2024");
    }

    #[test]
    fn test_parse_document_maps_response_fields() {
        let source = CiteseerxSource::new().unwrap();
        let response: CiteseerxResponse = serde_json::from_str(mock_json()).unwrap();
        let paper = source.parse_document(&response.response.docs[0]);
        assert_eq!(paper.paper_id, "csx-1");
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.published_date.as_deref(), Some("2024"));
        assert_eq!(paper.categories.as_deref(), Some("MockConf"));
        assert_eq!(paper.source, crate::models::SourceType::CiteSeerX);
    }

    #[test]
    fn test_year_as_string_handles_string_number_and_missing_values() {
        let doc_with_string_year = CiteseerxDocument {
            id: None,
            title: None,
            authors: None,
            r#abstract: None,
            doi: None,
            year: Some(serde_json::json!("2023")),
            url: None,
            venue: None,
        };
        assert_eq!(doc_with_string_year.year_as_string(), "2023");

        let doc_with_number_year = CiteseerxDocument {
            year: Some(serde_json::json!(2024)),
            ..doc_with_string_year
        };
        assert_eq!(doc_with_number_year.year_as_string(), "2024");

        let doc_without_year = CiteseerxDocument {
            year: None,
            ..doc_with_number_year
        };
        assert_eq!(doc_without_year.year_as_string(), "");
    }
}
