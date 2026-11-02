//! OpenAIRE research source implementation.
//!
//! Uses the OpenAIRE Graph API for searching and retrieving research publications.
//! API documentation: <https://graph.openaire.eu/develop/>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{
    DownloadRequest, DownloadResult, Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType,
};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const OPENAIRE_API_BASE: &str = "https://api.openaire.eu";

/// OpenAIRE research source
///
/// Uses the OpenAIRE Graph API for searching research publications across
/// open access repositories. OpenAIRE is free and requires no API key.
#[derive(Debug, Clone)]
pub struct OpenaireSource {
    client: Arc<HttpClient>,
}

impl OpenaireSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }

    fn build_search_url(&self, field: &str, value: &str, size: usize) -> String {
        format!(
            "{}/search/publications?format=json&page=1&size={}&{}={}",
            OPENAIRE_API_BASE,
            size,
            field,
            urlencoding::encode(value)
        )
    }

    fn is_pdf_url(url: &str) -> bool {
        let lower = url.to_lowercase();
        lower.ends_with(".pdf") || lower.contains(".pdf?") || lower.contains("/pdf/")
    }
}

impl Default for OpenaireSource {
    fn default() -> Self {
        Self::new().expect("Failed to create OpenaireSource")
    }
}

#[async_trait]
impl Source for OpenaireSource {
    fn id(&self) -> &str {
        "openaire"
    }

