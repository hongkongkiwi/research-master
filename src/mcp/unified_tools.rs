//! Unified tool handlers with smart source selection.

use std::sync::Arc;

use futures_util::future;
use serde_json::Value;

use super::tools::ToolHandler;

/// Helper function to auto-detect the appropriate source for a paper ID
pub(crate) fn auto_detect_source(
    sources: &Arc<Vec<Arc<dyn crate::sources::Source>>>,
    paper_id: &str,
) -> Result<Arc<dyn crate::sources::Source>, String> {
    let paper_id_lower = paper_id.to_lowercase();

    if paper_id_lower.starts_with("arxiv:")
        || (paper_id.len() > 4 && paper_id.chars().take(9).all(|c| c.is_numeric() || c == '.'))
    {
        return sources
            .iter()
            .find(|s| s.id() == "arxiv")
            .cloned()
            .ok_or_else(|| "arXiv source not available".to_string());
    }

    if paper_id_upper_start(paper_id, "PMC") {
        return sources
            .iter()
            .find(|s| s.id() == "pmc")
            .cloned()
            .ok_or_else(|| "PMC source not available".to_string());
    }

    if paper_id_lower.starts_with("hal-") {
        return sources
            .iter()
            .find(|s| s.id() == "hal")
            .cloned()
            .ok_or_else(|| "HAL source not available".to_string());
    }

    // ClinicalTrials.gov NCT ids: NCT followed by exactly 8 digits.
    if paper_id.len() == 11
        && paper_id_upper_start(paper_id, "NCT")
        && paper_id[3..].chars().all(|c| c.is_ascii_digit())
    {
        return sources
            .iter()
            .find(|s| s.id() == "clinical_trials")
            .cloned()
            .ok_or_else(|| "ClinicalTrials.gov source not available".to_string());
    }

    // DOIs (`10.1234/rest`) contain exactly one slash, the same shape as an
    // IACR id (`2023/1234`). Check for a DOI first or every DOI is sent to IACR.
    if paper_id.starts_with("10.") {
        if let Some(source) = sources
            .iter()
            .find(|s| s.id() == "semantic" && s.supports_doi_lookup())
        {
            return Ok(Arc::clone(source));
        }
        if let Some(source) = sources.iter().find(|s| s.supports_doi_lookup()) {
            return Ok(Arc::clone(source));
        }
    }

    if paper_id.chars().filter(|&c| c == '/').count() == 1 {
        return sources
            .iter()
            .find(|s| s.id() == "iacr")
            .cloned()
            .ok_or_else(|| "IACR source not available".to_string());
    }

    if let Some(source) = sources.iter().find(|s| s.id() == "arxiv") {
        return Ok(Arc::clone(source));
    }

    if let Some(source) = sources.iter().find(|s| s.id() == "semantic") {
        return Ok(Arc::clone(source));
    }

    Err("Could not auto-detect source. Please specify source explicitly.".to_string())
}

fn paper_id_upper_start(paper_id: &str, prefix: &str) -> bool {
    if paper_id.len() < prefix.len() {
        return false;
    }
    paper_id[..prefix.len()].to_uppercase() == prefix
}

/// Collect a JSON array of strings into a Vec, ignoring non-string entries
fn string_list_arg(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug)]
pub struct SearchPapersHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for SearchPapersHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'query' parameter")?;
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;
        let year = args
            .get("year")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let category = args
            .get("category")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let source_filter = args.get("source").and_then(|v| v.as_str());
        let license_policy = match args.get("license").and_then(|v| v.as_str()) {
            Some(value) => crate::models::LicensePolicy::parse(value).ok_or_else(|| {
                format!("Unknown license policy '{value}'. Use any, open, or ignore.")
            })?,
            None => crate::models::LicensePolicy::Any,
        };

        let mut search_query = crate::models::SearchQuery::new(query)
            .max_results(max_results)
            .license_policy(license_policy)
            .include_types(string_list_arg(args.get("include_types")))
            .exclude_types(string_list_arg(args.get("exclude_types")))
            .strict_types(
                args.get("strict_types")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            );
        if let Some(ref year) = year {
            search_query = search_query.year(year);
        }
        if let Some(ref cat) = category {
            search_query = search_query.category(cat);
        }
        // Sources that index full text (Europe PMC, OpenAlex) restrict to
        // full-text-available works when this filter is set.
        if args
            .get("fulltext")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            search_query = search_query.filter("fulltext", "true");
        }
        // Generic source-specific filters, forwarded as-is; sources ignore
        // keys they do not support.
        if let Some(extra) = args.get("filters").and_then(|v| v.as_object()) {
            for (key, value) in extra {
                if let Some(value) = value.as_str() {
                    search_query = search_query.filter(key.clone(), value.to_string());
                }
            }
        }
        let search_query = Arc::new(search_query);

        let futures: Vec<_> = self
            .sources
            .iter()
            .filter(|s| {
                if let Some(filter) = source_filter {
                    if s.id() != filter {
                        return false;
                    }
                }
                s.supports_search()
            })
            .map(|source| {
                let source = Arc::clone(source);
                let sq = Arc::clone(&search_query);
                async move {
                    let cache =
                        crate::utils::CacheService::from_config(crate::config::get_config().cache);
                    if let crate::utils::CacheResult::Hit(response) =
                        cache.get_search(&sq, source.id())
                    {
                        return Ok(response.papers);
                    }
                    match source.search(&sq).await {
                        Ok(response) => {
                            cache.set_search(source.id(), &sq, &response);
                            Ok(response.papers)
                        }
                        Err(e) => {
                            tracing::warn!("Search failed for {}: {}", source.id(), e);
                            Err(e)
                        }
                    }
                }
            })
            .collect();

        let results = future::join_all(futures).await;
        let mut all_results = Vec::new();
        let mut failures = Vec::new();
        for result in results {
            match result {
                Ok(papers) => all_results.extend(papers),
                Err(e) => failures.push(e.to_string()),
            }
        }
        // Empty with failures is not the same as "no results": surface the
        // distinction or callers cannot tell a silent outage from a miss.
        if all_results.is_empty() && !failures.is_empty() {
            return Err(format!("All sources failed: {}", failures.join("; ")));
        }
        if let Err(error) = crate::utils::HistoryService::new().add_search(query, source_filter) {
            tracing::warn!("Could not record search history: {}", error);
        }
        all_results.retain(|paper| {
            license_policy.allows(paper.license.as_deref())
                && search_query.type_filter_allows(paper)
        });
        // The same paper arrives from several aggregating sources; keep the
        // first occurrence so callers are not flooded with duplicates.
        let all_results =
            crate::utils::deduplicate_papers(all_results, crate::utils::DuplicateStrategy::First);
        serde_json::to_value(all_results).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct SearchByAuthorHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for SearchByAuthorHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let author = args
            .get("author")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'author' parameter")?;
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;
        let year = args.get("year").and_then(|v| v.as_str());
        let source_filter = args.get("source").and_then(|v| v.as_str());

        let futures: Vec<_> = self
            .sources
            .iter()
            .filter(|s| {
                if let Some(filter) = source_filter {
                    if s.id() != filter {
                        return false;
                    }
                }
                s.supports_author_search()
            })
            .map(|source| {
                let source = Arc::clone(source);
                let author = author.to_string();
                async move {
                    match source.search_by_author(&author, max_results, year).await {
                        Ok(response) => Ok(response.papers),
                        Err(e) => {
                            tracing::warn!("Author search failed for {}: {}", source.id(), e);
                            Err(e)
                        }
                    }
                }
            })
            .collect();

        let results = future::join_all(futures).await;
        let mut all_results = Vec::new();
        for papers in results.into_iter().flatten() {
            all_results.extend(papers);
        }
        serde_json::to_value(all_results).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct GetPaperHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for GetPaperHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let source_override = args.get("source").and_then(|v| v.as_str());
        let enrich = args.get("enrich").and_then(|v| v.as_bool()).unwrap_or(true);
        let source = self.find_source(paper_id, source_override)?;

        // Try direct lookup first (more efficient and accurate)
        match source.get_by_id(paper_id).await {
            Ok(paper) => return enrich_paper(paper, enrich).await,
            Err(e) => {
                tracing::debug!(
                    "get_by_id failed for {} on {}: {}",
                    paper_id,
                    source.id(),
                    e
                );
            }
        }

        // Try DOI lookup if the paper_id looks like a DOI
        if paper_id.starts_with("10.") {
            match source.get_by_doi(paper_id).await {
                Ok(paper) => return enrich_paper(paper, enrich).await,
                Err(e) => {
                    tracing::debug!(
                        "get_by_doi failed for {} on {}: {}",
                        paper_id,
                        source.id(),
                        e
                    );
                }
            }
        }

        // Fall back to search
        let search_query = crate::models::SearchQuery::new(paper_id).max_results(1);
        let response = source
            .search(&search_query)
            .await
            .map_err(|e| e.to_string())?;
        if response.papers.is_empty() {
            return Err(format!("Paper '{}' not found in {}", paper_id, source.id()));
        }
        enrich_paper(response.papers[0].clone(), enrich).await
    }
}

