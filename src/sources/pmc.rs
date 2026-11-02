//! PubMed Central (PMC) research source implementation.

use async_trait::async_trait;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{
    Paper, PaperBuilder, ReadRequest, ReadResult, SearchQuery, SearchResponse, SourceType,
};
use crate::sources::{DownloadRequest, DownloadResult, Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const PMC_EUTILS_BASE: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils";
const PMC_BASE_URL: &str = "https://www.ncbi.nlm.nih.gov/pmc";

/// PMC research source
///
/// Uses NCBI E-utilities API for PubMed Central full-text papers.
#[derive(Debug, Clone)]
pub struct PmcSource {
    client: Arc<HttpClient>,
}

impl PmcSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::for_source(
                "pmc",
                &crate::utils::get_user_agent(),
            )?),
        })
    }

    /// Clean PMCID (remove PMC prefix if present)
    fn clean_pmcid(&self, pmcid: &str) -> String {
        pmcid.replace("PMC", "").trim().to_string()
    }

    /// Parse PMC XML response into Paper using quick-xml
    fn parse_pmc_xml(&self, xml_content: &str, pmcid: &str) -> Result<Paper, SourceError> {
        let mut reader = Reader::from_str(xml_content);
        let mut buf = Vec::new();

        let mut title = String::new();
        let mut abstract_text = String::new();
        let mut authors: Vec<String> = Vec::new();
        let mut current_given = String::new();
        let mut current_surname = String::new();
        let mut in_contrib = false;
        let mut journal = String::new();
        let mut year = String::new();
        let mut month = String::new();
        let mut day = String::new();
        let mut doi = String::new();

        // Track what element we're currently in for text parsing
        enum Element {
            None,
            ArticleTitle,
            Abstract,
            Contrib,
            GivenNames,
            Surname,
            JournalTitle,
            Year,
            Month,
            Day,
            ArticleId,
        }
        let mut current_element = Element::None;
        let mut title_depth = 0usize;
        let mut abstract_depth = 0usize;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    // Convert element name to owned String to avoid lifetime issues
                    let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    if title_depth > 0 {
                        title_depth += 1;
                        continue;
                    }
                    if abstract_depth > 0 {
                        if tag_name == "p"
                            && !abstract_text.is_empty()
                            && !abstract_text.ends_with(char::is_whitespace)
                        {
                            abstract_text.push(' ');
                        }
                        abstract_depth += 1;
                        continue;
                    }
                    match tag_name.as_str() {
                        "article-title" => {
                            title_depth = 1;
                            current_element = Element::ArticleTitle;
                        }
                        "abstract" => {
                            abstract_depth = 1;
                            current_element = Element::Abstract;
                        }
                        "contrib" => {
                            // Check if this is an author contrib
                            if let Some(contrib_type) = get_attr(e, "contrib-type") {
                                if contrib_type == "author" {
                                    in_contrib = true;
                                    current_given.clear();
                                    current_surname.clear();
                                    current_element = Element::Contrib;
                                } else {
                                    current_element = Element::None;
                                }
                            } else {
                                current_element = Element::None;
                            }
                        }
                        "given-names" => {
                            current_element = Element::GivenNames;
                        }
                        "surname" => {
                            current_element = Element::Surname;
                        }
                        "journal-title" => {
                            current_element = Element::JournalTitle;
                        }
                        "year" => {
                            current_element = Element::Year;
                        }
                        "month" => {
                            current_element = Element::Month;
                        }
                        "day" => {
                            current_element = Element::Day;
                        }
                        "article-id" => {
                            if let Some(pub_id_type) = get_attr(e, "pub-id-type") {
                                if pub_id_type == "doi" {
                                    current_element = Element::ArticleId;
                                } else {
                                    current_element = Element::None;
                                }
                            } else {
                                current_element = Element::None;
                            }
                        }
                        _ => current_element = Element::None,
                    }
                }
                Ok(Event::Text(e)) => {
                    let raw = e.unescape().unwrap_or_default();
                    if matches!(current_element, Element::ArticleTitle) {
                        title.push_str(&raw);
                        continue;
                    }
                    if matches!(current_element, Element::Abstract) {
                        abstract_text.push_str(&raw);
                        continue;
                    }
                    let text = raw.trim().to_string();
                    if text.is_empty() {
                        continue;
                    }

                    match current_element {
                        Element::ArticleTitle | Element::Abstract => unreachable!(),
                        Element::Contrib => {
                            if current_surname.is_empty() {
                                current_surname = text;
                            } else if current_given.is_empty() {
                                current_given = text;
                            }
                        }
                        Element::GivenNames => {
                            current_given = text;
                        }
                        Element::Surname => {
                            current_surname = text;
                        }
                        Element::JournalTitle => {
                            journal = text;
                        }
                        Element::Year => {
                            year = text;
                        }
                        Element::Month => {
                            month = text;
                        }
                        Element::Day => {
                            day = text;
                        }
                        Element::ArticleId => {
                            doi = text;
                        }
                        Element::None => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    if title_depth > 0 {
                        title_depth -= 1;
                        if title_depth == 0 {
                            current_element = Element::None;
                        }
                        continue;
                    }
                    if abstract_depth > 0 {
                        abstract_depth -= 1;
                        if abstract_depth == 0 {
                            current_element = Element::None;
                        }
                        continue;
                    }
                    // Convert element name to owned String to avoid lifetime issues
                    let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    match tag_name.as_str() {
                        "article-title" => {
                            current_element = Element::None;
                        }
                        "abstract" => {
                            current_element = Element::None;
                        }
                        "contrib" => {
                            if in_contrib {
                                // Combine given names and surname
                                if !current_given.is_empty() && !current_surname.is_empty() {
                                    authors.push(format!("{} {}", current_given, current_surname));
                                } else if !current_surname.is_empty() {
                                    authors.push(current_surname.clone());
                                }
                                in_contrib = false;
                            }
                            current_element = Element::None;
                        }
                        "given-names" => {
                            current_element = Element::None;
                        }
                        "surname" => {
                            current_element = Element::None;
                        }
                        "journal-title" => {
                            current_element = Element::None;
                        }
                        "year" => {
                            current_element = Element::None;
                        }
                        "month" => {
                            current_element = Element::None;
                        }
                        "day" => {
                            current_element = Element::None;
                        }
                        "article-id" => {
                            current_element = Element::None;
                        }
                        _ => current_element = Element::None,
                    }
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(e) => {
                    return Err(SourceError::Parse(format!("XML parsing error: {}", e)));
                }
            }
            buf.clear();
        }

        // Build date string
        let published_date = if !year.is_empty() {
            if !month.is_empty() && !day.is_empty() {
                format!("{}-{}-{}", year, month, day)
            } else {
                year
            }
        } else {
            String::new()
        };

        let full_pmcid = format!("PMC{}", pmcid);
        let url = format!("{}/{}", PMC_BASE_URL, full_pmcid);
        let pdf_url = format!("{}/articles/{}/pdf/", PMC_BASE_URL, full_pmcid);

        Ok(
            PaperBuilder::new(full_pmcid.clone(), title.trim(), url, SourceType::PMC)
                .authors(authors.join("; "))
                .abstract_text(abstract_text.trim())
                .doi(doi)
                .published_date(published_date)
                .categories(journal)
                .pdf_url(pdf_url)
                .build(),
        )
    }
}

