//! PubMed research source implementation using E-utilities API.

use async_trait::async_trait;
use quick_xml::de::from_str;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

/// PubMed E-utilities API base URLs
const PUBMED_ESEARCH_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi";
const PUBMED_EFETCH_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi";

/// PubMed research source
///
/// Uses NCBI E-utilities API for searching and fetching PubMed records.
#[derive(Debug, Clone)]
pub struct PubMedSource {
    client: Arc<HttpClient>,
}

impl PubMedSource {
    /// Create a new PubMed source
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

    /// Build E-utilities search URL
    fn build_search_url(&self, query: &SearchQuery) -> String {
        let mut params = vec![
            ("db".to_string(), "pubmed".to_string()),
            ("term".to_string(), query.query.clone()),
            ("retmax".to_string(), query.max_results.to_string()),
            ("retmode".to_string(), "xml".to_string()),
        ];

        // Add year filter if specified
        if let Some(year) = &query.year {
            // PubMed uses various date formats
            if let Some(start) = year.strip_prefix('-') {
                // Until year: PDAT less than or equal to YYYY
                params.push(("datetype".to_string(), "pdat".to_string()));
                params.push(("reldate".to_string(), format!("{}-01-01:9999", start)));
            } else if let Some(end) = year.strip_suffix('-') {
                // From year: PDAT greater than or equal to YYYY
                params.push(("datetype".to_string(), "pdat".to_string()));
                params.push(("reldate".to_string(), format!("{}-01-01:9999", end)));
            } else if year.contains('-') {
                // Range
                let parts: Vec<&str> = year.splitn(2, '-').collect();
                if parts.len() == 2 {
                    params.push(("mindate".to_string(), format!("{}-01-01", parts[0])));
                    params.push(("maxdate".to_string(), format!("{}-12-31", parts[1])));
                }
            } else if year.len() == 4 {
                // Single year
                params.push(("mindate".to_string(), format!("{}-01-01", year)));
                params.push(("maxdate".to_string(), format!("{}-12-31", year)));
            }
        }

        // Add author filter
        if let Some(author) = &query.author {
            params.push(("term".to_string(), format!("{}[AUTH]", author)));
        }

        params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&")
    }

