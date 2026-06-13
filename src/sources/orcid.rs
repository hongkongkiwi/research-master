//! ORCID research source implementation.
//!
//! ORCID provides persistent digital identifiers for researchers.
//! Free API at https://pub.orcid.org/
//!
//! Capabilities: SEARCH, AUTHOR_SEARCH

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::HttpClient;

const ORCID_API_BASE: &str = "https://pub.orcid.org/v3.0";

/// ORCID research source
#[derive(Debug, Clone)]
pub struct OrcidSource {
    client: Arc<HttpClient>,
}

impl OrcidSource {
    pub fn new() -> Result<Self, SourceError> {
        let user_agent = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
        Ok(Self {
            client: Arc::new(HttpClient::with_rate_limit(user_agent, 2)?),
        })
    }

    fn build_url(&self, endpoint: &str) -> String {
        format!("{}{}", ORCID_API_BASE, endpoint)
    }
}

#[derive(Debug, Deserialize)]
struct OrcidSearchResponse {
    result: Option<Vec<OrcidResult>>,
    #[serde(rename = "num-found")]
    num_found: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OrcidResult {
    #[serde(rename = "orcid-identifier")]
    orcid_identifier: Option<OrcidIdentifier>,
    #[serde(rename = "person")]
    person: Option<OrcidPerson>,
}

#[derive(Debug, Deserialize)]
struct OrcidIdentifier {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OrcidPerson {
    name: Option<OrcidName>,
}

#[derive(Debug, Deserialize)]
struct OrcidName {
    #[serde(rename = "given-names")]
    given_names: Option<OrcidValue>,
    #[serde(rename = "family-name")]
    family_name: Option<OrcidValue>,
    #[serde(rename = "credit-name")]
    credit_name: Option<OrcidValue>,
}

#[derive(Debug, Deserialize)]
struct OrcidValue {
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OrcidWorksResponse {
    group: Option<Vec<OrcidWorkGroup>>,
}

#[derive(Debug, Deserialize)]
struct OrcidWorkGroup {
    #[serde(rename = "work-summary")]
    work_summary: Option<Vec<OrcidWorkSummary>>,
}

#[derive(Debug, Deserialize)]
struct OrcidWorkSummary {
    title: Option<OrcidWorkTitle>,
    #[serde(rename = "external-ids")]
    external_ids: Option<OrcidExternalIds>,
    #[serde(rename = "publication-date")]
    publication_date: Option<OrcidDate>,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OrcidWorkTitle {
    title: Option<OrcidValue>,
}

#[derive(Debug, Deserialize)]
struct OrcidExternalIds {
    #[serde(rename = "external-id")]
    external_id: Option<Vec<OrcidExternalId>>,
}

#[derive(Debug, Deserialize)]
struct OrcidExternalId {
    #[serde(rename = "external-id-type")]
    external_id_type: Option<String>,
    #[serde(rename = "external-id-value")]
    external_id_value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OrcidDate {
    year: Option<OrcidValue>,
    month: Option<OrcidValue>,
    day: Option<OrcidValue>,
}

impl OrcidSource {
    /// Build a search query from author name parts
    fn build_author_query(author: &str) -> String {
        let parts: Vec<&str> = author.split_whitespace().collect();
        if parts.len() >= 2 {
            // Try to split into given and family name
            let given = parts[..parts.len() - 1].join(" ");
            let family = parts[parts.len() - 1];
            format!(
                "given-names:{} AND family-name:{}",
                urlencoding::encode(&given),
                urlencoding::encode(family)
            )
        } else {
            format!("text:{}", urlencoding::encode(author))
        }
    }

    /// Parse an ORCID record into a Paper representing the researcher profile
    fn parse_profile(result: &OrcidResult) -> Paper {
        let orcid_path = result
            .orcid_identifier
            .as_ref()
            .and_then(|id| id.path.clone())
            .unwrap_or_default();

        let name = result
            .person
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .and_then(|n| {
                n.credit_name
                    .as_ref()
                    .and_then(|c| c.value.clone())
                    .or_else(|| {
                        let given = n
                            .given_names
                            .as_ref()
                            .and_then(|g| g.value.clone())
                            .unwrap_or_default();
                        let family = n
                            .family_name
                            .as_ref()
                            .and_then(|f| f.value.clone())
                            .unwrap_or_default();
                        if given.is_empty() && family.is_empty() {
                            None
                        } else {
                            Some(format!("{}, {}", family, given))
                        }
                    })
            })
            .unwrap_or_else(|| "Unknown ORCID Researcher".to_string());

        let profile_url = format!("https://orcid.org/{}", orcid_path);

        PaperBuilder::new(
            orcid_path.clone(),
            name,
            profile_url,
            SourceType::Other("ORCID".to_string()),
        )
        .build()
    }

