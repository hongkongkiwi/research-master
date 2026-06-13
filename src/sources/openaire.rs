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

        self.parse_result(&result)
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
            .unwrap_or_default();
        let abstract_text = result
            .description
            .as_ref()
            .and_then(|descriptions| descriptions.first_cloned())
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
            .and_then(|pid| pid.value.clone())
            .unwrap_or_default();

        let authors: String = result
            .creator
            .as_ref()
            .map(OneOrMany::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|creator| creator.name.clone())
            .collect::<Vec<_>>()
            .join("; ");

        let published_date = result
            .dateofacceptance
            .as_ref()
            .and_then(|date| date.value.clone())
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
                    .cloned()
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
                .filter_map(|pid| pid.value.clone())
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

#[derive(Debug, Deserialize, Default)]
struct OpenairePublication {
    #[serde(default)]
    title: Option<OneOrMany<String>>,
    #[serde(default)]
    creator: Option<OneOrMany<OpenaireCreator>>,
    #[serde(default)]
    description: Option<OneOrMany<String>>,
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
    #[serde(rename = "$name")]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenairePid {
    #[serde(rename = "@classid")]
    classid: Option<String>,
    #[serde(rename = "$")]
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenaireValue {
    #[serde(rename = "$")]
    value: Option<String>,
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
    url: Option<OneOrMany<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
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

    #[test]
    fn test_source_creation() {
        let source = OpenaireSource::new();
        assert!(source.is_ok());
    }
}
