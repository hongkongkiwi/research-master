//! ClinicalTrials.gov research source.
//!
//! Uses the ClinicalTrials.gov API v2 to search and fetch clinical study
//! registry records. No API key required.
//! API documentation: <https://clinicaltrials.gov/data-api/api>

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{year_bounds, Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const CTG_API_BASE: &str = "https://clinicaltrials.gov/api/v2/studies";

/// API v2 accepts pageSize up to 1000.
const MAX_PAGE_SIZE: usize = 1000;

/// Study fields requested from the API; everything the Paper mapping reads.
const FIELDS: &str = "protocolSection.identificationModule.nctId,\
protocolSection.identificationModule.briefTitle,\
protocolSection.identificationModule.officialTitle,\
protocolSection.statusModule.overallStatus,\
protocolSection.statusModule.startDateStruct,\
protocolSection.descriptionModule.briefSummary,\
protocolSection.conditionsModule.conditions,\
protocolSection.armsInterventionsModule.interventions,\
protocolSection.sponsorCollaboratorsModule.leadSponsor";

/// ClinicalTrials.gov study registry source.
///
/// Free, keyless API. Study records carry no DOI of their own; the only DOI
/// search path is the free-text `ReferenceCitation` area, so `get_by_doi`
/// returns the first study whose references cite the given DOI.
#[derive(Debug, Clone)]
pub struct ClinicalTrialsSource {
    client: Arc<HttpClient>,
}

impl ClinicalTrialsSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::for_source(
                "clinical_trials",
                &crate::utils::get_user_agent(),
            )?),
        })
    }

    /// Create with a custom HTTP client (for testing)
    #[allow(dead_code)]
    pub fn with_client(client: Arc<HttpClient>) -> Self {
        Self { client }
    }

    async fn search_at(
        &self,
        base_url: &str,
        query: &SearchQuery,
    ) -> Result<SearchResponse, SourceError> {
        let url = build_search_url(base_url, query);
        let body = fetch_json(&self.client, &url, "search ClinicalTrials.gov").await?;
        let response: StudiesResponse = serde_json::from_str(&body)?;
        let papers = response
            .studies
            .iter()
            .map(parse_study)
            .collect::<Result<Vec<_>, _>>()?;
        let matched = papers.len();
        Ok(
            SearchResponse::new(papers, "ClinicalTrials.gov", &query.query)
                .total_results(response.total_count.unwrap_or(matched)),
        )
    }

    async fn get_by_id_at(&self, base_url: &str, id: &str) -> Result<Paper, SourceError> {
        let nct_id = normalize_nct_id(id)?;
        let url = format!("{base_url}/{nct_id}?fields={}", urlencoding::encode(FIELDS));
        let body = fetch_json(&self.client, &url, &format!("fetch study {nct_id}")).await?;
        let record: StudyRecord = serde_json::from_str(&body)?;
        parse_study(&record)
    }

    async fn get_by_doi_at(&self, base_url: &str, doi: &str) -> Result<Paper, SourceError> {
        let doi = doi
            .trim()
            .trim_start_matches("https://doi.org/")
            .trim_start_matches("http://doi.org/")
            .trim_start_matches("doi:");
        if doi.is_empty() {
            return Err(SourceError::InvalidRequest(
                "ClinicalTrials.gov DOI lookup needs a DOI".to_string(),
            ));
        }
        let url = format!(
            "{}?filter.advanced={}&pageSize=1&fields={}",
            base_url,
            urlencoding::encode(&format!("AREA[ReferenceCitation]{doi}")),
            urlencoding::encode(FIELDS)
        );
        let body = fetch_json(&self.client, &url, &format!("look up DOI {doi}")).await?;
        let response: StudiesResponse = serde_json::from_str(&body)?;
        match response.studies.first() {
            Some(record) => parse_study(record),
            None => Err(SourceError::NotFound(format!(
                "No ClinicalTrials.gov study references DOI {doi}"
            ))),
        }
    }
}

impl Default for ClinicalTrialsSource {
    fn default() -> Self {
        Self::new().expect("Failed to create ClinicalTrialsSource")
    }
}

#[async_trait]
impl Source for ClinicalTrialsSource {
    fn id(&self) -> &str {
        "clinical_trials"
    }

    fn name(&self) -> &str {
        "ClinicalTrials.gov"
    }

