//! DBLP research source implementation.
//!
//! Uses DBLP XML API for computer science bibliography.

use async_trait::async_trait;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::sync::Arc;

use crate::models::{Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType};
use crate::sources::{Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const DBLP_BASE_URL: &str = "https://dblp.org";
const DBLP_SEARCH_URL: &str = "https://dblp.org/search/publ/api";

/// DBLP research source
///
/// Uses DBLP XML API for computer science bibliography.
#[derive(Debug, Clone)]
pub struct DblpSource {
    client: Arc<HttpClient>,
}

impl DblpSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::new()?),
        })
    }

    /// Parse DBLP XML response into papers using quick-xml
    fn parse_xml(&self, xml_content: &str) -> Result<Vec<Paper>, SourceError> {
        let mut reader = Reader::from_str(xml_content);
        // trim_text is not available in newer quick-xml, configure via config
        let mut buf = Vec::new();

        let mut papers = Vec::new();

        // State for parsing hit elements
        let mut in_hit = false;
        let mut current_hit_key = String::new();
        let mut current_hit_title = String::new();
        let mut current_authors: Vec<String> = Vec::new();
        let mut current_year = String::new();
        let mut current_venue = String::new();
        let mut current_pub_type = String::new();
        let mut current_doi = String::new();
        let mut current_abstract = String::new();
        let mut hit_depth = 0;
        let mut current_tag = String::new();
        let mut title_depth = 0usize;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    if e.name().as_ref() == b"hit" {
                        in_hit = true;
                        hit_depth = 1;
                        current_hit_key.clear();
                        // Reset other fields
                        current_hit_title.clear();
                        current_authors.clear();
                        current_year.clear();
                        current_venue.clear();
                        current_pub_type.clear();
                        current_doi.clear();
                        current_abstract.clear();
                    } else if in_hit {
                        hit_depth += 1;
                        if title_depth > 0 {
                            title_depth += 1;
                        } else {
                            current_tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                            if current_tag == "title" {
                                title_depth = 1;
                            }
                        }
                    }
                }
                Ok(Event::Text(e)) => {
                    if in_hit {
                        let text = e.unescape().unwrap_or_default();
                        if title_depth > 0 {
                            current_hit_title.push_str(&text);
                            continue;
                        }
                        let text = text.trim().to_string();
                        if !text.is_empty() {
                            match current_tag.as_str() {
                                "key" => current_hit_key = text,
                                "title" => current_hit_title.push_str(&text),
                                "author" => current_authors.push(text),
                                "year" => current_year = text,
                                "venue" => current_venue = text,
                                "doi" => current_doi = text,
                                "ee" if text.starts_with("https://doi.org/") => {
                                    current_doi =
                                        text.trim_start_matches("https://doi.org/").to_string()
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    if e.name().as_ref() == b"hit" {
                        if hit_depth == 1 && !current_hit_key.is_empty() {
                            // Create paper from parsed hit
                            let url = format!("{}/rec/{}.html", DBLP_BASE_URL, current_hit_key);

                            // Limit abstract length
                            let abstract_text = if current_abstract.len() > 2000 {
                                current_abstract[..2000].to_string()
                            } else {
                                current_abstract.clone()
                            };

                            let paper = PaperBuilder::new(
                                current_hit_key.clone(),
                                current_hit_title.trim().to_string(),
                                url,
                                SourceType::DBLP,
                            )
                            .authors(current_authors.join("; "))
                            .abstract_text(abstract_text)
                            .doi(current_doi.clone())
                            .published_date(current_year.clone())
                            .categories(current_venue.clone())
                            .build();

                            papers.push(paper);
                        }
                        in_hit = false;
                    } else if in_hit {
                        hit_depth -= 1;
                        if title_depth > 0 {
                            title_depth = title_depth.saturating_sub(1);
                        }
                        if title_depth == 0 {
                            current_tag.clear();
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Ok(_) => {
                    // Ignore other events (Empty, Comment, CData, Decl, PI)
                    if in_hit {
                        // Handle Empty events for self-closing tags
                    }
                }
                Err(e) => {
                    return Err(SourceError::Parse(format!("XML parsing error: {}", e)));
                }
            }
            buf.clear();
        }

        Ok(papers)
    }
}

impl Default for DblpSource {
    fn default() -> Self {
        Self::new().expect("Failed to create DblpSource")
    }
}

#[async_trait]
impl Source for DblpSource {
    fn id(&self) -> &str {
        "dblp"
    }

    fn name(&self) -> &str {
        "DBLP"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let mut url = format!(
            "?q={}&h={}&format=xml",
            urlencoding::encode(&query.query),
            query.max_results.min(1000)
        );

        // Add year filter if specified
        if let Some(year) = &query.year {
            if year.contains('-') {
                let parts: Vec<&str> = year.split('-').collect();
                if parts.len() == 2 {
                    url = format!(
                        "{}&yearMin={}&yearMax={}",
                        url,
                        parts[0].trim(),
                        parts[1].trim()
                    );
                }
            } else {
                url = format!("{}&yearMin={}&yearMax={}", url, year, year);
            }
        }

        // Clone values for retry closure
        let client = Arc::clone(&self.client);
        let url_for_retry = url.clone();

        let response = with_retry(api_retry_config(), || {
            let client = Arc::clone(&client);
            let url = url_for_retry.clone();
            async move {
                let response = client
                    .get(&format!("{}{}", DBLP_SEARCH_URL, url))
                    .header("Accept", "application/xml")
                    .send()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to search DBLP: {}", e)))?;

                if response.status() == 204 {
                    return Err(SourceError::NotFound("No results".to_string()));
                }

                if !response.status().is_success() {
                    let status = response.status();
                    // DBLP may return 500 for server errors or rate limiting
                    // Return empty results gracefully
                    if status == reqwest::StatusCode::INTERNAL_SERVER_ERROR {
                        tracing::debug!("DBLP API returned 500 - returning empty results");
                        return Err(SourceError::Api("DBLP server error".to_string()));
                    }
                    return Err(SourceError::Api(format!(
                        "DBLP API returned status: {}",
                        status
                    )));
                }

                Ok(response)
            }
        })
        .await;

        // Handle the case where 204 is returned (no results) or 500 server errors
        let response = match response {
            Ok(r) => r,
            Err(SourceError::NotFound(_)) => {
                return Ok(SearchResponse::new(vec![], "DBLP", &query.query));
            }
            Err(e) => return Err(e),
        };

        let xml_content = response
            .text()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to read XML: {}", e)))?;

        let papers = self.parse_xml(&xml_content)?;

        Ok(SearchResponse::new(papers, "DBLP", &query.query))
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let query = SearchQuery::new(clean_doi.to_string()).max_results(1);

        let response = self.search(&query).await?;

        response
            .papers
            .first()
            .cloned()
            .ok_or_else(|| SourceError::NotFound("DOI not found".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_realistic_search_hit() {
        let xml = r#"<result><hits><hit id="1"><info><authors><author pid="1">Ada Lovelace</author><author pid="2">Alan Turing</author></authors><title>Computing <i>machinery</i> and <sup>2</sup>.</title><venue>Mind</venue><year>1950</year><doi>10.1234/test</doi><key>journals/mind/turing</key></info></hit></hits></result>"#;
        let papers = DblpSource::new().unwrap().parse_xml(xml).unwrap();
        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].paper_id, "journals/mind/turing");
        assert_eq!(papers[0].title, "Computing machinery and 2.");
        assert_eq!(papers[0].authors, "Ada Lovelace; Alan Turing");
        assert_eq!(papers[0].published_date.as_deref(), Some("1950"));
        assert_eq!(papers[0].categories.as_deref(), Some("Mind"));
    }
}