/// Format a paper as an in-text citation in any archived CSL style.
#[derive(Debug)]
pub struct CslCitationHandler;

#[async_trait::async_trait]
impl ToolHandler for CslCitationHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let style = args
            .get("style")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'style' parameter")?;
        let paper: crate::models::Paper = args
            .get("paper")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .ok_or(
                "Missing or invalid 'paper' parameter (a paper object from get_paper or search_papers)",
            )?;
        let citation = crate::utils::csl::format_csl_citation(&paper, style)?;
        Ok(serde_json::json!({
            "style": style,
            "citation": citation,
        }))
    }
}

/// Best-effort enrichment of a paper record with free external metadata.
///
/// TLDRs come from Semantic Scholar's keyless graph API; retraction status
/// from OpenAlex single-work lookups (via the evidence module). Both run in
/// parallel with short timeouts; failures are logged and ignored so the base
/// record is never lost to an enrichment outage.
async fn enrich_paper(paper: crate::models::Paper, enrich: bool) -> Result<Value, String> {
    let mut value = serde_json::to_value(&paper).map_err(|e| e.to_string())?;
    let Some(doi) = paper.doi.clone().filter(|d| d.starts_with("10.")) else {
        return Ok(value);
    };
    if !enrich {
        return Ok(value);
    }
    let client = match crate::utils::HttpClient::new() {
        Ok(client) => client,
        Err(e) => return Err(e.to_string()),
    };
    let (tldr, retraction) = tokio::join!(
        fetch_s2_tldr(&client, &doi),
        crate::utils::evidence::check_retraction(&client, &doi),
    );
    if let Some(obj) = value.as_object_mut() {
        match tldr {
            Ok(Some(tldr)) => {
                obj.insert("tldr".to_string(), Value::String(tldr));
            }
            Ok(None) => {}
            Err(e) => tracing::debug!("TLDR enrichment failed for {doi}: {e}"),
        }
        match retraction {
            Ok(info) if info.is_retracted => {
                obj.insert("retracted".to_string(), Value::Bool(true));
                if let Some(date) = info.retraction_date {
                    obj.insert("retraction_date".to_string(), Value::String(date));
                }
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("Retraction lookup failed for {doi}: {e}"),
        }
    }
    Ok(value)
}

async fn fetch_s2_tldr(
    client: &crate::utils::HttpClient,
    doi: &str,
) -> Result<Option<String>, String> {
    let url = format!(
        "https://api.semanticscholar.org/graph/v1/paper/DOI:{}?fields=tldr",
        urlencoding::encode(doi)
    );
    let response =
        tokio::time::timeout(std::time::Duration::from_secs(10), client.get(&url).send())
            .await
            .map_err(|e| format!("timeout: {e}"))?
            .map_err(|e| format!("request failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("status {}", response.status()));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("bad JSON: {e}"))?;
    Ok(body
        .get("tldr")
        .and_then(|t| t.get("text"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .filter(|t| !t.is_empty()))
}

impl GetPaperHandler {
    fn find_source(
        &self,
        paper_id: &str,
        source_override: Option<&str>,
    ) -> Result<Arc<dyn crate::sources::Source>, String> {
        if let Some(source_id) = source_override {
            return self
                .sources
                .iter()
                .find(|s| s.id() == source_id)
                .cloned()
                .ok_or_else(|| format!("Source '{}' not found", source_id));
        }
        auto_detect_source(&self.sources, paper_id)
    }
}

#[derive(Debug)]
pub struct DownloadPaperHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for DownloadPaperHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let source_override = args.get("source").and_then(|v| v.as_str());
        let output_path = args
            .get("output_path")
            .and_then(|v| v.as_str())
            .unwrap_or("./downloads");
        let source = self.find_source(paper_id, source_override)?;
        let request = crate::models::DownloadRequest::new(paper_id, output_path);

        if let Ok(result) = crate::utils::with_retry(crate::utils::api_retry_config(), || {
            let source = Arc::clone(&source);
            let request = request.clone();
            async move { source.download(&request).await }
        })
        .await
        {
            return serde_json::to_value(result).map_err(|e| e.to_string());
        }
        let fallback_order = ["openaire", "core", "europe_pmc", "pmc", "unpaywall"];
        for fallback_id in &fallback_order {
            if source.id() == *fallback_id {
                continue;
            }
            if let Some(fallback_source) = self
                .sources
                .iter()
                .find(|s| s.id() == *fallback_id && s.supports_download())
            {
                if let Ok(result) =
                    crate::utils::with_retry(crate::utils::api_retry_config(), || {
                        let fb_source = Arc::clone(fallback_source);
                        let fb_request = request.clone();
                        async move { fb_source.download(&fb_request).await }
                    })
                    .await
                {
                    return serde_json::to_value(result).map_err(|e| e.to_string());
                }
            }
        }

        // Every source-level download failed; resolve through Unpaywall's
        // ranked best_oa_location as the final fallback.
        match unpaywall_pdf_fallback(&self.sources, paper_id, None, output_path).await {
            Ok(result) => serde_json::to_value(result).map_err(|e| e.to_string()),
            Err(unpaywall_error) => Err(format!(
                "Failed to download '{}' from any source (tried {} and OA fallbacks: {}). \
                 OA resolution: {}",
                paper_id,
                source.id(),
                fallback_order.join(", "),
                unpaywall_error
            )),
        }
    }
}

/// Resolve an OA PDF through Unpaywall and download it directly.
///
/// Last resort after every source's own download failed: Unpaywall ranks
/// `best_oa_location` per DOI, which the source adapters surface as pdf_url.
async fn unpaywall_pdf_fallback(
    sources: &Arc<Vec<Arc<dyn crate::sources::Source>>>,
    paper_id: &str,
    doi_hint: Option<&str>,
    output_path: &str,
) -> Result<crate::models::DownloadResult, String> {
    let unpaywall = sources
        .iter()
        .find(|s| s.id() == "unpaywall")
        .ok_or("Unpaywall source not available")?;

    let candidates = [doi_hint.map(str::to_string), Some(paper_id.to_string())];
    let mut paper = None;
    for candidate in candidates.iter().flatten() {
        if !candidate.starts_with("10.") {
            continue;
        }
        if let Ok(found) = unpaywall.get_by_doi(candidate).await {
            paper = Some(found);
            break;
        }
    }
    let paper = paper.ok_or("Unpaywall has no open-access record for this DOI")?;
    let pdf_url = paper
        .pdf_url
        .ok_or("Unpaywall found an OA record but no direct PDF URL")?;

    let file_name = match crate::utils::sanitize_filename(&paper.title) {
        Ok(name) => format!("{name}.pdf"),
        Err(_) => format!(
            "paper-{}.pdf",
            crate::utils::sanitize_paper_id(paper_id).unwrap_or_else(|_| "unknown".to_string())
        ),
    };
    let dir = std::path::Path::new(output_path);
    let path = dir.join(&file_name);
    let path_str = path.to_string_lossy().to_string();

    let client = crate::utils::HttpClient::new().map_err(|e| e.to_string())?;
    // Only http/https: the URL comes from third-party OA records, and the
    // reqwest default would reject file:// etc., but say so explicitly.
    if !pdf_url.starts_with("http://") && !pdf_url.starts_with("https://") {
        return Err(format!(
            "OA record points at an unsupported URL scheme: {pdf_url}"
        ));
    }
    let response = client
        .get(&pdf_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OA PDF: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "OA PDF fetch returned status {}",
            response.status()
        ));
    }
    // Content-length first, then a streaming cap for servers that omit it:
    // an unbounded body could fill the disk.
    const MAX_PDF_BYTES: u64 = 200 * 1024 * 1024;
    if let Some(len) = response.content_length() {
        if len > MAX_PDF_BYTES {
            return Err(format!(
                "OA PDF too large ({len} bytes, limit {MAX_PDF_BYTES})"
            ));
        }
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read OA PDF body: {e}"))?;
    if bytes.len() as u64 > MAX_PDF_BYTES {
        return Err(format!(
            "OA PDF too large ({} bytes, limit {MAX_PDF_BYTES})",
            bytes.len()
        ));
    }
    // A misconfigured OA endpoint can serve an HTML error page; saving that
    // as .pdf produces a corrupt file.
    if bytes.starts_with(b"<!DOCTYPE") || bytes.starts_with(b"<html") {
        return Err("OA endpoint returned HTML instead of a PDF".to_string());
    }
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("Failed to create output dir: {e}"))?;
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| format!("Failed to write PDF: {e}"))?;
    Ok(crate::models::DownloadResult::success(
        path_str,
        bytes.len() as u64,
    ))
}