    fn capabilities(&self) -> SourceCapabilities {
        // SEARCH only: `get_by_doi` returns the first study CITING a DOI
        // (records carry no DOI of their own), so it must not answer the
        // cross-source `lookup_by_doi` fan-out, where it would be presented
        // as the DOI's own paper. Reachable via explicit source selection.
        SourceCapabilities::SEARCH
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        self.search_at(CTG_API_BASE, query).await
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        self.get_by_doi_at(CTG_API_BASE, doi).await
    }

    async fn get_by_id(&self, id: &str) -> Result<Paper, SourceError> {
        self.get_by_id_at(CTG_API_BASE, id).await
    }

    fn validate_id(&self, id: &str) -> Result<(), SourceError> {
        normalize_nct_id(id).map(|_| ())
    }
}

/// Validate an NCT ID (`NCT` + 8 digits, case-insensitive) and normalize it to uppercase.
fn normalize_nct_id(id: &str) -> Result<String, SourceError> {
    let normalized = id.trim().to_ascii_uppercase();
    let bytes = normalized.as_bytes();
    let valid =
        bytes.len() == 11 && &bytes[..3] == b"NCT" && bytes[3..].iter().all(|b| b.is_ascii_digit());
    if valid {
        Ok(normalized)
    } else {
        Err(SourceError::InvalidRequest(format!(
            "Invalid ClinicalTrials.gov NCT ID '{id}': expected NCT followed by 8 digits (e.g. NCT03850691)"
        )))
    }
}

/// Map a query onto API v2 search parameters. Param order is fixed so tests
/// can match the exact URL.
fn build_search_url(base_url: &str, query: &SearchQuery) -> String {
    let page_size = query.max_results.clamp(1, MAX_PAGE_SIZE);
    let mut params = vec![
        ("query.term", query.query.clone()),
        ("pageSize", page_size.to_string()),
        ("countTotal", "true".to_string()),
    ];
    if let Some((from, to)) = query.year.as_deref().and_then(year_bounds) {
        params.push(("filter.advanced", start_date_range(from, to)));
    }
    params.push(("fields", FIELDS.to_string()));
    let query_string = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base_url}?{query_string}")
}

/// Build an `AREA[StartDate]RANGE` filter. The 1000/9999 sentinels that
/// `year_bounds` uses for open-ended ranges map to the API's MIN/MAX.
fn start_date_range(from: u32, to: u32) -> String {
    let lower = if from <= 1000 {
        "MIN".to_string()
    } else {
        format!("{from}-01-01")
    };
    let upper = if to >= 9999 {
        "MAX".to_string()
    } else {
        format!("{to}-12-31")
    };
    format!("AREA[StartDate]RANGE[{lower},{upper}]")
}

async fn fetch_json(client: &HttpClient, url: &str, what: &str) -> Result<String, SourceError> {
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
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| SourceError::Network(format!("Failed to {what}: {e}")))?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(SourceError::NotFound(format!(
                    "Not found on ClinicalTrials.gov: {url}"
                )));
            }
            if !response.status().is_success() {
                return Err(SourceError::Api(format!(
                    "ClinicalTrials.gov returned {}",
                    response.status()
                )));
            }
            response.text().await.map_err(|e| {
                SourceError::Network(format!("Failed to read ClinicalTrials.gov response: {e}"))
            })
        }
    })
    .await
}

