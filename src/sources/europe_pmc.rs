//! EuropePMC research source implementation using their REST API.
//!
//! EuropePMC indexes PubMed, PMC, and preprints from bioRxiv/medRxiv.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

/// EuropePMC REST API base URL
const EUROPE_PMC_SEARCH_URL: &str = "https://www.ebi.ac.uk/europepmc/webservices/rest/search";

/// EuropePMC research source
///
/// Uses EuropePMC's REST API for searching and fetching biomedical literature.
/// EuropePMC indexes PubMed, PMC, and preprints from bioRxiv/medRxiv.
#[derive(Debug, Clone)]
pub struct EuropePmcSource {
    client: Arc<HttpClient>,
}

impl EuropePmcSource {
    /// Create a new EuropePMC source
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

    /// Build search URL
    fn build_search_url(&self, query: &SearchQuery) -> String {
        let mut text_query = query.query.clone();
        // Full-text filter: restrict to works whose full text is indexed.
        if query.filters.get("fulltext").map(|v| v == "true") == Some(true) {
            text_query.push_str(" AND HAS_FT:y");
        }
        let mut params = vec![
            ("query".to_string(), text_query),
            ("resultType".to_string(), "core".to_string()),
            ("format".to_string(), "json".to_string()),
            ("pageSize".to_string(), query.max_results.to_string()),
            ("cursorMark".to_string(), "*".to_string()),
        ];

        // Add year filter
        if let Some(year) = &query.year {
            if year.contains('-') {
                let parts: Vec<&str> = year.splitn(2, '-').collect();
                if parts.len() == 2 {
                    params.push(("fromDate".to_string(), format!("{}-01-01", parts[0])));
                    params.push(("toDate".to_string(), format!("{}-12-31", parts[1])));
                }
            } else if year.ends_with('-') {
                // From year
                let y = year.trim_end_matches('-');
                params.push(("fromDate".to_string(), format!("{}-01-01", y)));
            } else if year.starts_with('-') {
                // Until year
                let y = year.trim_start_matches('-');
                params.push(("toDate".to_string(), format!("{}-12-31", y)));
            } else if year.len() == 4 {
                // Single year
                params.push(("fromDate".to_string(), format!("{}-01-01", year)));
                params.push(("toDate".to_string(), format!("{}-12-31", year)));
            }
        }

        // Add author filter
        if let Some(author) = &query.author {
            params.push(("author".to_string(), author.clone()));
        }

        params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&")
    }

    /// Parse search response JSON
    fn parse_search_response(json: &str) -> Result<SearchResult, SourceError> {
        serde_json::from_str(json)
            .map_err(|e| SourceError::Parse(format!("Failed to parse EuropePMC JSON: {}", e)))
    }

    /// Parse a single result into a Paper
    fn parse_result(result: &SearchResultItem) -> Paper {
        let id = result
            .pubmed_id
            .as_ref()
            .or(result.doi.as_ref())
            .or(result.id.as_ref())
            .cloned()
            .unwrap_or_else(|| result.external_id.clone().unwrap_or_default());

        let title = result.title.clone().unwrap_or_default();
        let url = result
            .full_text_url
            .as_ref()
            .and_then(|urls| urls.first())
            .cloned()
            .unwrap_or_else(|| {
                if let Some(pmid) = &result.pubmed_id {
                    format!("https://europepmc.org/article/med/{}", pmid)
                } else {
                    format!(
                        "https://europepmc.org/search?query={}",
                        urlencoding::encode(&title)
                    )
                }
            });

        let authors = result.author_string.as_ref().cloned().unwrap_or_default();

        let abstract_text = result.abstract_text.clone().unwrap_or_default();

        let published_date = result
            .published_date
            .as_ref()
            .cloned()
            .or_else(|| {
                result.journal_info.as_ref().and_then(|ji| {
                    ji.journal_volume.as_ref().map(|_| {
                        // Return a placeholder if we only have volume info
                        "".to_string()
                    })
                })
            })
            .unwrap_or_default();

        PaperBuilder::new(id, title, url, SourceType::EuropePMC)
            .authors(authors)
            .abstract_text(abstract_text)
            .doi(result.doi.clone().unwrap_or_default())
            .published_date(published_date)
            .build()
    }
}