    fn name(&self) -> &str {
        "OpenAIRE"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOWNLOAD | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let max_results = query.max_results.min(100);
        let url = self.build_search_url("title", &query.query, max_results);

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let response = client.get(&url).send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to search OpenAIRE: {}", e))
                })?;

                if !response.status().is_success() {
                    let status = response.status();
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        tracing::debug!("OpenAIRE API rate-limited - returning empty results");
                        return Err(SourceError::Api("OpenAIRE rate-limited".to_string()));
                    }
                    let text = response.text().await.unwrap_or_default();
                    return Err(SourceError::Api(format!(
                        "OpenAIRE API returned status {}: {}",
                        status, text
                    )));
                }

                let json: OpenaireResponse = response.json().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse OpenAIRE response: {}", e))
                })?;

                Ok(json)
            }
        })
        .await;

        let response = match response {
            Ok(r) => r,
            Err(SourceError::Api(msg)) if msg.contains("rate-limited") => {
                tracing::debug!("OpenAIRE rate-limited - returning empty results");
                return Ok(SearchResponse::new(vec![], "OpenAIRE", &query.query));
            }
            Err(e) => return Err(e),
        };

        let result_items = response.response.results.result.into_vec();
        let total = result_items.len();
        let papers: Result<Vec<Paper>, SourceError> = result_items
            .into_iter()
            .map(|item| self.parse_result(&item))
            .collect();

        let papers = papers?;
        let mut response = SearchResponse::new(papers, "OpenAIRE", &query.query);
        response.total_results = Some(total);
        Ok(response)
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let url = self.build_search_url("doi", &clean_doi, 1);

        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let response = client.get(&url).send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to lookup DOI in OpenAIRE: {}", e))
                })?;

                if !response.status().is_success() {
                    return Err(SourceError::NotFound(format!(
                        "Paper not found in OpenAIRE: {}",
                        doi
                    )));
                }

                response.json::<OpenaireResponse>().await.map_err(|e| {
                    SourceError::Parse(format!("Failed to parse OpenAIRE response: {}", e))
                })
            }
        })
        .await?;

        let result = response
            .response
            .results
            .result
            .into_vec()
            .into_iter()
            .next()
            .ok_or_else(|| {
                SourceError::NotFound(format!("Paper not found in OpenAIRE: {}", doi))
            })?;

        let paper = self.parse_result(&result)?;

        // Never return a DOI-less record as a junk-success to the
        // lookup_by_doi fan-out.
        if paper.doi.as_deref().map(str::trim).unwrap_or("").is_empty() {
            return Err(SourceError::NotFound(format!(
                "OpenAIRE record for DOI {} has no DOI pid; skipping",
                doi
            )));
        }

        Ok(paper)
    }

    async fn download(&self, request: &DownloadRequest) -> Result<DownloadResult, SourceError> {
        let lookup_id = request.doi.as_deref().unwrap_or(&request.paper_id);
        let paper = self.get_by_doi(lookup_id).await?;

        let pdf_url = paper
            .pdf_url
            .as_ref()
            .or_else(|| {
                if Self::is_pdf_url(&paper.url) {
                    Some(&paper.url)
                } else {
                    None
                }
            })
            .ok_or_else(|| SourceError::NotFound("No PDF available from OpenAIRE".to_string()))?;

        let response =
            self.client.get(pdf_url).send().await.map_err(|e| {
                SourceError::Network(format!("Failed to download OpenAIRE PDF: {}", e))
            })?;

        if !response.status().is_success() {
            return Err(SourceError::Api(format!(
                "OpenAIRE PDF download returned status: {}",
                response.status()
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to read OpenAIRE PDF: {}", e)))?;

        std::fs::create_dir_all(&request.save_path).map_err(|e| {
            SourceError::Io(std::io::Error::other(format!(
                "Failed to create directory: {}",
                e
            )))
        })?;

        let filename_base = paper
            .doi
            .as_deref()
            .unwrap_or(&request.paper_id)
            .replace(['/', ':'], "_");
        let filename = format!("{}.pdf", filename_base);
        let path = std::path::Path::new(&request.save_path).join(&filename);

        std::fs::write(&path, bytes.as_ref()).map_err(SourceError::Io)?;

        Ok(DownloadResult::success(
            path.to_string_lossy().to_string(),
            bytes.len() as u64,
        ))
    }
}

impl OpenaireSource {
    fn parse_result(&self, item: &OpenaireResultItem) -> Result<Paper, SourceError> {
        let result = &item.metadata.oaf_entity.oaf_result;

        let title = result
            .title
            .as_ref()
            .and_then(|titles| titles.first_cloned())
            .and_then(|title| title.text())
            .unwrap_or_default();
        let abstract_text = result
            .description
            .as_ref()
            .and_then(|descriptions| descriptions.first_cloned())
            .and_then(|description| description.text())
            .unwrap_or_default();

        let pids = result.pid.as_ref().map(OneOrMany::as_slice).unwrap_or(&[]);
        let doi = pids
            .iter()
            .find(|pid| {
                pid.classid
                    .as_deref()
                    .map(|classid| classid.eq_ignore_ascii_case("doi"))
                    .unwrap_or(false)
            })
            .and_then(|pid| value_text(pid.value.as_ref()))
            .unwrap_or_default();

        let authors: String = result
            .creator
            .as_ref()
            .map(OneOrMany::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|creator| creator.display_name())
            .collect::<Vec<_>>()
            .join("; ");

        let published_date = result
            .dateofacceptance
            .as_ref()
            .and_then(|date| date.text())
            .unwrap_or_default();

        let urls: Vec<String> = result
            .instance
            .as_ref()
            .map(OneOrMany::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|instance| instance.webresource.as_ref())
            .flat_map(|webresource| {
                webresource
                    .url
                    .as_ref()
                    .map(OneOrMany::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|value| value.text())
            })
            .collect();

        let pdf_url = urls.iter().find(|url| Self::is_pdf_url(url)).cloned();
        let url = urls.first().cloned().unwrap_or_else(|| {
            if !doi.is_empty() {
                format!("https://doi.org/{}", doi)
            } else {
                format!(
                    "https://explore.openaire.eu/search/publication?keyword={}",
                    urlencoding::encode(&title)
                )
            }
        });

        let id = if !doi.is_empty() {
            doi.clone()
        } else {
            pids.iter()
                .filter_map(|pid| value_text(pid.value.as_ref()))
                .find(|pid| !pid.is_empty())
                .unwrap_or_else(|| format!("openaire:{}", urlencoding::encode(&title)))
        };

        let mut builder = PaperBuilder::new(id, title, url, SourceType::OpenAIRE)
            .authors(&authors)
            .published_date(&published_date)
            .abstract_text(&abstract_text)
            .doi(&doi);

        if let Some(pdf_url) = pdf_url {
            builder = builder.pdf_url(pdf_url);
        }

        if let Some(access_right) = result
            .bestaccessright
            .as_ref()
            .and_then(|access| access.classid.clone())
        {
            builder = builder.extra("access_right", serde_json::Value::String(access_right));
        }

        Ok(builder.build())
    }
}

/// OpenAIRE API response
#[derive(Debug, Deserialize, Default)]
struct OpenaireResponse {
    #[serde(default)]
    response: OpenaireResponseBody,
}

#[derive(Debug, Deserialize, Default)]
struct OpenaireResponseBody {
    #[serde(default)]
    results: OpenaireResults,
}

#[derive(Debug, Deserialize, Default)]
struct OpenaireResults {
    #[serde(default)]
    result: OneOrMany<OpenaireResultItem>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenaireResultItem {
    #[serde(default)]
    metadata: OpenaireMetadata,
}

#[derive(Debug, Deserialize, Default)]
struct OpenaireMetadata {
    #[serde(rename = "oaf:entity", default)]
    oaf_entity: OpenaireEntity,
}

#[derive(Debug, Deserialize, Default)]
struct OpenaireEntity {
    #[serde(rename = "oaf:result", default)]
    oaf_result: OpenairePublication,
}

/// Text-carrying OpenAIRE field: the legacy JSON API puts the content under
/// "$", either as a string or a number (e.g. numeric pmid entries).
#[derive(Debug, Clone, Deserialize)]
struct OpenaireValue {
    #[serde(rename = "$", default)]
    value: Option<serde_json::Value>,
}

impl OpenaireValue {
    fn text(&self) -> Option<String> {
        value_text(self.value.as_ref())
    }
}

fn value_text(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[derive(Debug, Deserialize, Default)]
struct OpenairePublication {
    #[serde(default)]
    title: Option<OneOrMany<OpenaireValue>>,
    #[serde(default)]
    creator: Option<OneOrMany<OpenaireCreator>>,
    #[serde(default)]
    description: Option<OneOrMany<OpenaireValue>>,
    #[serde(default)]
    pid: Option<OneOrMany<OpenairePid>>,
    #[serde(default)]
    dateofacceptance: Option<OpenaireValue>,
    #[serde(default)]
    bestaccessright: Option<OpenaireAccessRight>,
    #[serde(default)]
    instance: Option<OneOrMany<OpenaireInstance>>,
}

#[derive(Debug, Deserialize)]
struct OpenaireCreator {
    #[serde(default, rename = "$")]
    full: Option<serde_json::Value>,
    #[serde(default, rename = "@name")]
    given: Option<String>,
    #[serde(default, rename = "@surname")]
    surname: Option<String>,
}

impl OpenaireCreator {
    fn display_name(&self) -> Option<String> {
        if let Some(full) = value_text(self.full.as_ref()) {
            return Some(full);
        }
        match (&self.given, &self.surname) {
            (Some(given), Some(surname)) => Some(format!("{} {}", given, surname)),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenairePid {
    #[serde(rename = "@classid")]
    classid: Option<String>,
    #[serde(rename = "$", default)]
    value: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct OpenaireAccessRight {
    #[serde(rename = "@classid")]
    classid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenaireInstance {
    webresource: Option<OpenaireWebResource>,
}

#[derive(Debug, Deserialize)]
struct OpenaireWebResource {
    url: Option<OneOrMany<OpenaireValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    // Many MUST precede One: serde derive structs also accept sequence form
    // (visit_seq), so with One first an array like `[item]` matches One(T)
    // as a 1-element seq, feeding the item object into the struct's first
    // field and silently defaulting everything else.
    Many(Vec<T>),
    One(T),
}

impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        OneOrMany::Many(Vec::new())
    }
}

impl<T> OneOrMany<T> {
    fn as_slice(&self) -> &[T] {
        match self {
            OneOrMany::One(value) => std::slice::from_ref(value),
            OneOrMany::Many(values) => values.as_slice(),
        }
    }

    fn into_vec(self) -> Vec<T> {
        match self {
            OneOrMany::One(value) => vec![value],
            OneOrMany::Many(values) => values,
        }
    }
}

impl<T: Clone> OneOrMany<T> {
    fn first_cloned(&self) -> Option<T> {
        match self {
            OneOrMany::One(value) => Some(value.clone()),
            OneOrMany::Many(values) => values.first().cloned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_json() -> &'static str {
        r#"{
            "response": {
                "results": {
                    "result": {
                        "metadata": {
                            "oaf:entity": {
                                "oaf:result": {
                                    "title": [
                                        {"@classid": "main title", "$": "Mock Paper Title"},
                                        {"$": "mock paper title"}
                                    ],
                                    "creator": {
                                        "@rank": "1",
                                        "@name": "Ada",
                                        "@surname": "Lovelace",
                                        "$": "Lovelace, Ada"
                                    },
                                    "description": {"$": "Mock abstract text."},
                                    "pid": [
                                        {"@classid": "doi", "$": "10.1234/mock"},
                                        {"@classid": "pmid", "$": 424242}
                                    ],
                                    "dateofacceptance": {"$": "2024-05-01"},
                                    "bestaccessright": {"@classid": "OPEN"},
                                    "instance": {
                                        "webresource": {
                                            "url": {"$": "https://repo.example/mock.pdf"}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }"#
    }

    #[test]
    fn test_source_creation() {
        let source = OpenaireSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_source_metadata() {
        let source = OpenaireSource::new().unwrap();
        assert_eq!(source.id(), "openaire");
        assert_eq!(source.name(), "OpenAIRE");
    }

    #[test]
    fn test_capabilities() {
        let source = OpenaireSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::DOWNLOAD));
        assert!(caps.contains(SourceCapabilities::DOI_LOOKUP));
        assert_eq!(
            caps,
            SourceCapabilities::SEARCH
                | SourceCapabilities::DOWNLOAD
                | SourceCapabilities::DOI_LOOKUP
        );
    }

    #[test]
    fn test_response_parsing_from_mock_json() {
        let response: OpenaireResponse = serde_json::from_str(mock_json()).unwrap();
        let results = response.response.results.result.as_slice();
        assert_eq!(results.len(), 1);
        let publication = &results[0].metadata.oaf_entity.oaf_result;
        assert_eq!(
            publication
                .title
                .as_ref()
                .unwrap()
                .first_cloned()
                .and_then(|title| title.text()),
            Some("Mock Paper Title".to_string())
        );
        let pids = publication.pid.as_ref().unwrap().as_slice();
        assert_eq!(
            pids.iter()
                .find(|pid| pid.classid.as_deref() == Some("pmid"))
                .and_then(|pid| value_text(pid.value.as_ref())),
            Some("424242".to_string())
        );
    }

    #[test]
    fn test_parse_result_maps_response_fields() {
        let source = OpenaireSource::new().unwrap();
        let response: OpenaireResponse = serde_json::from_str(mock_json()).unwrap();
        let paper = source
            .parse_result(&response.response.results.result.as_slice()[0])
            .unwrap();
        assert_eq!(paper.paper_id, "10.1234/mock");
        assert_eq!(paper.title, "Mock Paper Title");
        assert_eq!(paper.authors, "Lovelace, Ada");
        assert_eq!(paper.r#abstract, "Mock abstract text.");
        assert_eq!(paper.doi.as_deref(), Some("10.1234/mock"));
        assert_eq!(paper.url, "https://repo.example/mock.pdf");
        assert_eq!(paper.published_date.as_deref(), Some("2024-05-01"));
        assert_eq!(
            paper.pdf_url.as_deref(),
            Some("https://repo.example/mock.pdf")
        );
        assert_eq!(paper.source, crate::models::SourceType::OpenAIRE);
        assert_eq!(
            paper
                .extra
                .as_ref()
                .and_then(|m| m.get("access_right"))
                .and_then(|v| v.as_str()),
            Some("OPEN")
        );
    }

    #[test]
    fn test_helper_functions() {
        let source = OpenaireSource::new().unwrap();
        let url = source.build_search_url("title", "quantum networks", 25);
        assert!(url.contains("format=json"));
        assert!(url.contains("size=25"));
        assert!(url.contains("title=quantum%20networks"));

        assert!(OpenaireSource::is_pdf_url("https://example.org/paper.pdf"));
        assert!(OpenaireSource::is_pdf_url(
            "https://example.org/download.pdf?version=1"
        ));
        assert!(OpenaireSource::is_pdf_url("https://example.org/pdf/123"));
        assert!(!OpenaireSource::is_pdf_url("https://example.org/landing"));

        let one = OneOrMany::One("one".to_string());
        assert_eq!(one.as_slice().len(), 1);
        assert_eq!(one.first_cloned().as_deref(), Some("one"));
        let many = OneOrMany::Many(vec!["one".to_string(), "two".to_string()]);
        assert_eq!(many.as_slice().len(), 2);
        assert_eq!(many.into_vec(), vec!["one".to_string(), "two".to_string()]);

        let text = OpenaireValue {
            value: Some(serde_json::Value::String("text".to_string())),
        };
        assert_eq!(text.text().as_deref(), Some("text"));
        let numeric = OpenaireValue {
            value: Some(serde_json::json!(26771483)),
        };
        assert_eq!(numeric.text().as_deref(), Some("26771483"));
        assert_eq!(OpenaireValue { value: None }.text(), None);
    }

    #[test]
    fn test_creator_display_name_fallbacks() {
        let with_full: OpenaireCreator = serde_json::from_str(
            r#"{"@name": "Ada", "@surname": "Lovelace", "$": "Lovelace, Ada"}"#,
        )
        .unwrap();
        assert_eq!(with_full.display_name().as_deref(), Some("Lovelace, Ada"));

        let composed: OpenaireCreator =
            serde_json::from_str(r#"{"@name": "Ada", "@surname": "Lovelace"}"#).unwrap();
        assert_eq!(composed.display_name().as_deref(), Some("Ada Lovelace"));

        let none: OpenaireCreator = serde_json::from_str("{}").unwrap();
        assert_eq!(none.display_name(), None);
    }

    #[test]
    fn test_parse_result_joins_multiple_creators() {
        let raw = r#"{
            "response": {
                "results": {
                    "result": {
                        "metadata": {
                            "oaf:entity": {
                                "oaf:result": {
                                    "title": [{"$": "Multi Author Paper"}],
                                    "creator": [
                                        {"$": "Lovelace, Ada"},
                                        {"@name": "Alan", "@surname": "Turing"}
                                    ]
                                }
                            }
                        }
                    }
                }
            }
        }"#;
        let source = OpenaireSource::new().unwrap();
        let response: OpenaireResponse = serde_json::from_str(raw).unwrap();
        let paper = source
            .parse_result(&response.response.results.result.as_slice()[0])
            .unwrap();
        assert_eq!(paper.title, "Multi Author Paper");
        assert_eq!(paper.authors, "Lovelace, Ada; Alan Turing");
        assert_eq!(paper.paper_id, "openaire:Multi%20Author%20Paper");
    }

    #[test]
    fn test_live_captured_doi_response_parses() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/openaire_doi_response.json");
        let raw = std::fs::read_to_string(path).expect("OpenAIRE fixture should exist");
        let source = OpenaireSource::new().unwrap();
        let response: OpenaireResponse =
            serde_json::from_str(&raw).expect("live OpenAIRE DOI response should deserialize");
        let item = response
            .response
            .results
            .result
            .as_slice()
            .first()
            .expect("one result");
        let paper = source.parse_result(item).unwrap();
        assert_eq!(paper.title, "The Heroes of CRISPR");
        assert_eq!(paper.doi.as_deref(), Some("10.1016/j.cell.2015.12.041"));
    }
}