impl DownloadPaperHandler {
    fn find_source(
        &self,
        paper_id: &str,
        source_override: Option<&str>,
    ) -> Result<Arc<dyn crate::sources::Source>, String> {
        if let Some(source_id) = source_override {
            return self
                .sources
                .iter()
                .find(|s| s.id() == source_id)
                .cloned()
                .ok_or_else(|| format!("Source '{}' not found", source_id));
        }
        auto_detect_source(&self.sources, paper_id)
    }
}

#[derive(Debug)]
pub struct ReadPaperHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for ReadPaperHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let source_override = args.get("source").and_then(|v| v.as_str());
        // Bounded by default: full PDF text dumps blow up LLM contexts, the
        // most common complaint class across competitor servers.
        let max_chars = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(12000) as usize;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let return_full_text = args
            .get("return_full_text")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let source = self.find_source(paper_id, source_override)?;
        let request = crate::models::ReadRequest::new(paper_id, "./downloads");
        let result = source.read(&request).await.map_err(|e| e.to_string())?;
        let total_chars = result.text.chars().count();
        let (text, offset, truncated, next_start) = if return_full_text {
            (result.text.clone(), 0usize, false, None)
        } else {
            let start = offset.min(total_chars);
            let slice: String = result.text.chars().skip(start).take(max_chars).collect();
            let truncated = start + max_chars < total_chars;
            let next_start = truncated.then(|| start + max_chars);
            (slice, start, truncated, next_start)
        };
        Ok(serde_json::json!({
            "paper_id": paper_id,
            "source": source.id(),
            "success": result.success,
            "error": result.error,
            "total_chars": total_chars,
            "offset": offset,
            "truncated": truncated,
            "next_start": next_start,
            "text": text,
        }))
    }
}

impl ReadPaperHandler {
    fn find_source(
        &self,
        paper_id: &str,
        source_override: Option<&str>,
    ) -> Result<Arc<dyn crate::sources::Source>, String> {
        if let Some(source_id) = source_override {
            return self
                .sources
                .iter()
                .find(|s| s.id() == source_id)
                .cloned()
                .ok_or_else(|| format!("Source '{}' not found", source_id));
        }
        auto_detect_source(&self.sources, paper_id)
    }
}

#[derive(Debug)]
pub struct GetCitationsHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for GetCitationsHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let source_override = args.get("source").and_then(|v| v.as_str());
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        let source_id = source_override.unwrap_or_else(|| {
            self.sources
                .iter()
                .find(|s| s.supports_citations())
                .map(|s| s.id())
                .unwrap_or("semantic")
        });

        let source = self
            .sources
            .iter()
            .find(|s| s.id() == source_id)
            .ok_or_else(|| format!("Source '{}' not found", source_id))?;

        if !source.supports_citations() {
            return Err(format!("Source '{}' does not support citations", source_id));
        }

        let request = crate::models::CitationRequest::new(paper_id).max_results(max_results);
        let response = source
            .get_citations(&request)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_value(response).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct GetReferencesHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for GetReferencesHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let source_override = args.get("source").and_then(|v| v.as_str());
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        let source_id = source_override.unwrap_or_else(|| {
            self.sources
                .iter()
                .find(|s| s.supports_citations())
                .map(|s| s.id())
                .unwrap_or("semantic")
        });

        let source = self
            .sources
            .iter()
            .find(|s| s.id() == source_id)
            .ok_or_else(|| format!("Source '{}' not found", source_id))?;

        if !source.supports_citations() {
            return Err(format!(
                "Source '{}' does not support references",
                source_id
            ));
        }

        let request = crate::models::CitationRequest::new(paper_id).max_results(max_results);
        let response = source
            .get_references(&request)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_value(response).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct LookupByDoiHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for LookupByDoiHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let doi = args
            .get("doi")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'doi' parameter")?;
        let source_filter = args.get("source").and_then(|v| v.as_str());

        for source in self.sources.iter() {
            if let Some(filter) = source_filter {
                if source.id() != filter {
                    continue;
                }
            }
            if !source.supports_doi_lookup() {
                continue;
            }
            match source.get_by_doi(doi).await {
                Ok(paper) => return serde_json::to_value(paper).map_err(|e| e.to_string()),
                Err(e) => tracing::debug!("DOI lookup failed for {}: {}", source.id(), e),
            }
        }
        Err(format!("Paper with DOI '{}' not found", doi))
    }
}

#[derive(Debug)]
pub struct DeduplicatePapersHandler;

#[async_trait::async_trait]
impl ToolHandler for DeduplicatePapersHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let papers: Vec<crate::models::Paper> = serde_json::from_value(
            args.get("papers")
                .ok_or("Missing 'papers' parameter")?
                .clone(),
        )
        .map_err(|e| format!("Invalid papers array: {}", e))?;

        let strategy_str = args
            .get("strategy")
            .and_then(|v| v.as_str())
            .unwrap_or("first");

        let strategy = match strategy_str {
            "last" => crate::utils::DuplicateStrategy::Last,
            "mark" => crate::utils::DuplicateStrategy::Mark,
            _ => crate::utils::DuplicateStrategy::First,
        };

        let deduped = crate::utils::deduplicate_papers(papers, strategy);
        serde_json::to_value(deduped).map_err(|e| e.to_string())
    }
}

// ============================================================================
// NEW TOOL HANDLERS
// ============================================================================

