//! HAL (French open archive) research source implementation.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use crate::models::{
    Paper, PaperBuilder, ReadRequest, ReadResult, SearchQuery, SearchResponse, SourceType,
};
use crate::sources::{DownloadRequest, DownloadResult, Source, SourceCapabilities, SourceError};
use crate::utils::{api_retry_config, with_retry, HttpClient};

const HAL_API_BASE: &str = "https://api.archives-ouvertes.fr";

/// HAL research source
///
/// Uses HAL REST API for French scientific documents archive.
#[derive(Debug, Clone)]
pub struct HalSource {
    client: Arc<HttpClient>,
}

impl HalSource {
    pub fn new() -> Result<Self, SourceError> {
        Ok(Self {
            client: Arc::new(HttpClient::for_source(
                "hal",
                &crate::utils::get_user_agent(),
            )?),
        })
    }

    /// Parse HAL document into Paper
    fn parse_doc(&self, doc: &HALDoc) -> Result<Paper, SourceError> {
        let doc_id = doc.doc_id.clone().unwrap_or_default();

        // Extract title - handle missing/empty titles gracefully
        let title = if let Some(ref titles) = doc.title_s {
            if titles.is_empty() {
                tracing::debug!("HAL document {} has empty title - skipping", doc_id);
                return Err(SourceError::Parse("Empty title".to_string()));
            }
            // Prefer English title if available
            let mut title = &titles[0];
            for t in titles {
                if t.starts_with("A ") || t.starts_with("The ") {
                    title = t;
                    break;
                }
            }
            // Ensure title is not empty
            if title.is_empty() {
                tracing::debug!("HAL document {} has empty title - skipping", doc_id);
                return Err(SourceError::Parse("Empty title".to_string()));
            }
            title.clone()
        } else {
            tracing::debug!("HAL document {} has no title field - skipping", doc_id);
            return Err(SourceError::Parse("No title".to_string()));
        };

        // Extract authors
        let authors = if let Some(ref author_names) = doc.author_name_s {
            author_names.join("; ")
        } else {
            String::new()
        };

        // Extract abstract
        let abstract_text = if let Some(ref abstracts) = doc.abstract_s {
            if abstracts.is_empty() {
                String::new()
            } else {
                abstracts[0].clone()
            }
        } else {
            String::new()
        };

        // Extract DOI
        let doi = doc
            .doi_s
            .clone()
            .or_else(|| doc.doi_id_s.clone())
            .unwrap_or_default();

        // URL
        let url = doc
            .url_s
            .clone()
            .unwrap_or_else(|| format!("https://hal.science/{}", doc_id));

        // PDF URL
        let pdf_url = doc.file_url_s.clone().unwrap_or_default();

        // Publication date
        let published_date = doc
            .produced_date_s
            .clone()
            .map(|d| d[..d.len().min(10)].to_string())
            .unwrap_or_default();

        // Domain as category
        let categories = doc.domain_s.as_deref().unwrap_or_default().join("; ");

        // Document type
        let _doc_type = doc.doc_type_s.clone().unwrap_or_default();

        Ok(
            PaperBuilder::new(doc_id.clone(), title, url, SourceType::HAL)
                .authors(authors)
                .abstract_text(abstract_text)
                .doi(doi)
                .published_date(published_date)
                .categories(categories)
                .pdf_url(pdf_url)
                .build(),
        )
    }
}

impl Default for HalSource {
    fn default() -> Self {
        Self::new().expect("Failed to create HalSource")
    }
}

#[async_trait]
impl Source for HalSource {
    fn id(&self) -> &str {
        "hal"
    }