    /// Parse E-utilities search response XML
    fn parse_search_response(xml: &str) -> Result<Vec<String>, SourceError> {
        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct ESearchResult {
            IdList: IdList,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct IdList {
            #[serde(rename = "Id", default)]
            ids: Vec<String>,
        }

        let result: ESearchResult = from_str(xml)
            .map_err(|e| SourceError::Parse(format!("Failed to parse PubMed search XML: {}", e)))?;

        Ok(result.IdList.ids)
    }

    /// Build E-utilities fetch URL for specific PubMed IDs
    fn build_fetch_url(ids: &[String]) -> String {
        format!(
            "{}?db=pubmed&id={}&retmode=xml",
            PUBMED_EFETCH_URL,
            ids.join(",")
        )
    }

    /// Parse E-utilities fetch response XML
    fn parse_fetch_response(xml: &str) -> Result<Vec<Paper>, SourceError> {
        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct PubmedArticleSet {
            #[serde(rename = "PubmedArticle", default)]
            articles: Vec<PubmedArticle>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct PubmedArticle {
            MedlineCitation: Option<MedlineCitation>,
            PubmedData: Option<PubmedData>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct MedlineCitation {
            PMID: Option<Pmid>,
            Article: Option<Article>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct Pmid {
            #[serde(rename = "$text")]
            id: String,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct Article {
            Journal: Option<Journal>,
            ArticleTitle: Option<ArticleTitle>,
            Abstract: Option<Abstract>,
            AuthorList: Option<AuthorList>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct Journal {
            JournalIssue: Option<JournalIssue>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct JournalIssue {
            PubDate: Option<PubDate>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct PubDate {
            Year: Option<String>,
            #[serde(rename = "MedlineDate")]
            medline_date: Option<String>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct ArticleTitle {
            #[serde(rename = "$text")]
            title: String,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct Abstract {
            #[serde(rename = "AbstractText", default)]
            abstract_texts: Vec<AbstractText>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct AbstractText {
            #[serde(rename = "Label")]
            _label: Option<String>,
            #[serde(rename = "$text")]
            text: Option<String>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct AuthorList {
            #[serde(rename = "Author", default)]
            authors: Vec<Author>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct Author {
            LastName: Option<LastName>,
            ForeName: Option<ForeName>,
            Initials: Option<Initials>,
            CollectiveName: Option<CollectiveName>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct LastName {
            #[serde(rename = "$text")]
            name: String,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct ForeName {
            #[serde(rename = "$text")]
            name: String,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct Initials {
            #[serde(rename = "$text")]
            initials: String,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct CollectiveName {
            #[serde(rename = "$text")]
            name: String,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct PubmedData {
            ArticleIdList: Option<ArticleIdList>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(non_snake_case)]
        struct ArticleIdList {
            #[serde(rename = "ArticleId", default)]
            ids: Vec<ArticleId>,
        }

        #[derive(Debug, Deserialize)]
        struct ArticleId {
            #[serde(rename = "IdType")]
            id_type: String,
            #[serde(rename = "$text")]
            value: String,
        }

        let result: PubmedArticleSet = from_str(xml).map_err(|e| {
            let preview = xml.chars().take(500).collect::<String>();
            tracing::warn!("PubMed XML parse error (first 500 chars): {}", preview);
            SourceError::Parse(format!(
                "Failed to parse PubMed fetch XML: {}. XML preview: {}",
                e, preview
            ))
        })?;

        let mut papers = Vec::new();

        for article in result.articles {
            let pmid = article
                .MedlineCitation
                .as_ref()
                .and_then(|m| m.PMID.as_ref())
                .map(|p| p.id.clone())
                .unwrap_or_default();

            let title = article
                .MedlineCitation
                .as_ref()
                .and_then(|m| m.Article.as_ref())
                .and_then(|a| a.ArticleTitle.as_ref())
                .map(|t| t.title.clone())
                .unwrap_or_default();

            let authors = article
                .MedlineCitation
                .as_ref()
                .and_then(|m| m.Article.as_ref())
                .and_then(|a| a.AuthorList.as_ref())
                .map(|al| {
                    al.authors
                        .iter()
                        .map(|author| {
                            if let Some(collective) = &author.CollectiveName {
                                collective.name.clone()
                            } else {
                                let first = author
                                    .ForeName
                                    .as_ref()
                                    .map(|f| f.name.as_str())
                                    .unwrap_or("");
                                let last = author
                                    .LastName
                                    .as_ref()
                                    .map(|l| l.name.as_str())
                                    .unwrap_or("");
                                let initials = author
                                    .Initials
                                    .as_ref()
                                    .map(|i| i.initials.as_str())
                                    .unwrap_or("");
                                format!("{} {} {}", first, last, initials)
                                    .trim()
                                    .to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_default();

            let abstract_text = article
                .MedlineCitation
                .as_ref()
                .and_then(|m| m.Article.as_ref())
                .and_then(|a| a.Abstract.as_ref())
                .map(|ab| {
                    ab.abstract_texts
                        .iter()
                        .filter_map(|at| at.text.clone())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();

            let published_date = article
                .MedlineCitation
                .as_ref()
                .and_then(|m| m.Article.as_ref())
                .and_then(|a| a.Journal.as_ref())
                .and_then(|j| j.JournalIssue.as_ref())
                .and_then(|ji| ji.PubDate.as_ref())
                .and_then(|pd| pd.Year.as_ref().or(pd.medline_date.as_ref()))
                .cloned();

            let doi = article
                .PubmedData
                .as_ref()
                .and_then(|pd| pd.ArticleIdList.as_ref())
                .and_then(|ail| ail.ids.iter().find(|id| id.id_type == "doi"))
                .map(|id| id.value.clone());

            let url = format!("https://pubmed.ncbi.nlm.nih.gov/{}/", pmid);

            papers.push(
                PaperBuilder::new(pmid, title, url, SourceType::PubMed)
                    .authors(authors)
                    .abstract_text(abstract_text)
                    .doi(doi.unwrap_or_default())
                    .published_date(published_date.unwrap_or_default())
                    .build(),
            );
        }

        Ok(papers)
    }
}

impl Default for PubMedSource {
    fn default() -> Self {
        Self::new().expect("Failed to create PubMedSource")
    }
}

#[async_trait]
impl Source for PubMedSource {
    fn id(&self) -> &str {
        "pubmed"
    }

    fn name(&self) -> &str {
        "PubMed"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let search_url = format!("{}?{}", PUBMED_ESEARCH_URL, self.build_search_url(query));

        // Clone values for retry closure
        let client = Arc::clone(&self.client);
        let search_url_for_retry = search_url.clone();

        let xml =
            with_retry(api_retry_config(), || {
                let client = Arc::clone(&client);
                let url = search_url_for_retry.clone();
                async move {
                    let response = client.get(&url).send().await.map_err(|e| {
                        SourceError::Network(format!("Failed to search PubMed: {}", e))
                    })?;

                    if !response.status().is_success() {
                        let status = response.status();
                        // Handle rate limiting
                        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            tracing::debug!("PubMed API rate-limited - returning empty results");
                            return Err(SourceError::Api("PubMed rate-limited".to_string()));
                        }
                        // Check for service unavailable
                        if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                            tracing::debug!("PubMed API unavailable - returning empty results");
                            return Err(SourceError::Api("PubMed unavailable".to_string()));
                        }
                        return Err(SourceError::Api(format!(
                            "PubMed API returned status: {}",
                            response.status()
                        )));
                    }

                    response.text().await.map_err(|e| {
                        SourceError::Network(format!("Failed to read response: {}", e))
                    })
                }
            })
            .await;

        // Handle rate limiting gracefully
        let xml = match xml {
            Ok(x) => x,
            Err(SourceError::Api(msg)) if msg.contains("rate-limited") => {
                tracing::debug!("PubMed rate-limited - returning empty results");
                return Ok(SearchResponse::new(vec![], "PubMed", &query.query));
            }
            Err(SourceError::Api(msg)) if msg.contains("unavailable") => {
                tracing::debug!("PubMed unavailable - returning empty results");
                return Ok(SearchResponse::new(vec![], "PubMed", &query.query));
            }
            Err(e) => return Err(e),
        };

        let ids = Self::parse_search_response(&xml)?;

        if ids.is_empty() {
            return Ok(SearchResponse::new(vec![], "PubMed", &query.query));
        }

        // Fetch details for each paper (batch request)
        let fetch_url = Self::build_fetch_url(&ids);

        let client = Arc::clone(&self.client);
        let fetch_url_for_retry = fetch_url.clone();

        let fetch_xml = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = fetch_url_for_retry.clone();
            async move {
                let response = client.get(&url).send().await.map_err(|e| {
                    SourceError::Network(format!("Failed to fetch PubMed details: {}", e))
                })?;

                if !response.status().is_success() {
                    let status = response.status();
                    // Handle rate limiting
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        tracing::debug!(
                            "PubMed API rate-limited during fetch - returning empty results"
                        );
                        return Err(SourceError::Api("PubMed rate-limited".to_string()));
                    }
                    // Check for service unavailable
                    if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                        tracing::debug!(
                            "PubMed API unavailable during fetch - returning empty results"
                        );
                        return Err(SourceError::Api("PubMed unavailable".to_string()));
                    }
                    return Err(SourceError::Api(format!(
                        "PubMed API returned status: {}",
                        status
                    )));
                }

                response
                    .text()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to read response: {}", e)))
            }
        })
        .await?;

        // Handle rate limiting gracefully
        let papers = match Self::parse_fetch_response(&fetch_xml) {
            Ok(p) => p,
            Err(SourceError::Api(msg))
                if msg.contains("rate-limited") || msg.contains("unavailable") =>
            {
                tracing::debug!("PubMed fetch {} - returning empty results", msg);
                vec![]
            }
            Err(e) => return Err(e),
        };

        Ok(SearchResponse::new(papers, "PubMed", &query.query))
    }

    async fn get_by_id(&self, id: &str) -> Result<Paper, SourceError> {
        let query = crate::models::SearchQuery::new(id).max_results(1);
        let response = self.search(&query).await?;
        response
            .papers
            .into_iter()
            .next()
            .ok_or_else(|| SourceError::NotFound(format!("PubMed paper '{}' not found", id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_search_url() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("machine learning").max_results(10);
        let url = source.build_search_url(&query);

        assert!(url.contains("db=pubmed"));
        assert!(url.contains("term=machine%20learning"));
        assert!(url.contains("retmax=10"));
        assert!(url.contains("retmode=xml"));
    }

    #[test]
    fn test_build_search_url_with_year() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer").year("2020");
        let url = source.build_search_url(&query);

        assert!(url.contains("2020-01-01"));
        assert!(url.contains("2020-12-31"));
    }

    #[test]
    fn test_build_search_url_with_year_range() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer").year("2015-2020");
        let url = source.build_search_url(&query);

        assert!(url.contains("2015-01-01"));
        assert!(url.contains("2020-12-31"));
    }

    #[test]
    fn test_build_search_url_with_year_from() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer").year("2020-");
        let url = source.build_search_url(&query);

        assert!(url.contains("2020-01-01"));
    }

    #[test]
    fn test_build_search_url_with_year_until() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer").year("-2020");
        let url = source.build_search_url(&query);

        // The implementation uses reldate format for until year
        assert!(url.contains("2020-01-01"));
    }

    #[test]
    fn test_build_search_url_with_author() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer").author("Smith J");
        let url = source.build_search_url(&query);

        assert!(url.contains("Smith%20J%5BAUTH%5D"));
    }

    #[test]
    fn test_build_search_url_complex() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer")
            .year("2019-2021")
            .author("Smith J")
            .max_results(50);
        let url = source.build_search_url(&query);

        assert!(url.contains("term=cancer"));
        assert!(url.contains("term=Smith%20J%5BAUTH%5D"));
        assert!(url.contains("retmax=50"));
        assert!(url.contains("2019-01-01"));
        assert!(url.contains("2021-12-31"));
    }
}