impl Default for EuropePmcSource {
    fn default() -> Self {
        Self::new().expect("Failed to create EuropePmcSource")
    }
}

#[async_trait]
impl Source for EuropePmcSource {
    fn id(&self) -> &str {
        "europe_pmc"
    }

    fn name(&self) -> &str {
        "EuropePMC"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::READ
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let search_url = format!("{}?{}", EUROPE_PMC_SEARCH_URL, self.build_search_url(query));

        let client = Arc::clone(&self.client);
        let search_url_for_retry = search_url.clone();

        let json = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = search_url_for_retry.clone();
            async move {
                let response = client.get(&url).send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to search EuropePMC: {}", e))
                })?;

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "EuropePMC API returned status: {}",
                        response.status()
                    )));
                }

                response
                    .text()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to read response: {}", e)))
            }
        })
        .await?;

        let search_result = Self::parse_search_response(&json)?;

        let papers = search_result
            .result_list
            .result
            .iter()
            .map(Self::parse_result)
            .collect();

        Ok(SearchResponse::new(papers, "EuropePMC", &query.query))
    }
}

/// Search result wrapper
#[derive(Debug, Deserialize)]
#[allow(non_snake_case, dead_code)]
struct SearchResult {
    #[allow(dead_code)]
    version: String,
    #[allow(dead_code)]
    hitCount: u32,
    #[allow(dead_code)]
    request: SearchRequest,
    #[serde(rename = "resultList")]
    result_list: ResultList,
}

/// Search request info
#[derive(Debug, Deserialize)]
#[allow(non_snake_case, dead_code)]
struct SearchRequest {
    #[allow(dead_code)]
    query: String,
    #[allow(dead_code)]
    resultType: String,
    #[allow(dead_code)]
    format: String,
    #[allow(dead_code)]
    pageSize: u32,
}

/// List of results
#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct ResultList {
    result: Vec<SearchResultItem>,
}

/// Individual search result
#[derive(Debug, Deserialize)]
#[allow(non_snake_case, dead_code)]
struct SearchResultItem {
    #[serde(default)]
    pubmed_id: Option<String>,
    #[serde(default)]
    pmc_id: Option<String>,
    #[serde(default)]
    doi: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    author_string: Option<String>,
    #[serde(default)]
    abstract_text: Option<String>,
    #[serde(default)]
    published_date: Option<String>,
    #[serde(default)]
    journal_info: Option<JournalInfo>,
    #[serde(default)]
    external_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    full_text_url: Option<Vec<String>>,
}

/// Journal information
#[derive(Debug, Deserialize)]
#[allow(non_snake_case, dead_code)]
struct JournalInfo {
    journal_volume: Option<String>,
    #[allow(dead_code)]
    journal_issue: Option<String>,
    #[allow(dead_code)]
    pub_date: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_search_url() {
        let source = EuropePmcSource::new().unwrap();
        let query = SearchQuery::new("CRISPR").max_results(10);
        let url = source.build_search_url(&query);

        assert!(url.contains("query=CRISPR"));
        assert!(url.contains("pageSize=10"));
        assert!(url.contains("format=json"));
    }

    #[test]
    fn test_build_search_url_with_year() {
        let source = EuropePmcSource::new().unwrap();
        let query = SearchQuery::new("cancer").year("2020");
        let url = source.build_search_url(&query);

        assert!(url.contains("fromDate=2020-01-01"));
        assert!(url.contains("toDate=2020-12-31"));
    }

    #[test]
    fn test_build_search_url_with_year_range() {
        let source = EuropePmcSource::new().unwrap();
        let query = SearchQuery::new("cancer").year("2015-2020");
        let url = source.build_search_url(&query);

        assert!(url.contains("fromDate=2015-01-01"));
        assert!(url.contains("toDate=2020-12-31"));
    }

    #[test]
    fn test_build_search_url_with_author() {
        let source = EuropePmcSource::new().unwrap();
        let query = SearchQuery::new("cancer").author("Smith J");
        let url = source.build_search_url(&query);

        assert!(url.contains("author=Smith%20J"));
    }
}
