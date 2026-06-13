//! Unified tool handlers with smart source selection.

use std::sync::Arc;

use futures_util::future;
use serde_json::Value;

use super::tools::ToolHandler;

/// Helper function to auto-detect the appropriate source for a paper ID
fn auto_detect_source(
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

    if paper_id.chars().filter(|&c| c == '/').count() == 1 {
        return sources
            .iter()
            .find(|s| s.id() == "iacr")
            .cloned()
            .ok_or_else(|| "IACR source not available".to_string());
    }

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

        let mut search_query = crate::models::SearchQuery::new(query).max_results(max_results);
        if let Some(ref year) = year {
            search_query = search_query.year(year);
        }
        if let Some(ref cat) = category {
            search_query = search_query.category(cat);
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
                    match source.search(&sq).await {
                        Ok(response) => Ok(response.papers),
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
        for papers in results.into_iter().flatten() {
            all_results.extend(papers);
        }
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
        let source = self.find_source(paper_id, source_override)?;

        // Try direct lookup first (more efficient and accurate)
        match source.get_by_id(paper_id).await {
            Ok(paper) => return serde_json::to_value(&paper).map_err(|e| e.to_string()),
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
                Ok(paper) => return serde_json::to_value(&paper).map_err(|e| e.to_string()),
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
        serde_json::to_value(&response.papers[0]).map_err(|e| e.to_string())
    }
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

        Err(format!(
            "Failed to download '{}' from any source (tried {} and OA fallbacks: {})",
            paper_id,
            source.id(),
            fallback_order.join(", ")
        ))
    }
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
        let source = self.find_source(paper_id, source_override)?;
        let request = crate::models::ReadRequest::new(paper_id, "./downloads");
        let result = source.read(&request).await.map_err(|e| e.to_string())?;
        serde_json::to_value(result).map_err(|e| e.to_string())
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
            return Err("No valid paper IDs provided".to_string());
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
#[derive(Debug)]
pub struct WebSearchHandler;

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

        // Try Exa API first (requires EXA_API_KEY)
        if let Some(api_key) = std::env::var("EXA_API_KEY").ok().filter(|k| !k.is_empty()) {
            return search_exa(query, max_results, &api_key).await;
        }

        Err("Web search requires EXA_API_KEY environment variable. Get one at https://dashboard.exa.ai/api-keys".to_string())
    }
}

/// Search using the Exa API
async fn search_exa(query: &str, max_results: usize, api_key: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .post("https://api.exa.ai/search")
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

    let output = serde_json::json!({
        "query": query,
        "source": "exa",
        "total_results": results.len(),
        "results": results,
    });

    serde_json::to_value(output).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CitationRequest, DownloadRequest, ReadRequest};
    use crate::sources::{Source, SourceCapabilities};
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
}
