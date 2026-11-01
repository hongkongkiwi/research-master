//! INSPIRE-HEP research source.
//!
//! Searches the INSPIRE literature database for high-energy physics. The API is
//! open and needs no key.
//! API documentation: <https://github.com/inspirehep/rest-api-doc>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const INSPIRE_API: &str = "https://inspirehep.net/api/literature";

/// INSPIRE-HEP literature source.
#[derive(Debug, Clone)]
pub struct InspireSource {
    client: Arc<HttpClient>,
}

impl InspireSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }
}

impl Default for InspireSource {
    fn default() -> Self {
        Self::new().expect("Failed to create InspireSource")
    }
}

#[async_trait]
impl Source for InspireSource {
    fn id(&self) -> &str {
        "inspire"
    }

    fn name(&self) -> &str {
        "INSPIRE-HEP"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let size = query.max_results.clamp(1, 100);
        let mut q = query.query.clone();
        if let Some(year) = query.year.as_deref().and_then(crate::models::year_bounds) {
            let (from, to) = year;
            if from == to {
                q.push_str(&format!(" year:{from}"));
            } else {
                q.push_str(&format!(" year:[{from} TO {to}]"));
            }
        }
        let url = format!("{INSPIRE_API}?q={}&size={size}", urlencoding::encode(&q));

        let body = get_json(&self.client, &url).await?;
        let response: InspireSearch = serde_json::from_str(&body)?;
        let total = response.hits.total;
        let papers = response
            .hits
            .hits
            .iter()
            .map(parse_hit)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SearchResponse::new(papers, "INSPIRE-HEP", &query.query).total_results(total))
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let doi = doi
            .trim()
            .trim_start_matches("https://doi.org/")
            .trim_start_matches("doi:");
        let url = format!("{INSPIRE_API}?q=doi:{}&size=1", urlencoding::encode(doi));
        let body = get_json(&self.client, &url).await?;
        let response: InspireSearch = serde_json::from_str(&body)?;
        response
            .hits
            .hits
            .first()
            .map(parse_hit)
            .transpose()?
            .ok_or_else(|| SourceError::NotFound(format!("DOI {doi} not found in INSPIRE-HEP")))
    }
}

async fn get_json(client: &HttpClient, url: &str) -> Result<String, SourceError> {
    let client = Arc::new(client.clone());
    let url = url.to_string();
    with_retry(api_retry_config(), || {
        let client = Arc::clone(&client);
        let url = url.clone();
        async move {
            let response = client
                .get(&url)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| SourceError::Network(format!("Failed to query INSPIRE-HEP: {e}")))?;
            if !response.status().is_success() {
                return Err(SourceError::Api(format!(
                    "INSPIRE-HEP returned {}",
                    response.status()
                )));
            }
            response.text().await.map_err(|e| {
                SourceError::Network(format!("Failed to read INSPIRE-HEP response: {e}"))
            })
        }
    })
    .await
}

fn parse_hit(hit: &InspireHit) -> Result<Paper, SourceError> {
    let meta = &hit.metadata;
    let title = meta
        .titles
        .iter()
        .find_map(|t| t.title.clone())
        .unwrap_or_else(|| "Untitled".to_string());
    let authors = meta
        .authors
        .iter()
        .filter_map(|a| a.full_name.clone())
        .collect::<Vec<_>>()
        .join("; ");
    let abstract_text = meta
        .abstracts
        .iter()
        .find_map(|a| a.value.clone())
        .unwrap_or_default();
    let doi = meta.dois.iter().find_map(|d| d.value.clone());
    let arxiv = meta.arxiv_eprints.iter().find_map(|e| e.value.clone());
    let license = meta.license.iter().find_map(|l| l.license.clone());

    let id = hit.id.clone();
    let url = doi
        .as_ref()
        .map(|d| format!("https://doi.org/{d}"))
        .unwrap_or_else(|| format!("https://inspirehep.net/literature/{id}"));

    let mut paper = PaperBuilder::new(id, title, url, SourceType::Inspire)
        .authors(authors)
        .abstract_text(abstract_text)
        .published_date(meta.earliest_date.clone().unwrap_or_default())
        .citations(meta.citation_count.unwrap_or(0));
    if let Some(doi) = doi {
        paper = paper.doi(doi);
    }
    if let Some(arxiv) = arxiv {
        paper = paper.pdf_url(format!("https://arxiv.org/pdf/{arxiv}"));
    }
    if let Some(license) = license {
        paper = paper.license(license);
    }
    Ok(paper.build())
}

#[derive(Debug, Deserialize)]
struct InspireSearch {
    hits: InspireHits,
}

#[derive(Debug, Deserialize)]
struct InspireHits {
    total: usize,
    hits: Vec<InspireHit>,
}

#[derive(Debug, Deserialize)]
struct InspireHit {
    id: String,
    metadata: InspireMetadata,
}

#[derive(Debug, Deserialize)]
struct InspireMetadata {
    #[serde(default)]
    titles: Vec<InspireTitle>,
    #[serde(default)]
    authors: Vec<InspireAuthor>,
    #[serde(default)]
    abstracts: Vec<InspireAbstract>,
    #[serde(default)]
    dois: Vec<InspireDoi>,
    #[serde(default)]
    arxiv_eprints: Vec<InspireArxiv>,
    #[serde(default)]
    license: Vec<InspireLicense>,
    earliest_date: Option<String>,
    citation_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct InspireTitle {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InspireAuthor {
    full_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InspireAbstract {
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InspireDoi {
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InspireArxiv {
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InspireLicense {
    license: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_inspire_record() {
        let json = r#"{
            "id": "1234",
            "metadata": {
                "titles": [{"title": "Holography Transformer"}],
                "authors": [{"full_name": "Ada Lovelace"}],
                "abstracts": [{"value": "An abstract."}],
                "dois": [{"value": "10.1000/hep.1"}],
                "arxiv_eprints": [{"value": "2311.01724"}],
                "license": [{"license": "arXiv nonexclusive-distrib 1.0"}],
                "earliest_date": "2023-11-03",
                "citation_count": 7
            }
        }"#;
        let hit: InspireHit = serde_json::from_str(json).unwrap();
        let paper = parse_hit(&hit).unwrap();
        assert_eq!(paper.title, "Holography Transformer");
        assert_eq!(paper.citations, Some(7));
        assert_eq!(
            paper.pdf_url.as_deref(),
            Some("https://arxiv.org/pdf/2311.01724")
        );
        assert_eq!(
            paper.license.as_deref(),
            Some("arXiv nonexclusive-distrib 1.0")
        );
        assert_eq!(paper.source, SourceType::Inspire);
    }
}