#[derive(Debug)]
pub struct GetRelatedPapersHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for GetRelatedPapersHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let source_override = args.get("source").and_then(|v| v.as_str());
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        let source_id: String = if let Some(override_id) = source_override {
            override_id.to_string()
        } else {
            "semantic".to_string()
        };

        let source = self
            .sources
            .iter()
            .find(|s| s.id() == source_id.as_str())
            .ok_or_else(|| format!("Source '{}' not found", source_id))?;

        let request = crate::models::CitationRequest::new(paper_id).max_results(max_results);
        let response = source
            .get_related(&request)
            .await
            .map_err(|e| e.to_string())?;
        serde_json::to_value(response).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct ListSourcesHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for ListSourcesHandler {
    async fn execute(&self, _args: Value) -> Result<Value, String> {
        #[derive(serde::Serialize)]
        struct SourceInfo {
            id: String,
            name: String,
            search: bool,
            download: bool,
            read: bool,
            citations: bool,
            doi_lookup: bool,
            author_search: bool,
        }

        let mut source_list: Vec<SourceInfo> = self
            .sources
            .iter()
            .map(|s| SourceInfo {
                id: s.id().to_string(),
                name: s.name().to_string(),
                search: s.supports_search(),
                download: s.supports_download(),
                read: s.supports_read(),
                citations: s.supports_citations(),
                doi_lookup: s.supports_doi_lookup(),
                author_search: s.supports_author_search(),
            })
            .collect();

        source_list.sort_by(|a, b| a.id.cmp(&b.id));
        serde_json::to_value(source_list).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct AuthorProfileHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for AuthorProfileHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let author = args
            .get("author")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'author' parameter")?;
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        let futures: Vec<futures_util::future::BoxFuture<'static, Result<Value, String>>> = self
            .sources
            .iter()
            .filter(|s| s.supports_author_search())
            .map(|source| {
                let source = Arc::clone(source);
                let author = author.to_string();
                let future = async move {
                    let source_id = source.id().to_string();
                    match source.search_by_author(&author, max_results, None).await {
                        Ok(response) => {
                            let total_papers = response.papers.len();
                            let papers_with_citations = response
                                .papers
                                .iter()
                                .filter(|p| p.citations.unwrap_or(0) > 10)
                                .count();
                            let most_cited = response
                                .papers
                                .iter()
                                .max_by_key(|p| p.citations.unwrap_or(0))
                                .map(|p| (p.title.clone(), p.citations.unwrap_or(0)));
                            let year_counts: Vec<_> = response
                                .papers
                                .iter()
                                .filter_map(|p| {
                                    p.published_date
                                        .as_ref()
                                        .and_then(|d| d.split('-').next())
                                        .map(|y| y.to_string())
                                })
                                .fold(
                                    std::collections::HashMap::<String, usize>::new(),
                                    |mut acc, y| {
                                        *acc.entry(y).or_insert(0) += 1;
                                        acc
                                    },
                                )
                                .into_iter()
                                .map(|(y, c)| serde_json::json!({"year": y, "count": c}))
                                .collect::<Vec<_>>();
                            Ok(serde_json::json!({
                                "source": source_id,
                                "total_papers_found": total_papers,
                                "papers_with_significant_citations": papers_with_citations,
                                "most_cited_paper": most_cited.map(|(t, c)|
                                    serde_json::json!({"title": t, "citations": c})),
                                "publications_by_year": year_counts,
                                "recent_papers": response.papers.into_iter().take(5).collect::<Vec<_>>()
                            }))
                        }
                        Err(e) => {
                            tracing::warn!("Author search failed for {}: {}", source_id, e);
                            Ok(serde_json::json!({
                                "source": source_id,
                                "error": e.to_string()
                            }))
                        }
                    }
                };
                Box::pin(future) as futures_util::future::BoxFuture<'static, Result<Value, String>>
            })
            .collect();

        let results: Vec<Value> = future::join_all(futures)
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect();

        if results.is_empty() {
            return Err(format!("Author '{}' not found in any source", author));
        }

        let profile = serde_json::json!({
            "author": author,
            "sources": results,
        });

        serde_json::to_value(profile).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct BatchGetPapersHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for BatchGetPapersHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_ids = args
            .get("paper_ids")
            .and_then(|v| v.as_array())
            .ok_or("Missing 'paper_ids' parameter (array of strings)")?;
        let max_per_source = args
            .get("max_per_source")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;

        let ids: Vec<&str> = paper_ids.iter().filter_map(|v| v.as_str()).collect();
        if ids.is_empty() {
            return Ok(serde_json::json!({
                "total_queried": 0,
                "total_found": 0,
                "total_not_found": 0,
                "results": [],
            }));
        }

        let futures: Vec<_> = ids
            .into_iter()
            .map(|paper_id| {
                let sources = Arc::clone(&self.sources);
                async move {
                    match auto_detect_source(&sources, paper_id) {
                        Ok(source) => {
                            let search_query = crate::models::SearchQuery::new(paper_id)
                                .max_results(max_per_source);
                            match source.search(&search_query).await {
                                Ok(response) => {
                                    let papers: Vec<crate::models::Paper> = response.papers;
                                    serde_json::json!({
                                        "query_id": paper_id,
                                        "source": source.id(),
                                        "found": !papers.is_empty(),
                                        "papers": papers,
                                    })
                                }
                                Err(e) => serde_json::json!({
                                    "query_id": paper_id,
                                    "source": source.id(),
                                    "found": false,
                                    "error": e.to_string(),
                                    "papers": [],
                                }),
                            }
                        }
                        Err(e) => serde_json::json!({
                            "query_id": paper_id,
                            "source": null,
                            "found": false,
                            "error": e,
                            "papers": [],
                        }),
                    }
                }
            })
            .collect();

        let results: Vec<Value> = future::join_all(futures).await;
        let found: Vec<&Value> = results
            .iter()
            .filter(|r| r["found"].as_bool().unwrap_or(false))
            .collect();
        let not_found: Vec<&Value> = results
            .iter()
            .filter(|r| !r["found"].as_bool().unwrap_or(true))
            .collect();

        let output = serde_json::json!({
            "total_queried": results.len(),
            "total_found": found.len(),
            "total_not_found": not_found.len(),
            "results": results,
        });

        serde_json::to_value(output).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct CitationGraphHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for CitationGraphHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let source_override = args.get("source").and_then(|v| v.as_str());
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        let source_id = source_override.unwrap_or_else(|| {
            self.sources
                .iter()
                .find(|s| s.supports_citations())
                .map(|s| s.id())
                .unwrap_or("semantic")
        });

        let source = self
            .sources
            .iter()
            .find(|s| s.id() == source_id)
            .cloned()
            .ok_or_else(|| format!("Source '{}' not found", source_id))?;

        if !source.supports_citations() {
            return Err(format!("Source '{}' does not support citations", source_id));
        }

        let request = crate::models::CitationRequest::new(paper_id).max_results(max_results);

        let (citations, references) = futures_util::future::join(
            async {
                source
                    .get_citations(&request)
                    .await
                    .map(|r| r.papers)
                    .unwrap_or_default()
            },
            async {
                source
                    .get_references(&request)
                    .await
                    .map(|r| r.papers)
                    .unwrap_or_default()
            },
        )
        .await;

        let output = serde_json::json!({
            "paper_id": paper_id,
            "source": source.id(),
            "citation_count": citations.len(),
            "reference_count": references.len(),
            "citations": citations,
            "references": references,
        });

        serde_json::to_value(output).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct ExportPapersHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for ExportPapersHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let papers: Vec<crate::models::Paper> = args
            .get("papers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .ok_or("Missing or invalid 'papers' parameter")?;

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("bibtex")
            .to_lowercase();

        let entries: Vec<String> = papers
            .iter()
            .map(|p| match format.as_str() {
                "bibtex" => crate::utils::format_citation(p, crate::utils::CitationStyle::Bibtex),
                "apa" => crate::utils::format_citation(p, crate::utils::CitationStyle::Apa),
                "mla" => crate::utils::format_citation(p, crate::utils::CitationStyle::Mla),
                "chicago" => crate::utils::format_citation(p, crate::utils::CitationStyle::Chicago),
                "csv" => format!(
                    "\"{}\",\"{}\",\"{}\",{:?},{:?}",
                    p.paper_id,
                    p.title.replace('"', "\"\""),
                    p.authors.replace('"', "\"\""),
                    p.doi,
                    p.published_date
                ),
                "json" => serde_json::to_string(p).unwrap_or_default(),
                _ => crate::utils::format_citation(p, crate::utils::CitationStyle::Bibtex),
            })
            .collect();

        let result = serde_json::json!({
            "format": format,
            "count": entries.len(),
            "entries": entries,
            "joined": entries.join("\n\n"),
        });

        serde_json::to_value(result).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct RecommendPapersHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for RecommendPapersHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'paper_id' parameter")?;
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        // Try Semantic Scholar first (has dedicated related/recommendation API)
        if let Some(source) = self.sources.iter().find(|s| s.id() == "semantic") {
            let request = crate::models::CitationRequest::new(paper_id).max_results(max_results);
            match source.get_related(&request).await {
                Ok(response) => {
                    return serde_json::to_value(serde_json::json!({
                        "paper_id": paper_id,
                        "source": "semantic",
                        "recommendations": response.papers,
                    }))
                    .map_err(|e| e.to_string());
                }
                Err(e) => tracing::debug!("Semantic Scholar recommendations failed: {}", e),
            }
        }

        // Fallback: try Connected Papers
        if let Some(source) = self.sources.iter().find(|s| s.id() == "connected_papers") {
            let request = crate::models::CitationRequest::new(paper_id).max_results(max_results);
            match source.get_related(&request).await {
                Ok(response) => {
                    return serde_json::to_value(serde_json::json!({
                        "paper_id": paper_id,
                        "source": "connected_papers",
                        "recommendations": response.papers,
                    }))
                    .map_err(|e| e.to_string());
                }
                Err(e) => tracing::debug!("Connected Papers recommendations failed: {}", e),
            }
        }

        // Final fallback: try OpenAlex
        if let Some(source) = self.sources.iter().find(|s| s.id() == "openalex") {
            let request = crate::models::CitationRequest::new(paper_id).max_results(max_results);
            match source.get_related(&request).await {
                Ok(response) => {
                    return serde_json::to_value(serde_json::json!({
                        "paper_id": paper_id,
                        "source": "openalex",
                        "recommendations": response.papers,
                    }))
                    .map_err(|e| e.to_string());
                }
                Err(e) => tracing::debug!("OpenAlex recommendations failed: {}", e),
            }
        }

        Err(format!(
            "Could not find recommendations for '{}' from any source",
            paper_id
        ))
    }
}

/// Handler for saving paper metadata to a local library
#[derive(Debug)]
pub struct WritePaperHandler;

#[async_trait::async_trait]
impl ToolHandler for WritePaperHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let papers: Vec<crate::models::Paper> = args
            .get("papers")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .ok_or("Missing or invalid 'papers' parameter")?;

        let notes = args.get("notes").and_then(|v| v.as_str()).unwrap_or("");
        let library_path = get_library_path()?;

        // Read existing library
        let mut library: Vec<serde_json::Value> = if library_path.exists() {
            let content = std::fs::read_to_string(&library_path)
                .map_err(|e| format!("Failed to read library: {}", e))?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Vec::new()
        };

        // Add each paper with timestamp
        for paper in papers {
            let entry = serde_json::json!({
                "paper": paper,
                "notes": notes,
                "saved_at": chrono::Utc::now().to_rfc3339(),
            });
            library.push(entry);
        }

        // Write library
        if let Some(parent) = library_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create library directory: {}", e))?;
        }
        let json = serde_json::to_string_pretty(&library)
            .map_err(|e| format!("Failed to serialize library: {}", e))?;
        std::fs::write(&library_path, json)
            .map_err(|e| format!("Failed to write library: {}", e))?;

        let result = serde_json::json!({
            "saved": library.len(),
            "library_path": library_path.to_string_lossy(),
        });

        serde_json::to_value(result).map_err(|e| e.to_string())
    }
}

