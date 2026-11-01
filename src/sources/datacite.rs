//! DataCite research source.
//!
//! Uses the public DataCite REST API, which serves DOI metadata without
//! authentication. Creating or updating records needs a member account; this
//! source only reads.
//! API documentation: <https://support.datacite.org/docs/api>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const DATACITE_API: &str = "https://api.datacite.org/dois";

/// DataCite DOI registry source.
///
/// Free, no API key. DataCite asks clients that make repeated requests to send
/// a User-Agent that names the client and a contact address; the shared HTTP
/// client already sets one.
#[derive(Debug, Clone)]
pub struct DataCiteSource {
    client: Arc<HttpClient>,
}

impl DataCiteSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }
}

impl Default for DataCiteSource {
    fn default() -> Self {
        Self::new().expect("Failed to create DataCiteSource")
    }
}

#[async_trait]
impl Source for DataCiteSource {
    fn id(&self) -> &str {
        "datacite"
    }

    fn name(&self) -> &str {
        "DataCite"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let page_size = query.max_results.clamp(1, 100);
        let terms = match query.year.as_deref().and_then(crate::models::year_bounds) {
            Some((from, to)) => {
                let text = &query.query;
                if from == to {
                    format!("{text} publicationYear:{from}")
                } else {
                    format!("{text} publicationYear:[{from} TO {to}]")
                }
            }
            None => query.query.clone(),
        };
        let url = format!(
            "{}?query={}&page%5Bsize%5D={}",
            DATACITE_API,
            urlencoding::encode(&terms),
            page_size
        );

        let body = get_json(&self.client, &url, "search DataCite").await?;
        let response: DataCiteList = serde_json::from_str(&body)?;
        let total = response.meta.and_then(|m| m.total).unwrap_or(0);
        let papers = response
            .data
            .iter()
            .map(parse_work)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(SearchResponse::new(papers, "DataCite", &query.query).total_results(total))
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let doi = doi
            .trim()
            .trim_start_matches("https://doi.org/")
            .trim_start_matches("doi:");
        let url = format!("{}/{}", DATACITE_API, urlencoding::encode(doi));
        let body = get_json(&self.client, &url, "look up a DataCite DOI").await?;
        let response: DataCiteItem = serde_json::from_str(&body)?;
        parse_work(&response.data)
    }
}

async fn get_json(client: &HttpClient, url: &str, what: &str) -> Result<String, SourceError> {
    let client = Arc::new(client.clone());
    let url = url.to_string();
    let what = what.to_string();
    with_retry(api_retry_config(), || {
        let client = Arc::clone(&client);
        let url = url.clone();
        let what = what.clone();
        async move {
            let response = client
                .get(&url)
                .header("Accept", "application/vnd.api+json")
                .send()
                .await
                .map_err(|e| SourceError::Network(format!("Failed to {}: {}", what, e)))?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(SourceError::NotFound(format!(
                    "Not found in DataCite: {url}"
                )));
            }
            if !response.status().is_success() {
                return Err(SourceError::Api(format!(
                    "DataCite returned {}",
                    response.status()
                )));
            }
            response
                .text()
                .await
                .map_err(|e| SourceError::Network(format!("Failed to read DataCite response: {e}")))
        }
    })
    .await
}

fn parse_work(item: &DataCiteWork) -> Result<Paper, SourceError> {
    let attrs = &item.attributes;
    let title = attrs
        .titles
        .iter()
        .find_map(|t| t.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Untitled".to_string());
    let authors = attrs
        .creators
        .iter()
        .filter_map(|c| c.name.clone().or_else(|| c.given_name.clone()))
        .collect::<Vec<_>>()
        .join("; ");
    let abstract_text = attrs
        .descriptions
        .iter()
        .find_map(|d| d.description.clone())
        .unwrap_or_default();
    let license = attrs.rights_list.iter().find_map(|r| {
        r.rights_identifier
            .clone()
            .or_else(|| r.rights.clone())
            .filter(|s| !s.is_empty())
    });
    let doi = attrs.doi.clone().unwrap_or_default();
    let url = if doi.is_empty() {
        attrs.url.clone().unwrap_or_default()
    } else {
        format!("https://doi.org/{doi}")
    };

    let mut paper = PaperBuilder::new(item.id.clone(), title, url, SourceType::DataCite)
        .authors(authors)
        .abstract_text(abstract_text)
        .doi(&doi)
        .published_date(
            attrs
                .publication_year
                .map(|year| year.to_string())
                .unwrap_or_default(),
        );
    if let Some(license) = license {
        paper = paper.license(license);
    }
    Ok(paper.build())
}

#[derive(Debug, Deserialize)]
struct DataCiteList {
    data: Vec<DataCiteWork>,
    meta: Option<DataCiteMeta>,
}

#[derive(Debug, Deserialize)]
struct DataCiteItem {
    data: DataCiteWork,
}

#[derive(Debug, Deserialize)]
struct DataCiteMeta {
    total: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct DataCiteWork {
    id: String,
    attributes: DataCiteAttributes,
}

#[derive(Debug, Deserialize)]
struct DataCiteAttributes {
    doi: Option<String>,
    titles: Vec<DataCiteTitle>,
    #[serde(default)]
    creators: Vec<DataCiteCreator>,
    #[serde(default, rename = "descriptions")]
    descriptions: Vec<DataCiteDescription>,
    #[serde(rename = "publicationYear")]
    publication_year: Option<u32>,
    #[serde(default, rename = "rightsList")]
    rights_list: Vec<DataCiteRights>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DataCiteTitle {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DataCiteCreator {
    name: Option<String>,
    #[serde(rename = "givenName")]
    given_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DataCiteDescription {
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DataCiteRights {
    rights: Option<String>,
    #[serde(rename = "rightsIdentifier")]
    rights_identifier: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_datacite_work() {
        let json = r#"{
            "id": "10.5281/zenodo.1",
            "attributes": {
                "doi": "10.5281/zenodo.1",
                "titles": [{"title": "A Dataset"}],
                "creators": [{"name": "Ada Lovelace"}],
                "descriptions": [{"description": "An abstract."}],
                "publicationYear": 2024,
                "rightsList": [{"rightsIdentifier": "cc-by-4.0", "rights": "CC BY 4.0"}],
                "url": "https://zenodo.org/records/1"
            }
        }"#;
        let work: DataCiteWork = serde_json::from_str(json).unwrap();
        let paper = parse_work(&work).unwrap();
        assert_eq!(paper.title, "A Dataset");
        assert_eq!(paper.authors, "Ada Lovelace");
        assert_eq!(paper.license.as_deref(), Some("cc-by-4.0"));
        assert_eq!(paper.doi.as_deref(), Some("10.5281/zenodo.1"));
        assert_eq!(paper.source, SourceType::DataCite);
        assert_eq!(paper.published_date.as_deref(), Some("2024"));
    }
}
