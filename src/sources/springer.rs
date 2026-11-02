//! Springer research source implementation.
//!
//! Uses the Springer Nature API for searching publications from Springer Link.
//! API documentation: <https://dev.springernature.com>
//!
//! Requires a free API key from developer.springernature.com

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const SPRINGER_API_BASE: &str = "https://api.springernature.com/metadata/json";

/// Springer research source
///
/// Uses the Springer Nature API for searching publications from Springer Link.
/// Requires a free API key from developer.springernature.com
#[derive(Debug, Clone)]
pub struct SpringerSource {
    client: Arc<HttpClient>,
    api_key: Option<String>,
}

impl SpringerSource {
    pub fn new() -> Result<Self, SourceError> {
        let api_key = std::env::var("SPRINGER_API_KEY").ok();
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            api_key,
        })
    }
}

impl Default for SpringerSource {
    fn default() -> Self {
        Self::new().expect("Failed to create SpringerSource")
    }
}

#[async_trait]
impl Source for SpringerSource {
    fn id(&self) -> &str {
        "springer"
    }

    fn name(&self) -> &str {
        "Springer"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        if self.api_key.is_none() {
            return Err(SourceError::Other(
                "SPRINGER_API_KEY is not set. Get a free key at https://dev.springernature.com"
                    .to_string(),
            ));
        }

        let max_results = query.max_results.min(100);

        let url = format!(
            "{}?q={}&p={}",
            SPRINGER_API_BASE,
            urlencoding::encode(&query.query),
            max_results
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();
        let api_key = self.api_key.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            let api_key = api_key.clone();
            async move {
                let mut request = client.get(&url);

                if let Some(key) = api_key {
                    request = request.query(&[("api_key", key)]);
                }

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to search Springer: {}", e.without_url()))
                })?;

                if !response.status().is_success() {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "Springer API returned status {}: {}",
                        status, text
                    )));
                }

                let json: SpringerResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse Springer response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        let total = response.total();
        let papers: Result<Vec<Paper>, SourceError> = response
            .into_records()
            .into_iter()
            .map(|record| self.parse_result(&record))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "Springer", &query.query);
        response.total_results = Some(total);
        Ok(response)
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let url = format!(
            "{}?q=doi:\"{}\"&p=1",
            SPRINGER_API_BASE,
            urlencoding::encode(&clean_doi)
        );

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();
        let api_key = self.api_key.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            let api_key = api_key.clone();
            async move {
                let mut request = client.get(&url);

                if let Some(key) = api_key {
                    request = request.query(&[("api_key", key)]);
                }

                let response = request.send().await.map_err(|e| {
                    SourceError::Network(format!(
                        "Failed to lookup DOI in Springer: {}",
                        e.without_url()
                    ))
                })?;

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "Springer API returned status: {}",
                        response.status()
                    )));
                }

                let json: SpringerResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse Springer response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await?;

        let record = response.into_records().into_iter().next().ok_or_else(|| {
            SourceError::NotFound(format!("Paper not found in Springer: {}", doi))
        })?;

        self.parse_result(&record)
    }
}

impl SpringerSource {
    fn parse_result(&self, record: &SpringerRecord) -> Result<Paper, SourceError> {
        let id = record.doi.clone().unwrap_or_else(|| record.id.clone());
        let title = record
            .title
            .as_ref()
            .and_then(SpringerText::text)
            .unwrap_or_default();
        let abstract_text = record
            .abstract_text
            .as_ref()
            .and_then(SpringerText::text)
            .unwrap_or_default();

        let doi = record.doi.clone().unwrap_or_default();

        let authors: String = record
            .authors
            .iter()
            .filter_map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join("; ");

        let year = record.publication_date.clone().unwrap_or_default();
        let url = if !doi.is_empty() {
            format!("https://doi.org/{}", doi)
        } else {
            record
                .url
                .as_ref()
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string()
        };

        Ok(PaperBuilder::new(id, title, url, SourceType::Springer)
            .authors(&authors)
            .published_date(&year)
            .abstract_text(&abstract_text)
            .doi(&doi)
            .build())
    }
}

/// Springer API response
#[derive(Debug, Deserialize)]
struct SpringerResponse {
    #[serde(default)]
    result: Vec<SpringerResult>,
    #[serde(default)]
    records: Vec<SpringerRecord>,
}

impl SpringerResponse {
    fn total(&self) -> usize {
        self.result
            .first()
            .and_then(|r| r.total.as_ref())
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| v.as_u64().map(|n| n as usize))
            })
            .unwrap_or(0)
    }
    fn into_records(self) -> Vec<SpringerRecord> {
        if self.records.is_empty() {
            self.result
                .into_iter()
                .next()
                .map(|r| r.records)
                .unwrap_or_default()
        } else {
            self.records
        }
    }
}

#[derive(Debug, Deserialize)]
struct SpringerResult {
    total: Option<serde_json::Value>,
    #[serde(default)]
    records: Vec<SpringerRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SpringerText {
    One(String),
    Many(Vec<String>),
}
impl SpringerText {
    fn text(&self) -> Option<String> {
        match self {
            Self::One(s) => Some(s.clone()),
            Self::Many(values) => values.first().cloned(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SpringerRecord {
    #[serde(default, alias = "identifier")]
    id: String,
    doi: Option<String>,
    title: Option<SpringerText>,
    #[serde(rename = "abstract")]
    abstract_text: Option<SpringerText>,
    #[serde(alias = "publicationDate")]
    publication_date: Option<String>,
    #[serde(default, alias = "creators")]
    authors: Vec<SpringerAuthor>,
    url: Option<serde_json::Value>,
    journal: Option<SpringerJournal>,
}

#[derive(Debug, Deserialize)]
struct SpringerAuthor {
    #[serde(alias = "creator")]
    name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct SpringerJournal {
    title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_creation() {
        let source = SpringerSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = SpringerSource::new().unwrap();
        assert_eq!(source.id(), "springer");
        assert_eq!(source.name(), "Springer");
    }

    #[test]
    fn test_capabilities() {
        let source = SpringerSource::new().unwrap();
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
        let json = r#"{
            "result": [{"total": "1", "records": [{
                "id": "springer-1",
                "doi": "10.1234/mock",
                "title": ["Mock Paper Title"],
                "abstract": ["Mock abstract text."],
                "publicationDate": "2024-03-01",
                "creators": [{"creator": "Ada Lovelace"}, {"creator": "Alan Turing"}],
                "url": "https://link.springer.com/article/mock",
                "journal": {"title": "Mock Journal"}
            }]}]
        }"#;
        let response: SpringerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.total(), 1);
        assert_eq!(response.result[0].records.len(), 1);
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = SpringerSource::new().unwrap();
        let json = r#"{
            "result": [{"total": "1", "records": [{
                "id": "springer-1",
                "doi": "10.1234/mock",
                "title": ["Mock Paper Title"],
                "abstract": ["Mock abstract text."],
                "publicationDate": "2024-03-01",
                "creators": [{"creator": "Ada Lovelace"}, {"creator": "Alan Turing"}],
                "url": "https://link.springer.com/article/mock",
                "journal": {"title": "Mock Journal"}
            }]}]
        }"#;
        let response: SpringerResponse = serde_json::from_str(json).unwrap();
        let paper = source.parse_result(&response.result[0].records[0]).unwrap();
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Ada Lovelace; Alan Turing");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.source, crate::models::SourceType::Springer);
        assert_eq!(paper.paper_id, "10.1234/mock");
        assert_eq!(paper.url, "https://doi.org/10.1234/mock");
        assert_eq!(paper.published_date.as_deref(), Some("2024-03-01"));
    }
}