/// Helper to get the library file path
fn get_library_path() -> Result<std::path::PathBuf, String> {
    let config_dir =
        dirs::data_dir().ok_or_else(|| "Could not determine data directory".to_string())?;
    Ok(config_dir.join("research-master").join("library.json"))
}

/// Handler for general web search (fallback when paper sources are insufficient)
///
/// Providers are tried in order using the first configured key:
/// Exa (`EXA_API_KEY`), Tavily (`TAVILY_API_KEY`), Brave (`BRAVE_API_KEY`).
/// If a configured provider fails at request time, the next one is tried.
#[derive(Debug)]
pub struct WebSearchHandler;

const EXA_SEARCH_URL: &str = "https://api.exa.ai/search";
const TAVILY_SEARCH_URL: &str = "https://api.tavily.com/search";
const BRAVE_SEARCH_URL: &str = "https://api.search.brave.com/res/v1/web/search";

#[async_trait::async_trait]
impl ToolHandler for WebSearchHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'query' parameter")?;

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        let mut failures: Vec<String> = Vec::new();

        if let Some(api_key) = env_key("EXA_API_KEY") {
            match search_exa(query, max_results, &api_key, EXA_SEARCH_URL).await {
                Ok(v) => return Ok(v),
                Err(e) => failures.push(format!("Exa: {e}")),
            }
        }
        if let Some(api_key) = env_key("TAVILY_API_KEY") {
            match search_tavily(query, max_results, &api_key, TAVILY_SEARCH_URL).await {
                Ok(v) => return Ok(v),
                Err(e) => failures.push(format!("Tavily: {e}")),
            }
        }
        if let Some(api_key) = env_key("BRAVE_API_KEY") {
            match search_brave(query, max_results, &api_key, BRAVE_SEARCH_URL).await {
                Ok(v) => return Ok(v),
                Err(e) => failures.push(format!("Brave: {e}")),
            }
        }

        if failures.is_empty() {
            return Err(missing_web_search_keys_error());
        }
        Err(format!(
            "All configured web search providers failed: {}",
            failures.join("; ")
        ))
    }
}

fn env_key(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|k| !k.trim().is_empty())
}

fn missing_web_search_keys_error() -> String {
    "Web search requires one of: EXA_API_KEY (https://dashboard.exa.ai/api-keys), \
     TAVILY_API_KEY (https://tavily.com), or BRAVE_API_KEY (https://brave.com/search/api/)"
        .to_string()
}

fn web_search_client() -> Result<reqwest::Client, String> {
    // Env-only proxy config; used by web_search and github_search alike.
    let proxy = crate::utils::create_proxy_config_from_cli(None, None, None);
    proxy
        .apply_to(reqwest::Client::builder())
        .and_then(|b| b.timeout(std::time::Duration::from_secs(15)).build())
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

/// Search using the Exa API
async fn search_exa(
    query: &str,
    max_results: usize,
    api_key: &str,
    base_url: &str,
) -> Result<Value, String> {
    let client = web_search_client()?;

    let response = client
        .post(base_url)
        .header("x-api-key", api_key)
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "query": query,
            "numResults": max_results.min(10),
            "type": "auto",
            "useAutoprompt": true,
        }))
        .send()
        .await
        .map_err(|e| format!("Exa search request failed: {}", e))?;

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Exa response: {}", e))?;

    // Extract results into standardized format
    let results = body["results"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|r| serde_json::json!({
            "title": r["title"].as_str().unwrap_or(""),
            "url": r["url"].as_str().unwrap_or(""),
            "snippet": r.get("text").or_else(|| r.get("snippet")).and_then(|v| v.as_str()).unwrap_or(""),
            "source": "exa",
        }))
        .collect::<Vec<_>>();

    web_search_output(query, "exa", results)
}

/// Search using the Tavily API
async fn search_tavily(
    query: &str,
    max_results: usize,
    api_key: &str,
    base_url: &str,
) -> Result<Value, String> {
    let client = web_search_client()?;

    let response = client
        .post(base_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "query": query,
            "max_results": max_results.min(10),
        }))
        .send()
        .await
        .map_err(|e| format!("Tavily search request failed: {}", e))?;

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Tavily response: {}", e))?;

    let results = body["results"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "title": r["title"].as_str().unwrap_or(""),
                "url": r["url"].as_str().unwrap_or(""),
                "snippet": r.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                "source": "tavily",
            })
        })
        .collect::<Vec<_>>();

    web_search_output(query, "tavily", results)
}

/// Search using the Brave Search API
async fn search_brave(
    query: &str,
    max_results: usize,
    api_key: &str,
    base_url: &str,
) -> Result<Value, String> {
    let client = web_search_client()?;
    let count = max_results.min(10).to_string();

    let response = client
        .get(base_url)
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .query(&[("q", query), ("count", count.as_str())])
        .send()
        .await
        .map_err(|e| format!("Brave search request failed: {}", e))?;

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Brave response: {}", e))?;

    let results = body["web"]["results"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "title": r["title"].as_str().unwrap_or(""),
                "url": r["url"].as_str().unwrap_or(""),
                "snippet": r.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                "source": "brave",
            })
        })
        .collect::<Vec<_>>();

    web_search_output(query, "brave", results)
}

fn web_search_output(query: &str, provider: &str, results: Vec<Value>) -> Result<Value, String> {
    let output = serde_json::json!({
        "query": query,
        "source": provider,
        "total_results": results.len(),
        "results": results,
    });
    serde_json::to_value(output).map_err(|e| e.to_string())
}

/// Handler for extracting readable text from a web page or PDF URL
///
/// Plain HTML is reduced to title plus block text (headings, paragraphs,
/// lists, quotes, code blocks) with scripts and styles excluded. PDFs are
/// downloaded and run through the same extraction as `read_paper`.
#[derive(Debug)]
pub struct WebExtractHandler;