impl Default for PmcSource {
    fn default() -> Self {
        Self::new().expect("Failed to create PmcSource")
    }
}

#[async_trait]
impl Source for PmcSource {
    fn id(&self) -> &str {
        "pmc"
    }

    fn name(&self) -> &str {
        "PubMed Central"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOWNLOAD | SourceCapabilities::READ
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let mut url = format!(
            "{}/esearch.fcgi?db=pmc&term={}&retmax={}&retmode=json",
            PMC_EUTILS_BASE,
            urlencoding::encode(&query.query),
            query.max_results
        );

        // Add year filter if specified
        if let Some(year) = &query.year {
            if year.contains('-') {
                // Year range - for simplicity, just use the first year
                let start_year = year.split('-').next().unwrap_or(year);
                url = format!(
                    "{}&datetype=pubmed&mindate={}/01/01&maxdate={}/12/31",
                    url, start_year, start_year
                );
            } else {
                url = format!(
                    "{}&datetype=pubmed&mindate={}/01/01&maxdate={}/12/31",
                    url, year, year
                );
            }
        }

        // Clone values for retry closure
        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let response =
                    client.get(&url).send().await.map_err(|e| {
                        SourceError::Network(format!("Failed to search PMC: {}", e))
                    })?;

                if !response.status().is_success() {
                    return Err(SourceError::Api(format!(
                        "PMC API returned status: {}",
                        response.status()
                    )));
                }

                Ok(response)
            }
        })
        .await?;

        let data: ESearchResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let pmcids = data.esearchresult.idlist;

        if pmcids.is_empty() {
            return Ok(SearchResponse::new(vec![], "PMC", &query.query));
        }

        // Fetch details for each paper
        let mut papers = Vec::new();
        let mut failures = 0;
        for pmcid in pmcids.iter().take(query.max_results) {
            match self.fetch_paper_details(pmcid).await {
                Ok(Some(paper)) => papers.push(paper),
                Ok(None) => {
                    failures += 1;
                    tracing::warn!("PMC detail missing for {}", pmcid);
                }
                Err(e) => {
                    failures += 1;
                    tracing::warn!("Error fetching PMC paper {}: {}", pmcid, e);
                }
            }
        }
        if failures > 0 && papers.is_empty() {
            return Err(SourceError::Api(format!(
                "Failed to fetch all {} PMC details",
                failures
            )));
        }

        Ok(SearchResponse::new(papers, "PMC", &query.query))
    }

    async fn download(&self, request: &DownloadRequest) -> Result<DownloadResult, SourceError> {
        let pmcid = self.clean_pmcid(&request.paper_id);
        let full_pmcid = format!("PMC{}", pmcid);

        let pdf_url = format!("{}/articles/{}/pdf/", PMC_BASE_URL, full_pmcid);

        let response = self
            .client
            .get(&pdf_url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to download PDF: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SourceError::NotFound(format!(
                "Paper not found: {}",
                full_pmcid
            )));
        }
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SourceError::RateLimit);
        }
        if !response.status().is_success() {
            return Err(SourceError::Api(format!(
                "PMC download returned status: {}",
                response.status()
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to read PDF: {}", e)))?;

        std::fs::create_dir_all(&request.save_path).map_err(|e| {
            SourceError::Io(std::io::Error::other(format!(
                "Failed to create directory: {}",
                e
            )))
        })?;

        let filename = format!("{}.pdf", full_pmcid);
        let path = std::path::Path::new(&request.save_path).join(&filename);

        std::fs::write(&path, bytes.as_ref()).map_err(SourceError::Io)?;

        Ok(DownloadResult::success(
            path.to_string_lossy().to_string(),
            bytes.len() as u64,
        ))
    }

    async fn read(&self, request: &ReadRequest) -> Result<ReadResult, SourceError> {
        let download_request = DownloadRequest::new(&request.paper_id, &request.save_path);
        let download_result = self.download(&download_request).await?;

        // Extract text from the downloaded PDF
        let pdf_path = std::path::Path::new(&download_result.path);
        match crate::utils::extract_text(pdf_path) {
            Ok((text, _method)) => {
                // Estimate page count (rough approximation based on text length)
                let pages = (text.len() / 3000).max(1);
                Ok(ReadResult::success(text).pages(pages))
            }
            Err(e) => {
                // If extraction fails, return a result indicating partial success
                Ok(ReadResult::error(format!(
                    "PDF downloaded but text extraction failed: {}",
                    e
                )))
            }
        }
    }
}

impl PmcSource {
    async fn fetch_paper_details(&self, pmcid: &str) -> Result<Option<Paper>, SourceError> {
        let url = format!(
            "{}/efetch.fcgi?db=pmc&id={}&retmode=xml",
            PMC_EUTILS_BASE, pmcid
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch paper: {}", e)))?;

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SourceError::RateLimit);
        }
        if !response.status().is_success() {
            return Err(SourceError::Api(format!(
                "PMC detail returned status: {}",
                response.status()
            )));
        }

        let xml_content = response
            .text()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to read XML: {}", e)))?;

        Ok(Some(self.parse_pmc_xml(&xml_content, pmcid)?))
    }
}

/// Get attribute value from a BytesStart element
fn get_attr<'a>(e: &BytesStart<'a>, attr_name: &str) -> Option<String> {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == attr_name.as_bytes())
        .and_then(|a| {
            std::str::from_utf8(a.value.as_ref())
                .ok()
                .map(|s| s.to_string())
        })
}

