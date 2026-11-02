//! PubMed research source implementation using E-utilities API.
//!
//! Search queries support MeSH vocabulary clauses and publication-type ([pt]) pushdown.

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

    /// AND a filter clause onto the search term, tolerating an empty term.
    fn and_term(term: String, clause: String) -> String {
        if term.is_empty() {
            clause
        } else {
            format!("{} AND {}", term, clause)
        }
    }

    /// Combine the query term with MeSH clauses from `filters["mesh"]`.
    ///
    /// Clauses are semicolon-separated: `Term` searches the heading exploded
    /// (`Term[MeSH Terms]`), `Term:noexp` restricts to the exact heading
    /// (`Term[MeSH Terms:noexp]`), `Term/subheading` qualifies the heading,
    /// and a clause already containing `[` passes through verbatim.
    fn apply_mesh_clauses(query: &str, mesh: &str) -> String {
        let mut term = query.trim().to_string();
        for clause in mesh.split(';').map(str::trim).filter(|c| !c.is_empty()) {
            let clause = if clause.contains('[') {
                clause.to_string()
            } else {
                match clause.strip_suffix(":noexp") {
                    Some(heading) => format!("{}[MeSH Terms:noexp]", heading.trim()),
                    None => format!("{}[MeSH Terms]", clause),
                }
            };
            term = Self::and_term(term, clause);
        }
        term
    }

    /// Map a publication-type token to its PubMed [pt] tag, normalizing
    /// known tokens case-insensitively; unknown tokens pass through verbatim.
    fn publication_type_tag(token: &str) -> String {
        match token.trim().to_ascii_lowercase().as_str() {
            "review" => "Review".to_string(),
            "meta-analysis" => "Meta-Analysis".to_string(),
            "systematic-review" => "Systematic Review".to_string(),
            "randomized-controlled-trial" | "rct" => "Randomized Controlled Trial".to_string(),
            "clinical-trial" => "Clinical Trial".to_string(),
            "case-report" => "Case Reports".to_string(),
            "guideline" => "Practice Guideline".to_string(),
            "journal-article" | "article" => "Journal Article".to_string(),
            _ => token.trim().to_string(),
        }
    }

    /// Build the publication-type filter clause: `(Tok[pt] OR Tok[pt])` for
    /// includes, `NOT (...)` for excludes; None when no non-empty tokens.
    fn publication_type_clause(tokens: &[String], exclude: bool) -> Option<String> {
        let tags: Vec<String> = tokens
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .map(|t| format!("{}[pt]", Self::publication_type_tag(t)))
            .collect();
        if tags.is_empty() {
            return None;
        }
        if exclude {
            Some(format!("NOT ({})", tags.join(" OR ")))
        } else {
            Some(format!("({})", tags.join(" OR ")))
        }
    }

    /// Build E-utilities search URL
    fn build_search_url(&self, query: &SearchQuery) -> String {
        // Push MeSH and publication-type filters down into the ESearch term.
        let mut term = query.query.clone();
        if let Some(mesh) = query.filters.get("mesh") {
            term = Self::apply_mesh_clauses(&term, mesh);
        }
        if let Some(clause) = Self::publication_type_clause(&query.include_types, false) {
            term = Self::and_term(term, clause);
        }
        if let Some(clause) = Self::publication_type_clause(&query.exclude_types, true) {
            term = Self::and_term(term, clause);
        }
        // Author filter: ANDed into the single term parameter. E-utilities
        // takes one term; a second term= param would be dropped.
        if let Some(author) = &query.author {
            term = Self::and_term(term, format!("{}[AUTH]", author));
        }

        let mut params = vec![
            ("db".to_string(), "pubmed".to_string()),
            ("term".to_string(), term),
            ("retmax".to_string(), query.max_results.to_string()),
            ("retmode".to_string(), "xml".to_string()),
        ];

        // Add year filter if specified. E-utilities bounds dates through
        // mindate/maxdate under datetype=pdat; the reldate parameter expects
        // a day count, not a range, so open-ended years set only the
        // relevant bound.
        if let Some(year) = &query.year {
            let digits_only = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
            if let Some(end) = year.strip_prefix('-').filter(|y| digits_only(y)) {
                // "-2020": until year
                params.push(("datetype".to_string(), "pdat".to_string()));
                params.push(("maxdate".to_string(), format!("{}-12-31", end)));
            } else if let Some(start) = year.strip_suffix('-').filter(|y| digits_only(y)) {
                // "2020-": from year
                params.push(("datetype".to_string(), "pdat".to_string()));
                params.push(("mindate".to_string(), format!("{}-01-01", start)));
            } else if year.contains('-') {
                // Range "2019-2021"
                let parts: Vec<&str> = year.splitn(2, '-').collect();
                if parts.len() == 2 && parts.iter().all(|p| digits_only(p)) {
                    params.push(("datetype".to_string(), "pdat".to_string()));
                    params.push(("mindate".to_string(), format!("{}-01-01", parts[0])));
                    params.push(("maxdate".to_string(), format!("{}-12-31", parts[1])));
                }
            } else if year.len() == 4 && digits_only(year) {
                // Single year
                params.push(("datetype".to_string(), "pdat".to_string()));
                params.push(("mindate".to_string(), format!("{}-01-01", year)));
                params.push(("maxdate".to_string(), format!("{}-12-31", year)));
            }
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

    /// Remove lowercase inline-markup tags (<italic>, <sub>, <xref>, ...) that
    /// PubMed embeds inside titles and abstracts: the mixed content they
    /// create splits `$text` into duplicate events and breaks deserialization.
    /// PubMed data elements are always Capitalized, so lowercase tags carry
    /// no structural meaning here.
    fn strip_inline_markup(xml: &str) -> String {
        use std::sync::OnceLock;
        static RE: OnceLock<regex::Regex> = OnceLock::new();
        RE.get_or_init(|| {
            regex::Regex::new(r#"</?[a-z][a-zA-Z0-9:_.-]*(?:\s[^>]*)?/?>"#)
                .expect("inline markup pattern is valid")
        })
        .replace_all(xml, "")
        .into_owned()
    }

    /// Parse E-utilities fetch response XML
    fn parse_fetch_response(xml: &str) -> Result<Vec<Paper>, SourceError> {
        let xml = Self::strip_inline_markup(xml);
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
            PublicationTypeList: Option<PublicationTypeList>,
        }

        #[derive(Debug, Deserialize)]
        struct PublicationTypeList {
            #[serde(rename = "PublicationType", default)]
            types: Vec<PublicationType>,
        }

        #[derive(Debug, Deserialize)]
        struct PublicationType {
            #[serde(rename = "$text")]
            value: String,
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
            #[serde(rename = "@Label")]
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
            // Attributes need the `@` prefix in quick-xml deserialization;
            // without it this never matched and every efetch parse failed.
            #[serde(rename = "@IdType")]
            id_type: String,
            #[serde(rename = "$text")]
            value: String,
        }

        let result: PubmedArticleSet = from_str(&xml).map_err(|e| {
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

            let paper_type = article
                .MedlineCitation
                .as_ref()
                .and_then(|m| m.Article.as_ref())
                .and_then(|a| a.PublicationTypeList.as_ref())
                .map(|ptl| {
                    ptl.types
                        .iter()
                        .map(|pt| pt.value.as_str())
                        .collect::<Vec<_>>()
                        .join(";")
                })
                .filter(|s| !s.is_empty());

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
                    .paper_type(paper_type.unwrap_or_default())
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
    fn test_build_search_url_with_year_until() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer").year("-2020");
        let url = source.build_search_url(&query);

        // Until-year sets only maxdate; no reldate (it means a day count).
        assert!(url.contains("datetype=pdat"));
        assert!(url.contains("maxdate=2020-12-31"));
        assert!(!url.contains("mindate"));
        assert!(!url.contains("reldate"));
    }

    #[test]
    fn test_build_search_url_with_year_from() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer").year("2020-");
        let url = source.build_search_url(&query);

        // From-year sets only mindate; no reldate (it means a day count).
        assert!(url.contains("datetype=pdat"));
        assert!(url.contains("mindate=2020-01-01"));
        assert!(!url.contains("maxdate"));
        assert!(!url.contains("reldate"));
    }

    #[test]
    fn test_build_search_url_with_author() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer").author("Smith J");
        let url = source.build_search_url(&query);

        // Author is ANDed into the single term parameter.
        assert!(url.contains("term=cancer%20AND%20Smith%20J%5BAUTH%5D"));
        assert_eq!(url.matches("term=").count(), 1);
    }

    #[test]
    fn test_build_search_url_complex() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer")
            .year("2019-2021")
            .author("Smith J")
            .max_results(50);
        let url = source.build_search_url(&query);

        assert!(url.contains("term=cancer%20AND%20Smith%20J%5BAUTH%5D"));
        assert_eq!(url.matches("term=").count(), 1);
        assert!(url.contains("retmax=50"));
        assert!(url.contains("mindate=2019-01-01"));
        assert!(url.contains("maxdate=2021-12-31"));
    }

    #[test]
    fn test_apply_mesh_clauses_term_forms() {
        assert_eq!(
            PubMedSource::apply_mesh_clauses("cancer", "Neoplasms"),
            "cancer AND Neoplasms[MeSH Terms]"
        );
        assert_eq!(
            PubMedSource::apply_mesh_clauses("cancer", "Neoplasms:noexp"),
            "cancer AND Neoplasms[MeSH Terms:noexp]"
        );
        assert_eq!(
            PubMedSource::apply_mesh_clauses("cancer", "Neoplasms/therapy"),
            "cancer AND Neoplasms/therapy[MeSH Terms]"
        );
        assert_eq!(
            PubMedSource::apply_mesh_clauses("cancer", "Neoplasms[MeSH Major Topic]"),
            "cancer AND Neoplasms[MeSH Major Topic]"
        );
    }

    #[test]
    fn test_apply_mesh_clauses_multiple_clauses_and_empty_query() {
        assert_eq!(
            PubMedSource::apply_mesh_clauses("cancer", "Neoplasms; Diabetes Mellitus:noexp"),
            "cancer AND Neoplasms[MeSH Terms] AND Diabetes Mellitus[MeSH Terms:noexp]"
        );
        assert_eq!(
            PubMedSource::apply_mesh_clauses("", "Neoplasms"),
            "Neoplasms[MeSH Terms]"
        );
    }

    #[test]
    fn test_build_search_url_with_mesh() {
        let source = PubMedSource::new().unwrap();
        let query = SearchQuery::new("cancer").filter("mesh", "Neoplasms:noexp");
        let url = source.build_search_url(&query);

        assert!(url.contains("term=cancer%20AND%20Neoplasms%5BMeSH%20Terms%3Anoexp%5D"));
    }

    #[test]
    fn test_publication_type_tag_mapping() {
        assert_eq!(PubMedSource::publication_type_tag("review"), "Review");
        assert_eq!(PubMedSource::publication_type_tag("Review"), "Review");
        assert_eq!(
            PubMedSource::publication_type_tag("meta-analysis"),
            "Meta-Analysis"
        );
        assert_eq!(
            PubMedSource::publication_type_tag("systematic-review"),
            "Systematic Review"
        );
        assert_eq!(
            PubMedSource::publication_type_tag("rct"),
            "Randomized Controlled Trial"
        );
        assert_eq!(
            PubMedSource::publication_type_tag("randomized-controlled-trial"),
            "Randomized Controlled Trial"
        );
        assert_eq!(
            PubMedSource::publication_type_tag("clinical-trial"),
            "Clinical Trial"
        );
        assert_eq!(
            PubMedSource::publication_type_tag("case-report"),
            "Case Reports"
        );
        assert_eq!(
            PubMedSource::publication_type_tag("guideline"),
            "Practice Guideline"
        );
        assert_eq!(
            PubMedSource::publication_type_tag("journal-article"),
            "Journal Article"
        );
        assert_eq!(
            PubMedSource::publication_type_tag("article"),
            "Journal Article"
        );
        assert_eq!(PubMedSource::publication_type_tag("Letter"), "Letter");
    }

    #[test]
    fn test_publication_type_clause_polarities() {
        let include = vec!["review".to_string(), "cross-section".to_string()];
        assert_eq!(
            PubMedSource::publication_type_clause(&include, false).unwrap(),
            "(Review[pt] OR cross-section[pt])"
        );
        let exclude = vec!["case-report".to_string()];
        assert_eq!(
            PubMedSource::publication_type_clause(&exclude, true).unwrap(),
            "NOT (Case Reports[pt])"
        );
        assert!(PubMedSource::publication_type_clause(&[], false).is_none());
    }

    #[test]
    fn test_build_search_url_with_publication_types() {
        let source = PubMedSource::new().unwrap();
        let mut query = SearchQuery::new("sepsis");
        query.include_types = vec!["review".to_string(), "meta-analysis".to_string()];
        query.exclude_types = vec!["case-report".to_string()];
        let url = source.build_search_url(&query);

        assert!(url.contains(
            "term=sepsis%20AND%20%28Review%5Bpt%5D%20OR%20Meta-Analysis%5Bpt%5D%29%20AND%20NOT%20%28Case%20Reports%5Bpt%5D%29"
        ));
    }

    #[test]
    fn test_parse_fetch_response_paper_type() {
        let xml = r#"<?xml version="1.0" ?>
<!DOCTYPE PubmedArticleSet PUBLIC "-//NLM//DTD PubMedArticle, 1st January 2023//EN" "https://dtd.nlm.nih.gov/ncbi/pubmed/out/pubmed_230101.dtd">
<PubmedArticleSet>
  <PubmedArticle>
    <MedlineCitation Status="In-Data-Review" Owner="NLM">
      <PMID Version="1">35652452</PMID>
      <Article PubModel="Electronic">
        <Journal>
          <JournalIssue PubStatus="epublish">
            <PubDate>
              <Year>2022</Year>
            </PubDate>
          </JournalIssue>
        </Journal>
        <ArticleTitle>Magnetic resonance imaging in multiple sclerosis: a review.</ArticleTitle>
        <AuthorList>
          <Author>
            <LastName>Smith</LastName>
            <ForeName>John</ForeName>
            <Initials>J</Initials>
          </Author>
        </AuthorList>
        <Language>eng</Language>
        <PublicationTypeList>
          <PublicationType UI="D016428">Journal Article</PublicationType>
          <PublicationType UI="D016454">Review</PublicationType>
        </PublicationTypeList>
      </Article>
    </MedlineCitation>
    <PubmedData>
      <ArticleIdList>
        <ArticleId IdType="pubmed">35652452</ArticleId>
        <ArticleId IdType="doi">10.1002/example.2022</ArticleId>
      </ArticleIdList>
    </PubmedData>
  </PubmedArticle>
</PubmedArticleSet>"#;

        let papers = PubMedSource::parse_fetch_response(xml).unwrap();
        assert_eq!(papers.len(), 1);
        assert_eq!(
            papers[0].paper_type.as_deref(),
            Some("Journal Article;Review")
        );
        assert_eq!(papers[0].doi.as_deref(), Some("10.1002/example.2022"));
    }
}