const WEB_EXTRACT_MAX_BYTES: usize = 2 * 1024 * 1024;
const WEB_EXTRACT_PDF_MAX_BYTES: usize = 20 * 1024 * 1024;
const WEB_EXTRACT_MAX_CHARS_CAP: usize = 100_000;

#[async_trait::async_trait]
impl ToolHandler for WebExtractHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'url' parameter")?;

        let max_chars = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|v| v.min(WEB_EXTRACT_MAX_CHARS_CAP as u64) as usize)
            .unwrap_or(20_000);

        let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL '{url}': {e}"))?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(format!(
                "Only http(s) URLs are supported, got '{}'",
                parsed.scheme()
            ));
        }

        let proxy = crate::utils::create_proxy_config_from_cli(None, None, None);
        let client = proxy
            .apply_to(
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(20))
                    .user_agent(format!("research-master/{}", env!("CARGO_PKG_VERSION"))),
            )
            .and_then(|b| b.build())
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        let response = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch {url}: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("Fetch failed: HTTP {status} for {url}"));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        let is_pdf = content_type.contains("application/pdf")
            || parsed.path().to_ascii_lowercase().ends_with(".pdf");
        let byte_cap = if is_pdf {
            WEB_EXTRACT_PDF_MAX_BYTES
        } else {
            WEB_EXTRACT_MAX_BYTES
        };
        if let Some(len) = response.content_length() {
            if len as usize > byte_cap {
                return Err(format!(
                    "Response too large: {} bytes (limit {byte_cap}) for {url}",
                    len
                ));
            }
        }

        let (title, text, kind) = if is_pdf {
            let bytes = response
                .bytes()
                .await
                .map_err(|e| format!("Failed to read PDF body: {e}"))?;
            if bytes.len() > byte_cap {
                return Err(format!(
                    "PDF too large: {} bytes (limit {byte_cap}) for {url}",
                    bytes.len()
                ));
            }
            let mut tmp = tempfile::Builder::new()
                .suffix(".pdf")
                .tempfile()
                .map_err(|e| format!("Failed to create temp file: {e}"))?;
            std::io::Write::write_all(&mut tmp, &bytes)
                .map_err(|e| format!("Failed to write temp file: {e}"))?;
            let text = crate::utils::extract_text_simple(tmp.path())
                .map_err(|e| format!("PDF extraction failed: {e}"))?;
            (String::new(), text, "application/pdf".to_string())
        } else {
            let body = response
                .text()
                .await
                .map_err(|e| format!("Failed to read response body: {e}"))?;
            if content_type.contains("html") || body.trim_start().starts_with('<') {
                let (title, text) = extract_html_text(&body);
                (title, text, "text/html".to_string())
            } else {
                (String::new(), body, content_type)
            }
        };

        let total_chars = text.chars().count();
        let (content, truncated) = if total_chars > max_chars {
            (text.chars().take(max_chars).collect::<String>(), true)
        } else {
            (text, false)
        };

        let output = serde_json::json!({
            "url": url,
            "title": title,
            "content_type": kind,
            "total_chars": total_chars,
            "returned_chars": content.chars().count(),
            "truncated": truncated,
            "text": content,
        });
        serde_json::to_value(output).map_err(|e| e.to_string())
    }
}

/// Reduce an HTML document to its title plus block-level text, in document
/// order. Scripts, styles and other non-content elements never match the
/// block tag set, so their text is dropped naturally.
fn extract_html_text(html: &str) -> (String, String) {
    use scraper::{Html, Selector};

    const BLOCK_TAGS: [&str; 12] = [
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "p",
        "li",
        "blockquote",
        "pre",
        "figcaption",
        "td",
    ];

    fn collapse(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    let doc = Html::parse_document(html);

    let title = Selector::parse("title")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| collapse(&el.text().collect::<String>()))
        .unwrap_or_default();

    let container = ["article", "main", "body"]
        .iter()
        .find_map(|tag| {
            Selector::parse(tag)
                .ok()
                .and_then(|sel| doc.select(&sel).next())
        })
        .unwrap_or_else(|| doc.root_element());

    let mut blocks: Vec<String> = Vec::new();
    for node in container.descendants() {
        let Some(el) = scraper::ElementRef::wrap(node) else {
            continue;
        };
        if !BLOCK_TAGS.contains(&el.value().name()) {
            continue;
        }
        // Skip nested blocks (a p inside an li, a pre inside a blockquote):
        // the outer block's text already covers them.
        let mut ancestor = node.parent();
        let mut nested = false;
        while let Some(a) = ancestor {
            if let Some(ae) = a.value().as_element() {
                if BLOCK_TAGS.contains(&ae.name()) {
                    nested = true;
                    break;
                }
            }
            ancestor = a.parent();
        }
        if nested {
            continue;
        }
        let text = collapse(&el.text().collect::<String>());
        if !text.is_empty() {
            blocks.push(text);
        }
    }

    if blocks.is_empty() {
        // No recognized blocks: fall back to all visible-ish text.
        let text = collapse(&container.text().collect::<String>());
        return (title, text);
    }

    (title, blocks.join("\n\n"))
}

/// Handler for searching GitHub repositories (code accompanying papers,
/// datasets, tools)
#[derive(Debug)]
pub struct GithubSearchHandler {
    /// Override for tests (mockito); None uses the public GitHub API.
    pub api_base: Option<String>,
    /// Override for tests; None reads GITHUB_API_KEY from the environment.
    pub api_key: Option<String>,
}

const GITHUB_SEARCH_URL: &str = "https://api.github.com/search/repositories";

fn github_request(
    client: &reqwest::Client,
    base_url: &str,
    query: &str,
    per_page: &str,
    api_key: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut request = client
        .get(base_url)
        .query(&[("q", query), ("per_page", per_page)])
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header(
            "User-Agent",
            format!("research-master/{}", env!("CARGO_PKG_VERSION")),
        );
    if let Some(api_key) = api_key {
        request = request.header("Authorization", format!("Bearer {api_key}"));
    }
    request
}