// ===== PMC API Types =====

#[derive(Debug, Deserialize)]
struct ESearchResponse {
    esearchresult: ESearchResult,
}

#[derive(Debug, Deserialize)]
struct ESearchResult {
    idlist: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nested_article_title_and_abstract() {
        let source = PmcSource::new().unwrap();
        let xml = "<article><article-title>Gene <italic>editing</italic> works</article-title><abstract><p>First <italic>important</italic> result.</p><p>Second result.</p></abstract></article>";
        let paper = source.parse_pmc_xml(xml, "123").unwrap();
        assert_eq!(paper.title, "Gene editing works");
        assert_eq!(paper.r#abstract, "First important result. Second result.");
        let xml = "<article><article-title>A <italic>gene</italic>, tested</article-title><abstract><p>One <italic>important</italic> result.</p><p>Next paragraph.</p></abstract></article>";
        let paper = source.parse_pmc_xml(xml, "123").unwrap();
        assert_eq!(paper.title, "A gene, tested");
        assert_eq!(paper.r#abstract, "One important result. Next paragraph.");
    }

    #[test]
    fn test_pmc_source_creation() {
        let source = PmcSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_pmc_capabilities() {
        let source = PmcSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::DOWNLOAD));
        assert!(caps.contains(SourceCapabilities::READ));
    }

    #[test]
    fn test_pmc_id() {
        let source = PmcSource::new().unwrap();
        assert_eq!(source.id(), "pmc");
    }

    #[test]
    fn test_pmc_name() {
        let source = PmcSource::new().unwrap();
        assert_eq!(source.name(), "PubMed Central");
    }
}
