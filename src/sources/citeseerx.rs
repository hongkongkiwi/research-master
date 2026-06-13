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

    #[test]
    fn test_source_creation() {
        let source = CiteseerxSource::new();
        assert!(source.is_ok());

        let source = source.unwrap();
        assert_eq!(source.id(), "citeseerx");
        assert_eq!(source.name(), "CiteSeerX");
        assert!(source.capabilities().contains(SourceCapabilities::SEARCH));
        assert!(source
            .capabilities()
            .contains(SourceCapabilities::DOI_LOOKUP));
    }
}
