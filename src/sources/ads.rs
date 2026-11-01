//! NASA Astrophysics Data System source.
//!
//! ADS requires a free API token from <https://ui.adsabs.harvard.edu/user/settings/token>.
//! Without `ADS_API_KEY` the source stays registered but searches return nothing,
//! matching the other key-gated sources.
//! API documentation: <https://ui.adsabs.harvard.edu/help/api/>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const ADS_API: &str = "https://api.adsabs.harvard.edu/v1/search/query";

/// NASA ADS source.
#[derive(Debug, Clone)]
pub struct AdsSource {
    client: Arc<HttpClient>,
    api_key: Option<String>,
}

impl AdsSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
            api_key: std::env::var("ADS_API_KEY").ok().filter(|k| !k.is_empty()),
        })
    }
}

impl Default for AdsSource {
    fn default() -> Self {
        Self::new().expect("Failed to create AdsSource")
    }
}

#[async_trait]
impl Source for AdsSource {
    fn id(&self) -> &str {
        "ads"
    }

    fn name(&self) -> &str {
        "NASA ADS"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let Some(api_key) = &self.api_key else {
            tracing::debug!("ADS_API_KEY not set - skipping search");
            return Ok(SearchResponse::new(Vec::new(), "NASA ADS", &query.query));
        };

        let rows = query.max_results.clamp(1, 100);
        let mut q = query.query.clone();
        if let Some(year) = query.year.as_deref().and_then(crate::models::year_bounds) {
            let (from, to) = year;
            if from == to {
                q.push_str(&format!(" year:{from}"));
            } else {
                q.push_str(&format!(" year:{from}-{to}"));
            }
        }
        let url = format!(
            "{ADS_API}?q={}&rows={rows}&fl=bibcode,title,author,abstract,doi,year,property",
            urlencoding::encode(&q)
        );

        let body = get_json(&self.client, &url, api_key).await?;
        let response: AdsResponse = serde_json::from_str(&body)?;
        let total = response.response.num_found;
        let papers = response
            .response
            .docs
            .iter()
            .map(parse_doc)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SearchResponse::new(papers, "NASA ADS", &query.query).total_results(total))
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| SourceError::Api("ADS_API_KEY is not set".to_string()))?;
        let doi = doi
            .trim()
            .trim_start_matches("https://doi.org/")
            .trim_start_matches("doi:");
        let url = format!(
            "{ADS_API}?q=doi:{}&rows=1&fl=bibcode,title,author,abstract,doi,year,property",
            urlencoding::encode(doi)
        );
        let body = get_json(&self.client, &url, api_key).await?;
        let response: AdsResponse = serde_json::from_str(&body)?;
        response
            .response
            .docs
            .first()
            .map(parse_doc)
            .transpose()?
            .ok_or_else(|| SourceError::NotFound(format!("DOI {doi} not found in NASA ADS")))
    }
}

async fn get_json(client: &HttpClient, url: &str, api_key: &str) -> Result<String, SourceError> {
    let client = Arc::new(client.clone());
    let url = url.to_string();
    let api_key = api_key.to_string();
    with_retry(api_retry_config(), || {
        let client = Arc::clone(&client);
        let url = url.clone();
        let api_key = api_key.clone();
        async move {
            let response = client
                .get(&url)
                .header("Authorization", format!("Bearer {api_key}"))
                .send()
                .await
                .map_err(|e| SourceError::Network(format!("Failed to query NASA ADS: {e}")))?;
            if !response.status().is_success() {
                return Err(SourceError::Api(format!(
                    "NASA ADS returned {}",
                    response.status()
                )));
            }
            response
                .text()
                .await
                .map_err(|e| SourceError::Network(format!("Failed to read NASA ADS response: {e}")))
        }
    })
    .await
}

fn parse_doc(doc: &AdsDoc) -> Result<Paper, SourceError> {
    let title = doc
        .title
        .iter()
        .find(|t| !t.is_empty())
        .cloned()
        .unwrap_or_else(|| "Untitled".to_string());
    let authors = doc.author.join("; ");
    let doi = doc.doi.iter().find(|d| !d.is_empty()).cloned();
    let bibcode = doc.bibcode.clone();
    let url = doi
        .as_ref()
        .map(|d| format!("https://doi.org/{d}"))
        .unwrap_or_else(|| format!("https://ui.adsabs.harvard.edu/abs/{bibcode}"));
    // ADS open-access flags do not identify the work's reuse license.

    let mut paper = PaperBuilder::new(bibcode, title, url, SourceType::Ads)
        .authors(authors)
        .abstract_text(doc.r#abstract.clone().unwrap_or_default())
        .published_date(doc.year.clone().unwrap_or_default());
    if let Some(doi) = doi {
        paper = paper.doi(doi);
    }
    Ok(paper.build())
}

#[derive(Debug, Deserialize)]
struct AdsResponse {
    response: AdsInner,
}

#[derive(Debug, Deserialize)]
struct AdsInner {
    #[serde(rename = "numFound")]
    num_found: usize,
    docs: Vec<AdsDoc>,
}

#[derive(Debug, Deserialize)]
struct AdsDoc {
    bibcode: String,
    #[serde(default)]
    title: Vec<String>,
    #[serde(default)]
    author: Vec<String>,
    r#abstract: Option<String>,
    #[serde(default)]
    doi: Vec<String>,
    year: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_ads_record() {
        let json = r#"{
            "bibcode": "2024ApJ...900..123A",
            "title": ["A Survey of Galaxies"],
            "author": ["Lovelace, Ada", "Turing, Alan"],
            "abstract": "An abstract.",
            "doi": ["10.3847/1538-4357/test"],
            "year": "2024",
            "property": ["REFEREED", "PUB_OPENACCESS"]
        }"#;
        let doc: AdsDoc = serde_json::from_str(json).unwrap();
        let paper = parse_doc(&doc).unwrap();
        assert_eq!(paper.title, "A Survey of Galaxies");
        assert_eq!(paper.authors, "Lovelace, Ada; Turing, Alan");
        assert!(paper.license.is_none());
        assert_eq!(paper.paper_id, "2024ApJ...900..123A");
        assert_eq!(paper.source, SourceType::Ads);
    }

    #[test]
    fn skips_search_without_a_key() {
        let source = AdsSource {
            client: Arc::new(HttpClient::new().unwrap()),
            api_key: None,
        };
        assert!(source.api_key.is_none());
    }
}