    fn name(&self) -> &str {
        "HAL"
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOWNLOAD | SourceCapabilities::READ
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        let mut url = format!(
            "{}/search?q={}&rows={}&wt=json&fl=docid,title_s,authFullName_s,abstract_s,doi_s,doiId_s,uri_s,fileUrl_s,producedDate_s,domain_s,docType_s",
            HAL_API_BASE,
            urlencoding::encode(&query.query),
            query.max_results.min(1000)
        );

        // Add year filter if specified
        if let Some(year) = &query.year {
            if year.contains('-') {
                let parts: Vec<&str> = year.split('-').collect();
                if parts.len() == 2 {
                    url = format!(
                        "{}&fq=producedDate_s:[{} TO {}]",
                        url,
                        parts[0].trim(),
                        parts[1].trim()
                    );
                }
            } else {
                url = format!("{}&fq=producedDate_s:{}", url, year);
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
                    .get(&url)
                    .header("Accept", "application/json")
                    .send()
                    .await
                    .map_err(|e| SourceError::Network(format!("Failed to search HAL: {}", e)))?;

                if !response.status().is_success() {
                    let status = response.status();
                    // Handle rate limiting
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        tracing::debug!("HAL API rate-limited - returning empty results");
                        return Err(SourceError::Api("HAL rate-limited".to_string()));
                    }
                    // Check for redirects or errors
                    if status == reqwest::StatusCode::INTERNAL_SERVER_ERROR {
                        tracing::debug!("HAL API server error - returning empty results");
                        return Err(SourceError::Api("HAL server error".to_string()));
                    }
                    return Err(SourceError::Api(format!(
                        "HAL API returned status: {}",
                        response.status()
                    )));
                }

                // Check content-type to ensure we got JSON
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                if !content_type.contains("application/json") {
                    tracing::debug!(
                        "HAL returned non-JSON content-type: {} - rate-limited?",
                        content_type
                    );
                    return Err(SourceError::Api("HAL rate-limited".to_string()));
                }

                Ok(response)
            }
        })
        .await;

        let response = response?;

        let data: HALResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let mut papers = Vec::new();
        for doc in data.response.docs {
            match self.parse_doc(&doc) {
                Ok(paper) => papers.push(paper),
                Err(SourceError::Parse(msg)) => {
                    tracing::debug!("Skipping HAL document: {}", msg);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(SearchResponse::new(papers, "HAL", &query.query))
    }

    async fn download(&self, request: &DownloadRequest) -> Result<DownloadResult, SourceError> {
        // First, get the document to find the file URL
        let url = format!("{}/document/{}", HAL_API_BASE, request.paper_id);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to fetch document: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SourceError::NotFound(format!(
                "Document not found: {}",
                request.paper_id
            )));
        }
        if !response.status().is_success() {
            return Err(SourceError::Api(format!(
                "HAL returned {}",
                response.status()
            )));
        }

        let data: HALResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let doc = data
            .response
            .docs
            .first()
            .ok_or_else(|| SourceError::NotFound("Document not found".to_string()))?;

        let pdf_url = doc
            .file_url_s
            .as_ref()
            .ok_or_else(|| SourceError::NotFound("No file available".to_string()))?;

        let pdf_response = self
            .client
            .get(pdf_url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to download PDF: {}", e)))?;

        if pdf_response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SourceError::NotFound("PDF not available".to_string()));
        }
        if !pdf_response.status().is_success() {
            return Err(SourceError::Api(format!(
                "HAL PDF returned {}",
                pdf_response.status()
            )));
        }

        let bytes = pdf_response
            .bytes()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to read PDF: {}", e)))?;

        std::fs::create_dir_all(&request.save_path).map_err(|e| {
            SourceError::Io(std::io::Error::other(format!(
                "Failed to create directory: {}",
                e
            )))
        })?;

        let filename = format!("hal_{}.pdf", request.paper_id);
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

        let pdf_path = std::path::Path::new(&download_result.path);
        match crate::utils::extract_text(pdf_path) {
            Ok((text, _method)) => {
                let pages = (text.len() / 3000).max(1);
                Ok(ReadResult::success(text).pages(pages))
            }
            Err(e) => Ok(ReadResult::error(format!(
                "PDF downloaded but text extraction failed: {}",
                e
            ))),
        }
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        let clean_doi = doi
            .replace("https://doi.org/", "")
            .replace("doi:", "")
            .trim()
            .to_string();

        let url = format!(
            "{}/search?fq=doi_s:{}&rows=1&wt=json&fl=docid,title_s,authFullName_s,abstract_s,doi_s,doiId_s,uri_s,fileUrl_s,producedDate_s,domain_s,docType_s",
            HAL_API_BASE,
            urlencoding::encode(&clean_doi)
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SourceError::Network(format!("Failed to search DOI: {}", e)))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SourceError::NotFound("DOI not found".to_string()));
        }
        if !response.status().is_success() {
            return Err(SourceError::Api(format!(
                "HAL returned {}",
                response.status()
            )));
        }

        let data: HALResponse = response
            .json()
            .await
            .map_err(|e| SourceError::Parse(format!("Failed to parse JSON: {}", e)))?;

        let doc = data
            .response
            .docs
            .first()
            .ok_or_else(|| SourceError::NotFound("DOI not found".to_string()))?;

        self.parse_doc(doc)
    }
}

// ===== HAL API Types =====

#[derive(Debug, Deserialize)]
struct HALResponse {
    response: HALResponseInner,
}

#[derive(Debug, Deserialize)]
struct HALResponseInner {
    #[serde(rename = "numFound")]
    #[allow(dead_code)]
    num_found: usize,
    docs: Vec<HALDoc>,
}

#[derive(Debug, Deserialize)]
struct HALDoc {
    #[serde(rename = "docid")]
    doc_id: Option<String>,
    #[serde(rename = "title_s")]
    title_s: Option<Vec<String>>,
    #[serde(rename = "authFullName_s")]
    author_name_s: Option<Vec<String>>,
    #[serde(rename = "abstract_s")]
    abstract_s: Option<Vec<String>>,
    #[serde(rename = "doi_s")]
    doi_s: Option<String>,
    #[serde(rename = "doiId_s")]
    doi_id_s: Option<String>,
    #[serde(rename = "uri_s")]
    url_s: Option<String>,
    #[serde(rename = "fileUrl_s")]
    file_url_s: Option<String>,
    #[serde(rename = "producedDate_s")]
    produced_date_s: Option<String>,
    #[serde(rename = "domain_s")]
    domain_s: Option<Vec<String>>,
    #[serde(rename = "docType_s")]
    doc_type_s: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hal_live_shape() {
        let response: HALResponse = serde_json::from_str(r#"{"response":{"numFound":1,"docs":[{"docid":"4672710","title_s":["Title"],"authFullName_s":["Polina Muzyukina"],"domain_s":["0.sdv"]}]}}"#).unwrap();
        let paper = HalSource::new()
            .unwrap()
            .parse_doc(&response.response.docs[0])
            .unwrap();
        assert_eq!(paper.paper_id, "4672710");
        assert_eq!(paper.authors, "Polina Muzyukina");
    }

    #[test]
    fn test_hal_source_creation() {
        let source = HalSource::new();
        assert!(source.is_ok());
    }

    #[test]
    fn test_hal_capabilities() {
        let source = HalSource::new().unwrap();
        let caps = source.capabilities();
        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::DOWNLOAD));
    }

    #[test]
    fn test_hal_id() {
        let source = HalSource::new().unwrap();
        assert_eq!(source.id(), "hal");
    }

    #[test]
    fn test_hal_name() {
        let source = HalSource::new().unwrap();
        assert_eq!(source.name(), "HAL");
    }
}