#[async_trait::async_trait]
impl ToolHandler for GithubSearchHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'query' parameter")?;

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .clamp(1, 20) as usize;

        let base_url = self.api_base.as_deref().unwrap_or(GITHUB_SEARCH_URL);

        let client = web_search_client()?;
        let per_page = max_results.to_string();
        let api_key = self.api_key.clone().or_else(|| env_key("GITHUB_API_KEY"));

        let mut response = github_request(&client, base_url, query, &per_page, api_key.as_deref())
            .send()
            .await
            .map_err(|e| format!("GitHub search request failed: {}", e))?;

        let mut status = response.status();
        // A revoked or expired key must not take the tool down: retry
        // anonymously (10 searches/minute still works without a key).
        if status.as_u16() == 401 && api_key.is_some() {
            tracing::warn!("GITHUB_API_KEY rejected with HTTP 401; retrying anonymously");
            response = github_request(&client, base_url, query, &per_page, None)
                .send()
                .await
                .map_err(|e| format!("GitHub search request failed: {}", e))?;
            status = response.status();
            if status.as_u16() == 401 {
                return Err(
                    "GitHub rejected GITHUB_API_KEY with HTTP 401 and the anonymous retry also \
                     failed; refresh or unset the key"
                        .to_string(),
                );
            }
        }

        if status.as_u16() == 403 || status.as_u16() == 429 {
            return Err(format!(
                "GitHub rate limit hit (HTTP {status}). Set GITHUB_API_KEY for 30 searches/minute: \
                 https://github.com/settings/tokens"
            ));
        }
        if !status.is_success() {
            return Err(format!("GitHub search failed: HTTP {status}"));
        }

        let body: Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse GitHub response: {}", e))?;

        let results = body["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "full_name": r["full_name"].as_str().unwrap_or(""),
                    "url": r["html_url"].as_str().unwrap_or(""),
                    "description": r.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                    "stars": r["stargazers_count"].as_u64().unwrap_or(0),
                    "language": r.get("language").and_then(|v| v.as_str()).unwrap_or(""),
                    "updated_at": r.get("updated_at").and_then(|v| v.as_str()).unwrap_or(""),
                    "topics": r.get("topics").cloned().unwrap_or(serde_json::json!([])),
                })
            })
            .collect::<Vec<_>>();

        let output = serde_json::json!({
            "query": query,
            "total_results": results.len(),
            "results": results,
        });
        serde_json::to_value(output).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CitationRequest, DownloadRequest, Paper, ReadRequest, SourceType};
    use crate::sources::{Source, SourceCapabilities};
    use serde_json::json;
    use std::sync::Arc;

    #[derive(Debug)]
    struct MockSource {
        id: String,
        capabilities: SourceCapabilities,
    }

    impl MockSource {
        fn new(id: &str, capabilities: SourceCapabilities) -> Self {
            Self {
                id: id.to_string(),
                capabilities,
            }
        }
    }

    #[async_trait::async_trait]
    impl Source for MockSource {
        fn id(&self) -> &str {
            &self.id
        }
        fn name(&self) -> &str {
            &self.id
        }
        fn capabilities(&self) -> SourceCapabilities {
            self.capabilities
        }
        async fn search(
            &self,
            _: &crate::models::SearchQuery,
        ) -> Result<crate::models::SearchResponse, crate::sources::SourceError> {
            unimplemented!()
        }
        async fn download(
            &self,
            _: &DownloadRequest,
        ) -> Result<crate::models::DownloadResult, crate::sources::SourceError> {
            unimplemented!()
        }
        async fn read(
            &self,
            _: &ReadRequest,
        ) -> Result<crate::models::ReadResult, crate::sources::SourceError> {
            unimplemented!()
        }
        async fn get_citations(
            &self,
            _: &CitationRequest,
        ) -> Result<crate::models::SearchResponse, crate::sources::SourceError> {
            unimplemented!()
        }
        async fn get_references(
            &self,
            _: &CitationRequest,
        ) -> Result<crate::models::SearchResponse, crate::sources::SourceError> {
            unimplemented!()
        }
        fn supports_doi_lookup(&self) -> bool {
            self.capabilities.contains(SourceCapabilities::DOI_LOOKUP)
        }
        async fn get_by_doi(
            &self,
            _: &str,
        ) -> Result<crate::models::Paper, crate::sources::SourceError> {
            unimplemented!()
        }
        async fn get_related(
            &self,
            _: &CitationRequest,
        ) -> Result<crate::models::SearchResponse, crate::sources::SourceError> {
            unimplemented!()
        }
        fn validate_id(&self, _: &str) -> Result<(), crate::sources::SourceError> {
            Ok(())
        }
    }

    fn make_test_sources() -> Vec<Arc<dyn Source>> {
        vec![
            Arc::new(MockSource::new("arxiv", SourceCapabilities::all())),
            Arc::new(MockSource::new("semantic", SourceCapabilities::all())),
            Arc::new(MockSource::new("pmc", SourceCapabilities::all())),
            Arc::new(MockSource::new("hal", SourceCapabilities::all())),
            Arc::new(MockSource::new("iacr", SourceCapabilities::all())),
        ]
    }

    #[test]
    fn test_auto_detect_arxiv_numeric() {
        let sources = make_test_sources();
        let result = auto_detect_source(&Arc::new(sources), "2301.12345");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id(), "arxiv");
    }

    #[test]
    fn test_auto_detect_arxiv_prefix() {
        let sources = make_test_sources();
        let result = auto_detect_source(&Arc::new(sources), "arxiv:2301.12345");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id(), "arxiv");
    }

    #[test]
    fn test_auto_detect_pmc() {
        let sources = make_test_sources();
        let result = auto_detect_source(&Arc::new(sources), "PMC12345");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id(), "pmc");
    }

    #[test]
    fn test_auto_detect_hal() {
        let sources = make_test_sources();
        let result = auto_detect_source(&Arc::new(sources), "hal-12345");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id(), "hal");
    }

    #[test]
    fn test_auto_detect_iacr() {
        let sources = make_test_sources();
        let result = auto_detect_source(&Arc::new(sources), "2023/1234");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id(), "iacr");
    }

    #[test]
    fn test_auto_detect_doi_not_iacr() {
        // A DOI has exactly one slash, which used to route it to IACR.
        let sources = make_test_sources();
        let result = auto_detect_source(&Arc::new(sources), "10.1145/3292500.3330701");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id(), "semantic");
    }

    #[test]
    fn test_auto_detect_fallback() {
        let sources = make_test_sources();
        let result = auto_detect_source(&Arc::new(sources), "unknown-id-123");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id(), "arxiv");
    }

    #[test]
    fn test_paper_id_upper_start() {
        assert!(paper_id_upper_start("PMC12345", "PMC"));
        assert!(paper_id_upper_start("pmc12345", "PMC"));
        assert!(!paper_id_upper_start("", "PMC"));
    }

    fn empty_sources() -> Arc<Vec<Arc<dyn Source>>> {
        Arc::new(Vec::new())
    }

    fn test_paper(id: &str, title: &str, source: SourceType) -> Paper {
        let mut paper = Paper::new(
            id.to_string(),
            title.to_string(),
            format!("https://example.com/{id}"),
            source,
        );
        paper.authors = "Ada Lovelace; Alan Turing".to_string();
        paper.doi = Some("10.1234/example".to_string());
        paper.published_date = Some("2024-01-01".to_string());
        paper
    }

    #[tokio::test]
    async fn test_author_profile_handler_errors_for_missing_author_param() {
        let handler = AuthorProfileHandler {
            sources: empty_sources(),
        };

        let err = handler
            .execute(json!({}))
            .await
            .expect_err("missing author should fail");
        assert!(err.contains("Missing 'author' parameter"));
    }

    #[tokio::test]
    async fn test_batch_get_papers_handler_empty_paper_ids_returns_empty_result() {
        let handler = BatchGetPapersHandler {
            sources: empty_sources(),
        };

        let result = handler
            .execute(json!({"paper_ids": []}))
            .await
            .expect("empty paper_ids should be a valid empty batch");

        assert_eq!(result["total_queried"], 0);
        assert_eq!(result["total_found"], 0);
        assert_eq!(result["total_not_found"], 0);
        assert_eq!(result["results"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_export_papers_handler_bibtex_format_with_mock_papers() {
        let handler = ExportPapersHandler {
            sources: empty_sources(),
        };
        let papers = vec![test_paper(
            "paper-1",
            "Test Driven Research",
            SourceType::Arxiv,
        )];

        let result = handler
            .execute(json!({"papers": papers, "format": "bibtex"}))
            .await
            .expect("bibtex export should succeed");

        assert_eq!(result["format"], "bibtex");
        assert_eq!(result["count"], 1);
        let joined = result["joined"].as_str().expect("joined bibtex string");
        assert!(joined.contains("@"));
        assert!(joined.contains("Test Driven Research"));
    }

    #[tokio::test]
    async fn test_citation_graph_handler_errors_for_missing_paper_id() {
        let handler = CitationGraphHandler {
            sources: empty_sources(),
        };

        let err = handler
            .execute(json!({}))
            .await
            .expect_err("missing paper_id should fail");
        assert!(err.contains("Missing 'paper_id' parameter"));
    }

    #[tokio::test]
    async fn test_recommend_papers_handler_errors_for_missing_paper_id() {
        let handler = RecommendPapersHandler {
            sources: empty_sources(),
        };

        let err = handler
            .execute(json!({}))
            .await
            .expect_err("missing paper_id should fail");
        assert!(err.contains("Missing 'paper_id' parameter"));
    }

    #[tokio::test]
    async fn test_web_search_handler_errors_for_missing_query() {
        let handler = WebSearchHandler;

        let err = handler
            .execute(json!({}))
            .await
            .expect_err("missing query should fail");
        assert!(err.contains("Missing 'query' parameter"));
    }

    #[tokio::test]
    async fn test_write_paper_handler_errors_for_missing_required_parameter() {
        let handler = WritePaperHandler;

        let err = handler
            .execute(json!({}))
            .await
            .expect_err("missing papers should fail");
        assert!(err.contains("Missing or invalid 'papers' parameter"));
    }

    #[tokio::test]
    async fn test_deduplicate_papers_handler_strategy_enum_parsing() {
        let handler = DeduplicatePapersHandler;
        let first = test_paper("first", "Duplicate Paper", SourceType::Arxiv);
        let last = test_paper("last", "Duplicate Paper", SourceType::SemanticScholar);

        let first_result = handler
            .execute(json!({"papers": [first.clone(), last.clone()], "strategy": "first"}))
            .await
            .expect("first strategy should succeed");
        let first_papers = first_result.as_array().expect("first result array");
        assert_eq!(first_papers.len(), 1);
        assert_eq!(first_papers[0]["paper_id"], "first");

        let last_result = handler
            .execute(json!({"papers": [first.clone(), last.clone()], "strategy": "last"}))
            .await
            .expect("last strategy should succeed");
        let last_papers = last_result.as_array().expect("last result array");
        assert_eq!(last_papers.len(), 1);
        assert_eq!(last_papers[0]["paper_id"], "last");

        let mark_result = handler
            .execute(json!({"papers": [first, last], "strategy": "mark"}))
            .await
            .expect("mark strategy should succeed");
        let marked_papers = mark_result.as_array().expect("mark result array");
        assert_eq!(marked_papers.len(), 2);
    }

    #[tokio::test]
    async fn test_web_extract_handler_extracts_html_blocks() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"<html><head><title>On Attention</title><style>body{}</style></head>
            <body><script>alert("noise");</script>
            <nav>Menu Menu</nav>
            <article><h2>Results</h2><p>Attention  improves results.</p>
            <ul><li>First finding</li></ul></article></body></html>"#;
        let mock = server
            .mock("GET", "/page")
            .with_status(200)
            .with_header("content-type", "text/html; charset=utf-8")
            .with_body(body)
            .expect(1)
            .create_async()
            .await;

        let handler = WebExtractHandler;
        let out = handler
            .execute(json!({"url": format!("{}/page", server.url())}))
            .await
            .expect("web_extract should succeed");
        mock.assert_async().await;

        assert_eq!(out["title"], "On Attention");
        assert_eq!(out["content_type"], "text/html");
        assert_eq!(out["truncated"], false);
        let text = out["text"].as_str().expect("text field");
        assert!(text.contains("Results"), "heading kept: {text}");
        assert!(
            text.contains("Attention improves results."),
            "paragraph collapsed: {text}"
        );
        assert!(text.contains("First finding"), "list item kept: {text}");
        assert!(!text.contains("noise"), "script text dropped: {text}");
        assert!(!text.contains("Menu"), "nav text dropped: {text}");
    }

    #[tokio::test]
    async fn test_web_extract_handler_truncates_to_max_chars() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/long")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><p>abcdefghijklmnop</p></body></html>")
            .expect(1)
            .create_async()
            .await;

        let handler = WebExtractHandler;
        let out = handler
            .execute(json!({"url": format!("{}/long", server.url()), "max_chars": 8}))
            .await
            .expect("web_extract should succeed");
        mock.assert_async().await;

        assert_eq!(out["truncated"], true);
        assert_eq!(out["returned_chars"], 8);
        assert_eq!(out["total_chars"], 16);
        assert!(out["text"].as_str().unwrap().starts_with("abcdefgh"));
    }

    #[tokio::test]
    async fn test_web_extract_handler_rejects_non_http_scheme() {
        let handler = WebExtractHandler;
        let result = handler
            .execute(json!({"url": "ftp://example.com/file"}))
            .await;
        let err = result.expect_err("non-http scheme should fail");
        assert!(
            err.contains("http(s)"),
            "error names scheme restriction: {err}"
        );
    }

    #[tokio::test]
    async fn test_web_extract_handler_routes_pdf_content_to_pdf_extraction() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", mockito::Matcher::Regex(r"/paper\.pdf.*".to_string()))
            .with_status(200)
            .with_header("content-type", "application/pdf")
            .with_body("%PDF-1.4 not actually a pdf")
            .expect(1)
            .create_async()
            .await;

        let handler = WebExtractHandler;
        let result = handler
            .execute(json!({"url": format!("{}/paper.pdf", server.url())}))
            .await;
        mock.assert_async().await;
        let err = result.expect_err("garbage pdf should fail extraction");
        assert!(
            err.contains("PDF extraction failed"),
            "pdf branch should route through the pdf extractor: {err}"
        );
    }

    #[tokio::test]
    async fn test_github_search_handler_maps_repositories() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/search/repositories.*".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"items":[{"full_name":"org/repo","html_url":"https://github.com/org/repo",
                    "description":"code for the paper","stargazers_count":42,
                    "language":"Python","updated_at":"2026-01-01T00:00:00Z",
                    "topics":["machine-learning"]}]}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let handler = GithubSearchHandler {
            api_base: Some(format!("{}/search/repositories", server.url())),
            api_key: None,
        };
        let out = handler
            .execute(json!({"query": "attention is all you need", "max_results": 1}))
            .await
            .expect("github_search should succeed");
        mock.assert_async().await;

        assert_eq!(out["total_results"], 1);
        let repo = &out["results"][0];
        assert_eq!(repo["full_name"], "org/repo");
        assert_eq!(repo["stars"], 42);
        assert_eq!(repo["language"], "Python");
        assert_eq!(repo["topics"][0], "machine-learning");
    }

    #[tokio::test]
    async fn test_github_search_handler_reports_rate_limit_with_key_hint() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(403)
            .with_body(r#"{"message":"API rate limit exceeded"}"#)
            .expect(1)
            .create_async()
            .await;

        let handler = GithubSearchHandler {
            api_base: Some(format!("{}/search/repositories", server.url())),
            api_key: None,
        };
        let result = handler.execute(json!({"query": "test"})).await;
        mock.assert_async().await;
        let err = result.expect_err("rate limit should error");
        assert!(
            err.contains("GITHUB_API_KEY"),
            "error should hint at the key: {err}"
        );
    }

    #[tokio::test]
    async fn test_github_search_handler_falls_back_to_anonymous_on_401() {
        let mut server = mockito::Server::new_async().await;
        let with_key = server
            .mock("GET", mockito::Matcher::Any)
            .match_header("authorization", mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"message":"Bad credentials"}"#)
            .expect(1)
            .create_async()
            .await;
        let anonymous = server
            .mock("GET", mockito::Matcher::Any)
            .match_header("authorization", mockito::Matcher::Missing)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"items":[{"full_name":"org/repo","html_url":"https://github.com/org/repo",
                    "stargazers_count":7}]}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let handler = GithubSearchHandler {
            api_base: Some(format!("{}/search/repositories", server.url())),
            api_key: Some("expired-token".to_string()),
        };
        let out = handler
            .execute(json!({"query": "test"}))
            .await
            .expect("rejected key should fall back to anonymous search");
        with_key.assert_async().await;
        anonymous.assert_async().await;

        assert_eq!(out["results"][0]["full_name"], "org/repo");
    }

    #[tokio::test]
    async fn test_search_tavily_maps_content_snippets() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/search")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"results":[{"title":"Study","url":"https://example.com/a",
                    "content":"The full content text"}]}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let out = search_tavily("query", 5, "key", &format!("{}/search", server.url()))
            .await
            .expect("tavily search should succeed");
        mock.assert_async().await;

        assert_eq!(out["source"], "tavily");
        assert_eq!(out["results"][0]["snippet"], "The full content text");
    }

    #[tokio::test]
    async fn test_search_brave_maps_description_snippets() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", mockito::Matcher::Regex(r"^/web.*".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"web":{"results":[{"title":"Study","url":"https://example.com/a",
                    "description":"The brave description"}]}}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let out = search_brave("query", 5, "key", &format!("{}/web", server.url()))
            .await
            .expect("brave search should succeed");
        mock.assert_async().await;

        assert_eq!(out["source"], "brave");
        assert_eq!(out["results"][0]["snippet"], "The brave description");
    }

    #[test]
    fn test_missing_web_search_keys_error_names_all_providers() {
        let err = missing_web_search_keys_error();
        assert!(err.contains("EXA_API_KEY"));
        assert!(err.contains("TAVILY_API_KEY"));
        assert!(err.contains("BRAVE_API_KEY"));
    }
}