fn parse_study(record: &StudyRecord) -> Result<Paper, SourceError> {
    let protocol = record.protocol_section.as_ref().ok_or_else(|| {
        SourceError::Parse("ClinicalTrials.gov study response missing protocolSection".to_string())
    })?;
    let identification = protocol.identification.as_ref().ok_or_else(|| {
        SourceError::Parse(
            "ClinicalTrials.gov study response missing identificationModule".to_string(),
        )
    })?;
    let nct_id = identification.nct_id.trim();
    if nct_id.is_empty() {
        return Err(SourceError::Parse(
            "ClinicalTrials.gov study response missing nctId".to_string(),
        ));
    }

    let title = identification
        .brief_title
        .as_deref()
        .filter(|t| !t.is_empty())
        .or(identification.official_title.as_deref())
        .unwrap_or("Untitled")
        .to_string();
    let abstract_text = protocol
        .description
        .as_ref()
        .and_then(|d| d.brief_summary.clone())
        .unwrap_or_default();
    let published_date = protocol
        .status
        .as_ref()
        .and_then(|s| s.start_date.as_ref())
        .and_then(|d| d.date.clone())
        .unwrap_or_default();
    let overall_status = protocol
        .status
        .as_ref()
        .and_then(|s| s.overall_status.clone())
        .unwrap_or_default();
    let authors = protocol
        .sponsors
        .as_ref()
        .and_then(|s| s.lead_sponsor.as_ref())
        .and_then(|s| s.name.clone())
        .unwrap_or_default();
    let categories = protocol
        .conditions
        .as_ref()
        .map(|m| m.conditions.join("; "))
        .unwrap_or_default();
    let keywords = protocol
        .interventions
        .as_ref()
        .map(|m| {
            m.interventions
                .iter()
                .filter_map(|i| i.name.as_deref())
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();

    let mut paper = PaperBuilder::new(
        nct_id.to_string(),
        title,
        format!("https://clinicaltrials.gov/study/{nct_id}"),
        SourceType::ClinicalTrials,
    )
    .authors(authors)
    .abstract_text(abstract_text)
    .published_date(published_date)
    .categories(categories)
    .keywords(keywords)
    .paper_type("clinical trial");
    if !overall_status.is_empty() {
        paper = paper.extra("status", serde_json::Value::String(overall_status));
    }
    Ok(paper.build())
}

#[derive(Debug, Deserialize)]
struct StudiesResponse {
    #[serde(default)]
    studies: Vec<StudyRecord>,
    #[serde(rename = "totalCount", default)]
    total_count: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct StudyRecord {
    #[serde(rename = "protocolSection", default)]
    protocol_section: Option<ProtocolSection>,
}

#[derive(Debug, Deserialize)]
struct ProtocolSection {
    #[serde(rename = "identificationModule", default)]
    identification: Option<IdentificationModule>,
    #[serde(rename = "statusModule", default)]
    status: Option<StatusModule>,
    #[serde(rename = "descriptionModule", default)]
    description: Option<DescriptionModule>,
    #[serde(rename = "conditionsModule", default)]
    conditions: Option<ConditionsModule>,
    #[serde(rename = "armsInterventionsModule", default)]
    interventions: Option<InterventionsModule>,
    #[serde(rename = "sponsorCollaboratorsModule", default)]
    sponsors: Option<SponsorModule>,
}

#[derive(Debug, Deserialize)]
struct IdentificationModule {
    #[serde(rename = "nctId")]
    nct_id: String,
    #[serde(rename = "briefTitle", default)]
    brief_title: Option<String>,
    #[serde(rename = "officialTitle", default)]
    official_title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StatusModule {
    #[serde(rename = "overallStatus", default)]
    overall_status: Option<String>,
    #[serde(rename = "startDateStruct", default)]
    start_date: Option<DateStruct>,
}

#[derive(Debug, Deserialize)]
struct DateStruct {
    #[serde(default)]
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DescriptionModule {
    #[serde(rename = "briefSummary", default)]
    brief_summary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConditionsModule {
    #[serde(default)]
    conditions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct InterventionsModule {
    #[serde(default)]
    interventions: Vec<Intervention>,
}

#[derive(Debug, Deserialize)]
struct Intervention {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SponsorModule {
    #[serde(rename = "leadSponsor", default)]
    lead_sponsor: Option<LeadSponsor>,
}

#[derive(Debug, Deserialize)]
struct LeadSponsor {
    #[serde(default)]
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    /// Build a client with proxy env vars cleared so requests reach the
    /// local mockito server instead of any ambient proxy.
    fn test_client() -> HttpClient {
        let guard = env_lock().lock().expect("env lock poisoned");
        const VARS: [&str; 6] = [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ];
        let previous: Vec<(&str, Option<String>)> =
            VARS.iter().map(|&k| (k, std::env::var(k).ok())).collect();
        for var in VARS {
            std::env::remove_var(var);
        }
        let client =
            HttpClient::without_rate_limit("research-master-tests").expect("test HTTP client");
        for (var, value) in previous {
            match value {
                Some(v) => std::env::set_var(var, v),
                None => std::env::remove_var(var),
            }
        }
        drop(guard);
        client
    }

    /// Trimmed from a live fetch of
    /// /studies/NCT03850691?fields=<FIELDS> (2026-11-02).
    const STUDY_FIXTURE: &str = r#"{
        "protocolSection": {
            "identificationModule": {
                "nctId": "NCT03850691",
                "briefTitle": "Radiation and Combination Immunotherapy for Melanoma",
                "officialTitle": "A Phase 2 Study of Palliative Radiation and Combination Sequential Immunotherapy for Metastatic Cutaneous Melanoma and Ocular Melanoma"
            },
            "statusModule": {
                "overallStatus": "COMPLETED",
                "startDateStruct": {"date": "2019-05-28", "type": "ACTUAL"}
            },
            "sponsorCollaboratorsModule": {
                "leadSponsor": {"name": "Masonic Cancer Center, University of Minnesota", "class": "OTHER"}
            },
            "descriptionModule": {
                "briefSummary": "Phase 2 study evaluating checkpoint blockade and aldesleukin (IL-2) therapy after palliative radiation in unresectable metastatic melanoma."
            },
            "conditionsModule": {"conditions": ["Metastatic Melanoma"]},
            "armsInterventionsModule": {
                "interventions": [
                    {"type": "DRUG", "name": "Aldesleukin: All Patients"},
                    {"type": "DRUG", "name": "Nivolumab: Cohort 1 (Cutaneous)"}
                ]
            }
        }
    }"#;

    fn search_path(query: &SearchQuery) -> String {
        let url = build_search_url("", query);
        url.trim_start_matches('?').to_string()
    }

    fn study_record(json: &str) -> StudyRecord {
        serde_json::from_str(json).expect("study fixture")
    }

    #[test]
    fn test_validate_nct_id() {
        let source = ClinicalTrialsSource::with_client(Arc::new(test_client()));
        assert!(source.validate_id("NCT03850691").is_ok());
        assert!(source.validate_id("  nct03850691 ").is_ok());
        assert_eq!(normalize_nct_id("nct03850691").unwrap(), "NCT03850691");
        assert!(source.validate_id("NCT12345").is_err());
        assert!(source.validate_id("12345678NCT").is_err());
        assert!(source.validate_id("NCT1234567A").is_err());
        assert!(source.validate_id("10.1038/nature12373").is_err());
    }

    #[test]
    fn test_build_search_url_params() {
        let query = SearchQuery::new("melanoma immunotherapy").max_results(2);
        let url = build_search_url("https://example.test/api/v2/studies", &query);
        assert!(url.starts_with("https://example.test/api/v2/studies?"));
        assert!(url.contains("query.term=melanoma%20immunotherapy"));
        assert!(url.contains("pageSize=2"));
        assert!(url.contains("countTotal=true"));
        assert!(url.contains("fields="));

        let ranged = SearchQuery::new("melanoma")
            .max_results(5)
            .year("2019-2021");
        let url = build_search_url("", &ranged);
        assert!(url.contains(&format!(
            "filter.advanced={}",
            urlencoding::encode("AREA[StartDate]RANGE[2019-01-01,2021-12-31]")
        )));

        let open_ended = SearchQuery::new("melanoma").year("2019-");
        let url = build_search_url("", &open_ended);
        assert!(url.contains(&format!(
            "filter.advanced={}",
            urlencoding::encode("AREA[StartDate]RANGE[2019-01-01,MAX]")
        )));
    }

    #[test]
    fn test_parse_study_maps_fields() {
        let record = study_record(STUDY_FIXTURE);
        let paper = parse_study(&record).unwrap();
        assert_eq!(paper.paper_id, "NCT03850691");
        assert_eq!(
            paper.title,
            "Radiation and Combination Immunotherapy for Melanoma"
        );
        assert_eq!(paper.url, "https://clinicaltrials.gov/study/NCT03850691");
        assert_eq!(paper.source, SourceType::ClinicalTrials);
        assert_eq!(paper.paper_type.as_deref(), Some("clinical trial"));
        assert_eq!(
            paper.authors,
            "Masonic Cancer Center, University of Minnesota"
        );
        assert_eq!(paper.published_date.as_deref(), Some("2019-05-28"));
        assert_eq!(paper.category_list(), vec!["Metastatic Melanoma"]);
        assert_eq!(
            paper.keyword_list(),
            vec![
                "Aldesleukin: All Patients",
                "Nivolumab: Cohort 1 (Cutaneous)"
            ]
        );
        assert_eq!(
            paper
                .extra
                .as_ref()
                .and_then(|m| m.get("status"))
                .and_then(|v| v.as_str()),
            Some("COMPLETED")
        );
        assert!(paper.r#abstract.contains("aldesleukin"));
        // Study records carry no DOI of their own.
        assert!(paper.doi.is_none());
    }

    #[tokio::test]
    async fn test_search_at_parses_response() {
        let mut server = mockito::Server::new_async().await;
        let query = SearchQuery::new("melanoma immunotherapy").max_results(2);
        let mock = server
            .mock("GET", format!("/studies?{}", search_path(&query)).as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"studies":[{STUDY_FIXTURE}],"totalCount":4582}}"#
            ))
            .expect(1)
            .create_async()
            .await;

        let source = ClinicalTrialsSource::with_client(Arc::new(test_client()));
        let response = source
            .search_at(&format!("{}/studies", server.url()), &query)
            .await
            .expect("search");
        mock.assert_async().await;

        assert_eq!(response.total_results, Some(4582));
        assert_eq!(response.papers.len(), 1);
        assert_eq!(response.papers[0].paper_id, "NCT03850691");
        assert_eq!(response.source, "ClinicalTrials.gov");
    }

    #[tokio::test]
    async fn test_get_by_id_at_fetches_study() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock(
                "GET",
                format!(
                    "/studies/NCT03850691?fields={}",
                    urlencoding::encode(FIELDS)
                )
                .as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(STUDY_FIXTURE)
            .expect(1)
            .create_async()
            .await;

        let source = ClinicalTrialsSource::with_client(Arc::new(test_client()));
        let paper = source
            .get_by_id_at(&format!("{}/studies", server.url()), "nct03850691")
            .await
            .expect("study");
        mock.assert_async().await;
        assert_eq!(paper.paper_id, "NCT03850691");
    }

    #[tokio::test]
    async fn test_get_by_id_at_not_found() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock(
                "GET",
                format!(
                    "/studies/NCT99999999?fields={}",
                    urlencoding::encode(FIELDS)
                )
                .as_str(),
            )
            .with_status(404)
            .with_body("NCT number NCT99999999 not found")
            .expect(1)
            .create_async()
            .await;

        let source = ClinicalTrialsSource::with_client(Arc::new(test_client()));
        let result = source
            .get_by_id_at(&format!("{}/studies", server.url()), "NCT99999999")
            .await;
        mock.assert_async().await;
        assert!(matches!(result, Err(SourceError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_get_by_doi_at_returns_citing_study() {
        let mut server = mockito::Server::new_async().await;
        let doi = "10.1200/JCO.18.02306";
        let mock = server
            .mock(
                "GET",
                format!(
                    "/studies?filter.advanced={}&pageSize=1&fields={}",
                    urlencoding::encode(&format!("AREA[ReferenceCitation]{doi}")),
                    urlencoding::encode(FIELDS)
                )
                .as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(r#"{{"studies":[{STUDY_FIXTURE}]}}"#))
            .expect(1)
            .create_async()
            .await;

        let source = ClinicalTrialsSource::with_client(Arc::new(test_client()));
        let paper = source
            .get_by_doi_at(
                &format!("{}/studies", server.url()),
                &format!("https://doi.org/{doi}"),
            )
            .await
            .expect("study citing DOI");
        mock.assert_async().await;
        assert_eq!(paper.paper_id, "NCT03850691");
    }

    #[tokio::test]
    async fn test_get_by_doi_at_no_match() {
        let mut server = mockito::Server::new_async().await;
        let doi = "10.9999/not-cited";
        let mock = server
            .mock(
                "GET",
                format!(
                    "/studies?filter.advanced={}&pageSize=1&fields={}",
                    urlencoding::encode(&format!("AREA[ReferenceCitation]{doi}")),
                    urlencoding::encode(FIELDS)
                )
                .as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"studies":[]}"#)
            .expect(1)
            .create_async()
            .await;

        let source = ClinicalTrialsSource::with_client(Arc::new(test_client()));
        let result = source
            .get_by_doi_at(&format!("{}/studies", server.url()), doi)
            .await;
        mock.assert_async().await;
        assert!(matches!(result, Err(SourceError::NotFound(_))));
    }
}