    /// Parse an ORCID work summary into a Paper
    fn parse_work(summary: &OrcidWorkSummary) -> Paper {
        let title = summary
            .title
            .as_ref()
            .and_then(|t| t.title.as_ref().and_then(|v| v.value.clone()))
            .unwrap_or_else(|| "Untitled Work".to_string());

        let doi = summary
            .external_ids
            .as_ref()
            .and_then(|ids| ids.external_id.as_ref())
            .and_then(|ids| {
                ids.iter()
                    .find(|id| id.external_id_type.as_deref() == Some("doi"))
                    .and_then(|id| id.external_id_value.clone())
            });

        let paper_id = doi
            .clone()
            .unwrap_or_else(|| summary.path.clone().unwrap_or_default());

        let url = doi
            .as_ref()
            .map(|d| format!("https://doi.org/{}", d))
            .unwrap_or_else(|| {
                format!(
                    "https://orcid.org/{}",
                    summary.path.as_deref().unwrap_or("")
                )
            });

        let published_date = summary.publication_date.as_ref().and_then(|d| {
            let year = d
                .year
                .as_ref()
                .and_then(|y| y.value.as_ref())
                .map(|s| s.to_string())
                .unwrap_or_default();
            if year.is_empty() {
                return None;
            }
            let month = d
                .month
                .as_ref()
                .and_then(|m| m.value.clone())
                .unwrap_or_else(|| "01".to_string());
            let day = d
                .day
                .as_ref()
                .and_then(|d| d.value.clone())
                .unwrap_or_else(|| "01".to_string());
            Some(format!("{}-{:0>2}-{:0>2}", year, month, day))
        });

        let mut paper =
            PaperBuilder::new(paper_id, title, url, SourceType::Other("ORCID".to_string()));
        if let Some(date) = published_date {
            paper = paper.published_date(date);
        }
        if let Some(d) = doi {
            paper = paper.doi(d);
        }
        paper.build()
    }
}

#[async_trait]
impl Source for OrcidSource {
    fn id(&self) -> &str {
        "orcid"
    }

    fn name(&self) -> &str {
        "ORCID"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::AUTHOR_SEARCH
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        // Search for ORCID records matching the query text
        let search_query = if query.query.chars().any(|c| c == ' ') {
            OrcidSource::build_author_query(&query.query)
        } else {
            format!("text:{}", urlencoding::encode(&query.query))
        };

        let url = format!(
            "/search?q={}&rows={}",
            search_query,
            query.max_results.min(100)
        );

        let response = self
            .client
            .get(&self.build_url(&url))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("ORCID search failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(SourceError::Api(format!(
                "ORCID API returned {}",
                response.status()
            )));
        }

        let data: OrcidSearchResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("ORCID JSON parse error: {}", e)))?;

        let papers: Vec<Paper> = data
            .result
            .unwrap_or_default()
            .iter()
            .map(Self::parse_profile)
            .collect();

        let total = data.num_found.unwrap_or(papers.len() as u64) as usize;

        Ok(SearchResponse::new(papers, "ORCID", &query.query).total_results(total))
    }

    async fn search_by_author(
        &self,
        author: &str,
        max_results: usize,
        _year: Option<&str>,
    ) -> Result<SearchResponse, SourceError> {
        // Search for ORCID profile
        let search_query = OrcidSource::build_author_query(author);
        let url = format!("/search?q={}&rows=1", search_query);

        let response = self
            .client
            .get(&self.build_url(&url))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("ORCID author search failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(SourceError::Api(format!(
                "ORCID API returned {}",
                response.status()
            )));
        }

        let data: OrcidSearchResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("ORCID JSON parse error: {}", e)))?;

        // Get the ORCID
        let orcid = data
            .result
            .and_then(|r| r.into_iter().next())
            .and_then(|r| r.orcid_identifier.and_then(|id| id.path));

        let orcid = match orcid {
            Some(id) => id,
            None => {
                return Err(SourceError::NotFound(format!(
                    "ORCID not found for '{}'",
                    author
                )))
            }
        };

        // Fetch works for this ORCID
        let works_url = format!("/{}/works", urlencoding::encode(&orcid));
        let works_response = self
            .client
            .get(&self.build_url(&works_url))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("ORCID works fetch failed: {}", e)))?;

        if !works_response.status().is_success() {
            return Err(SourceError::NotFound(format!(
                "Works not found for ORCID '{}'",
                orcid
            )));
        }

        let works: OrcidWorksResponse = works_response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("ORCID works JSON parse error: {}", e)))?;

        let papers: Vec<Paper> = works
            .group
            .unwrap_or_default()
            .iter()
            .filter_map(|g| g.work_summary.as_ref()?.first())
            .take(max_results)
            .map(Self::parse_work)
            .collect();

        Ok(SearchResponse::new(papers, "ORCID", author))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_author_query() {
        let query = OrcidSource::build_author_query("John Smith");
        assert!(query.contains("given-names:"));
        assert!(query.contains("family-name:"));
        assert!(query.contains("John"));
        assert!(query.contains("Smith"));
    }

    #[test]
    fn test_build_author_query_single_name() {
        let query = OrcidSource::build_author_query("Einstein");
        assert!(query.contains("text:"));
        assert!(query.contains("Einstein"));
    }

    #[test]
    fn test_orcid_source_capabilities() {
        let caps = SourceCapabilities::SEARCH | SourceCapabilities::AUTHOR_SEARCH;
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::AUTHOR_SEARCH));
        assert!(!caps.contains(SourceCapabilities::DOWNLOAD));
    }
}
