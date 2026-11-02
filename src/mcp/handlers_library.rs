//! MCP tool handlers for the local saved-papers library.

use serde_json::{json, Value};

use super::tools::ToolHandler;
use crate::models::{Paper, SourceType};
use crate::utils::evidence::{check_retraction, check_retraction_from, RetractionInfo};
use crate::utils::library::{
    AddedOrUpdated, PaperLibrary, RetractionWrite, RetractionWriteOutcome,
};
use crate::utils::HttpClient;

fn collection_of(args: &Value) -> Option<String> {
    args.get("collection")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn tags_of(args: &Value) -> Vec<String> {
    match args.get("tags") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|t| !t.trim().is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn limit_of(args: &Value, default: usize) -> usize {
    args.get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(default)
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Missing '{key}' parameter"))
}

fn non_empty_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
}

fn source_from(value: Option<&Value>) -> SourceType {
    match value.and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => {
            serde_json::from_value::<SourceType>(Value::String(s.to_string()))
                .unwrap_or_else(|_| SourceType::Other(s.to_string()))
        }
        _ => SourceType::Other("local".to_string()),
    }
}

/// Build a Paper from a full `paper` object, or from `paper_id`/`doi` + `title`.
fn paper_from_args(args: &Value) -> Result<Paper, String> {
    if let Some(paper_value) = args.get("paper") {
        return serde_json::from_value::<Paper>(paper_value.clone())
            .map_err(|e| format!("Invalid 'paper' JSON: {e}"));
    }

    let title = require_str(args, "title")?;
    let paper_id = match non_empty_str(args, "paper_id") {
        Some(id) => id.to_string(),
        None => non_empty_str(args, "doi")
            .ok_or(
                "Missing 'paper': provide a full paper object, or 'paper_id'/'doi' with 'title'",
            )?
            .to_string(),
    };
    let url = non_empty_str(args, "url").unwrap_or_default().to_string();
    let mut paper = Paper::new(
        paper_id,
        title.to_string(),
        url,
        source_from(args.get("source")),
    );
    if let Some(doi) = non_empty_str(args, "doi") {
        paper.doi = Some(doi.to_string());
    }
    if let Some(authors) = non_empty_str(args, "authors") {
        paper.authors = authors.to_string();
    }
    if let Some(abstract_text) = non_empty_str(args, "abstract") {
        paper.r#abstract = abstract_text.to_string();
    }
    Ok(paper)
}

/// OpenAlex works prefix for retraction lookups; `RESEARCH_MASTER_OPENALEX_WORKS_URL`
/// overrides it (test seam, mirroring `RESEARCH_MASTER_ARXIV_API_BASE`).
fn openalex_works_url() -> Option<String> {
    std::env::var("RESEARCH_MASTER_OPENALEX_WORKS_URL")
        .ok()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// Best-effort OpenAlex retraction lookup for a DOI; `None` on any failure so
/// the caller records "unknown" instead of failing.
async fn lookup_retraction(doi: &str) -> Option<RetractionInfo> {
    let client = match HttpClient::new() {
        Ok(client) => client,
        Err(e) => {
            tracing::debug!("retraction lookup unavailable: {e}");
            return None;
        }
    };
    let result = match openalex_works_url() {
        Some(base) => check_retraction_from(&base, &client, doi).await,
        None => check_retraction(&client, doi).await,
    };
    match result {
        Ok(info) => Some(info),
        Err(e) => {
            tracing::debug!("retraction lookup failed for {doi}: {e}");
            None
        }
    }
}

/// Add a paper to the saved library, or refresh it when already saved.
#[derive(Debug)]
pub struct LibraryAddHandler;

#[async_trait::async_trait]
impl ToolHandler for LibraryAddHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper = paper_from_args(&args)?;
        let collection = collection_of(&args);
        let tags = tags_of(&args);

        let library = tokio::task::spawn_blocking(PaperLibrary::new)
            .await
            .map_err(|e| e.to_string())?;
        let (store, saved_paper, saved_collection, saved_tags) = (
            library.clone(),
            paper.clone(),
            collection.clone(),
            tags.clone(),
        );
        let outcome = tokio::task::spawn_blocking(move || {
            store.add_paper(saved_paper, saved_collection, saved_tags)
        })
        .await
        .map_err(|e| e.to_string())??;
        let status = match outcome {
            AddedOrUpdated::Added => "added",
            AddedOrUpdated::Updated => "updated",
        };

        // Best-effort retraction check on the DOI; a failed lookup records
        // "unknown" and never fails the add.
        let mut retraction_status: Option<String> = None;
        let mut retraction_date: Option<String> = None;
        let mut checked_at: Option<String> = None;
        let mut retraction_status_persisted = None;
        if let Some(doi) = paper
            .doi
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            let (found_status, date) = match lookup_retraction(doi).await {
                Some(info) => (
                    if info.is_retracted {
                        "retracted"
                    } else {
                        "not_retracted"
                    },
                    info.retraction_date,
                ),
                None => ("unknown", None),
            };
            let id = if paper.paper_id.trim().is_empty() {
                doi
            } else {
                paper.paper_id.as_str()
            };
            let at = chrono::Utc::now().to_rfc3339();
            let write = if found_status == "unknown" {
                RetractionWrite::PreserveConfirmedOnFailure
            } else {
                RetractionWrite::Overwrite(found_status.to_string(), date)
            };
            let id = id.to_string();
            let doi = doi.to_string();
            let saved_collection = collection.clone();
            let saved_at = at.clone();
            let result = tokio::task::spawn_blocking(move || {
                library.set_retraction_scoped(
                    &id,
                    saved_collection.as_deref(),
                    Some(&doi),
                    write,
                    saved_at,
                )
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|result| result);
            match result {
                Ok(RetractionWriteOutcome::Applied { status, date }) => {
                    retraction_status = Some(status);
                    retraction_date = date;
                    checked_at = Some(at);
                    retraction_status_persisted = Some(true);
                }
                Ok(_) => retraction_status_persisted = Some(false),
                Err(e) => {
                    tracing::warn!("library retraction write failed: {e}");
                    retraction_status_persisted = Some(false);
                }
            }
        }

        Ok(json!({
            "status": status,
            "paper_id": paper.paper_id,
            "title": paper.title,
            "doi": paper.doi,
            "collection": collection,
            "tags": tags,
            "retraction_status": retraction_status,
            "retraction_date": retraction_date,
            "retraction_checked_at": checked_at,
            "retraction_status_persisted": retraction_status_persisted,
        }))
    }
}

/// List saved papers, newest first.
#[derive(Debug)]
pub struct LibraryListHandler;

#[async_trait::async_trait]
impl ToolHandler for LibraryListHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let collection = collection_of(&args);
        let limit = limit_of(&args, 100);

        let library = tokio::task::spawn_blocking(PaperLibrary::new)
            .await
            .map_err(|e| e.to_string())?;
        let entries = tokio::task::spawn_blocking(move || library.list(collection.as_deref()))
            .await
            .map_err(|e| e.to_string())?;
        let total = entries.len();
        let papers: Result<Vec<Value>, String> = entries
            .into_iter()
            .take(limit)
            .map(|entry| {
                serde_json::to_value(&entry).map_err(|e| format!("cannot serialize entry: {e}"))
            })
            .collect();
        let papers = papers?;
        Ok(json!({
            "total": total,
            "returned": papers.len(),
            "papers": papers,
        }))
    }
}

/// Remove a saved paper by id or DOI.
#[derive(Debug)]
pub struct LibraryRemoveHandler;

#[async_trait::async_trait]
impl ToolHandler for LibraryRemoveHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let id = match non_empty_str(&args, "id") {
            Some(id) => id.to_string(),
            None => require_str(&args, "paper_id")?.to_string(),
        };
        let collection = collection_of(&args);

        let library = tokio::task::spawn_blocking(PaperLibrary::new)
            .await
            .map_err(|e| e.to_string())?;
        let saved_id = id.clone();
        let saved_collection = collection.clone();
        let removed = tokio::task::spawn_blocking(move || {
            library.remove_paper(&saved_id, saved_collection.as_deref())
        })
        .await
        .map_err(|e| e.to_string())?;
        Ok(json!({
            "removed": removed,
            "id": id,
            "collection": collection,
        }))
    }
}

/// Keyword search over saved papers.
#[derive(Debug)]
pub struct LibrarySearchHandler;

#[async_trait::async_trait]
impl ToolHandler for LibrarySearchHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let query = require_str(&args, "query")?.to_string();
        let collection = collection_of(&args);
        let retracted_only = args
            .get("retracted_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let limit = limit_of(&args, 10);

        let library = tokio::task::spawn_blocking(PaperLibrary::new)
            .await
            .map_err(|e| e.to_string())?;
        let saved_query = query.clone();
        let entries = tokio::task::spawn_blocking(move || {
            library.search(&saved_query, collection.as_deref(), retracted_only, limit)
        })
        .await
        .map_err(|e| e.to_string())?;
        let results: Result<Vec<Value>, String> = entries
            .into_iter()
            .map(|(entry, score)| {
                serde_json::to_value(&entry.paper)
                    .map(|paper| {
                        json!({
                            "score": score,
                            "paper": paper,
                            "collection": entry.collection,
                            "tags": entry.tags,
                            "notes": entry.notes,
                            "retraction_status": entry.retraction_status,
                            "retraction_date": entry.retraction_date,
                            "retraction_checked_at": entry.retraction_checked_at,
                        })
                    })
                    .map_err(|e| format!("cannot serialize entry: {e}"))
            })
            .collect();
        let results = results?;
        Ok(json!({
            "query": query,
            "total": results.len(),
            "results": results,
        }))
    }
}

/// Retrieve the saved papers most relevant to a question.
///
/// Retrieval only: no language model is involved and no answer text is generated.
#[derive(Debug)]
pub struct LibraryAskHandler;

#[async_trait::async_trait]
impl ToolHandler for LibraryAskHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let question = require_str(&args, "question")?.to_string();
        let collection = collection_of(&args);
        let top_k = args
            .get("top_k")
            .or_else(|| args.get("limit"))
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(5);

        let library = tokio::task::spawn_blocking(PaperLibrary::new)
            .await
            .map_err(|e| e.to_string())?;
        let answer = tokio::task::spawn_blocking(move || {
            library.ask(&question, collection.as_deref(), top_k)
        })
        .await
        .map_err(|e| e.to_string())?;
        let matched: Result<Vec<Value>, String> = answer
            .matched
            .iter()
            .map(|m| {
                serde_json::to_value(&m.entry)
                    .map(|entry| {
                        json!({
                            "score": m.score,
                            "matched_terms": m.matched_terms,
                            "entry": entry,
                        })
                    })
                    .map_err(|e| format!("cannot serialize entry: {e}"))
            })
            .collect();
        let matched = matched?;
        Ok(json!({
            "question": answer.question,
            "note": "Retrieval over saved library papers only; no language model was involved.",
            "total": matched.len(),
            "matched": matched,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::LibraryEntry;

    use crate::test_support::{EnvSnapshot, ENV_LOCK};

    /// Point PaperLibrary::new() at a fresh temp dir for one test.
    async fn scoped_library_path(
        name: &str,
    ) -> (tokio::sync::MutexGuard<'static, ()>, EnvSnapshot) {
        let guard = ENV_LOCK.lock().await;
        let env = EnvSnapshot::new(&[
            "RESEARCH_MASTER_LIBRARY_PATH",
            "RESEARCH_MASTER_OPENALEX_WORKS_URL",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]);
        let dir = std::env::temp_dir().join(format!(
            "research-master-handler-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("RESEARCH_MASTER_LIBRARY_PATH", &dir);
        (guard, env)
    }

    fn sample_paper() -> Value {
        json!({
            "paper_id": "2401.00001",
            "title": "Sparse Attention Survey",
            "authors": "A. Author; B. Author",
            "abstract": "A survey of sparse attention methods for large models.",
            "doi": "10.1000/sparse",
            "published_date": "2024-01-01",
            "updated_date": null,
            "pdf_url": null,
            "url": "https://arxiv.org/abs/2401.00001",
            "source": "arxiv",
            "categories": null,
            "keywords": null,
            "citations": null,
            "references": null,
            "extra": null,
            "license": null
        })
    }

    /// Ambient proxy vars would route mockito-bound requests through a proxy;
    /// clear them while the env guard is held, restore with the returned pair
    /// list when done (same trick as the evidence module's test client).
    fn clear_proxy_env() -> Vec<(&'static str, Option<String>)> {
        const VARS: [&str; 6] = [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ];
        let saved: Vec<(&'static str, Option<String>)> =
            VARS.iter().map(|v| (*v, std::env::var(v).ok())).collect();
        for var in VARS {
            std::env::remove_var(var);
        }
        saved
    }

    fn restore_proxy_env(saved: Vec<(&'static str, Option<String>)>) {
        for (var, value) in saved {
            match value {
                Some(v) => std::env::set_var(var, v),
                None => std::env::remove_var(var),
            }
        }
    }

    #[test]
    fn old_entry_json_without_retraction_fields_loads_with_defaults() {
        let stored_paper = serde_json::to_string(&sample_paper()).unwrap();
        let old_line = format!(
            r#"{{"added_at":1700000000,"collection":"survey","tags":["attention"],"notes":"n","paper":{stored_paper}}}"#
        );
        let entry: LibraryEntry = serde_json::from_str(&old_line).unwrap();
        assert_eq!(entry.retraction_status, None);
        assert_eq!(entry.retraction_date, None);
        assert_eq!(entry.retraction_checked_at, None);
    }

    #[tokio::test]
    async fn add_list_remove_roundtrip_via_handlers() {
        let (_guard, _env) = scoped_library_path("roundtrip").await;
        let mut server = mockito::Server::new_async().await;
        let retraction_mock = server
            .mock("GET", "/works/doi:10.1000%2Fsparse")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"id":"https://openalex.org/W1","title":"Sparse Attention Survey",
                    "is_retracted":true,"retraction_date":"2024-06-01"}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let saved_proxies = clear_proxy_env();
        std::env::set_var(
            "RESEARCH_MASTER_OPENALEX_WORKS_URL",
            format!("{}/works/doi:", server.url()),
        );

        let added = LibraryAddHandler
            .execute(json!({
                "paper": sample_paper(),
                "collection": "survey",
                "tags": ["attention"]
            }))
            .await
            .unwrap();
        assert_eq!(added["status"], "added");
        assert_eq!(added["paper_id"], "2401.00001");
        assert_eq!(added["collection"], "survey");
        assert_eq!(added["retraction_status"], "retracted");
        assert_eq!(added["retraction_date"], "2024-06-01");
        assert!(added["retraction_checked_at"].as_str().is_some());

        let listed = LibraryListHandler
            .execute(json!({ "collection": "survey" }))
            .await
            .unwrap();
        assert_eq!(listed["total"], 1);
        assert_eq!(listed["returned"], 1);
        assert_eq!(listed["papers"][0]["paper"]["paper_id"], "2401.00001");
        assert_eq!(listed["papers"][0]["tags"][0], "attention");
        assert_eq!(listed["papers"][0]["retraction_status"], "retracted");
        assert_eq!(listed["papers"][0]["retraction_date"], "2024-06-01");

        let removed = LibraryRemoveHandler
            .execute(json!({ "id": "10.1000/sparse" }))
            .await
            .unwrap();
        assert_eq!(removed["removed"], true);

        let listed = LibraryListHandler.execute(json!({})).await.unwrap();
        assert_eq!(listed["total"], 0);

        retraction_mock.assert_async().await;
        restore_proxy_env(saved_proxies);
        std::env::remove_var("RESEARCH_MASTER_OPENALEX_WORKS_URL");
    }

    #[tokio::test]
    async fn add_minimal_fields_constructs_paper() {
        let (_guard, _env) = scoped_library_path("minimal").await;
        // A DOI OpenAlex does not index leaves the entry "unknown" without
        // failing the add (404 is a permanent error, so no retries).
        let mut server = mockito::Server::new_async().await;
        let missing_mock = server
            .mock("GET", "/works/doi:10.9999%2Fmin")
            .with_status(404)
            .with_body("not found")
            .expect(1)
            .create_async()
            .await;
        let saved_proxies = clear_proxy_env();
        std::env::set_var(
            "RESEARCH_MASTER_OPENALEX_WORKS_URL",
            format!("{}/works/doi:", server.url()),
        );

        let added = LibraryAddHandler
            .execute(json!({
                "doi": "10.9999/min",
                "title": "Minimal Paper",
                "authors": "C. Author",
                "abstract": "Bare bones."
            }))
            .await
            .unwrap();
        assert_eq!(added["status"], "added");
        assert_eq!(added["paper_id"], "10.9999/min");
        assert_eq!(added["retraction_status"], "unknown");
        assert_eq!(added["retraction_date"], Value::Null);
        assert!(added["retraction_checked_at"].as_str().is_some());

        missing_mock.assert_async().await;
        restore_proxy_env(saved_proxies);
        std::env::remove_var("RESEARCH_MASTER_OPENALEX_WORKS_URL");
    }

    #[tokio::test]
    async fn re_add_reports_updated() {
        let (_guard, _env) = scoped_library_path("re-add").await;

        let args = json!({
            "paper_id": "1234.5678",
            "title": "Twice Saved",
            "collection": "a"
        });
        assert_eq!(
            LibraryAddHandler.execute(args.clone()).await.unwrap()["status"],
            "added"
        );
        // Same collection updates in place; a different collection adds a
        // second membership for the same paper.
        let same = LibraryAddHandler
            .execute(json!({ "paper_id": "1234.5678", "title": "Twice Saved", "collection": "a" }))
            .await
            .unwrap();
        assert_eq!(same["status"], "updated");
        let other = LibraryAddHandler
            .execute(json!({ "paper_id": "1234.5678", "title": "Twice Saved", "collection": "b" }))
            .await
            .unwrap();
        assert_eq!(other["status"], "added");
    }

    #[tokio::test]
    async fn search_handler_returns_scored_results() {
        let (_guard, _env) = scoped_library_path("search").await;

        LibraryAddHandler
            .execute(json!({
                "paper_id": "attn-1",
                "title": "Transformer Architectures",
                "abstract": "Self-attention and transformers."
            }))
            .await
            .unwrap();
        LibraryAddHandler
            .execute(json!({
                "paper_id": "gene-1",
                "title": "CRISPR Screening",
                "abstract": "Genome editing methods."
            }))
            .await
            .unwrap();

        let results = LibrarySearchHandler
            .execute(json!({ "query": "transformer" }))
            .await
            .unwrap();
        assert_eq!(results["total"], 1);
        assert!(results["results"][0]["score"].as_f64().unwrap() > 0.0);
        assert_eq!(results["results"][0]["paper"]["paper_id"], "attn-1");
    }

    #[tokio::test]
    async fn ask_handler_reports_matched_terms() {
        let (_guard, _env) = scoped_library_path("ask").await;

        LibraryAddHandler
            .execute(json!({
                "paper_id": "p1",
                "title": "Protein Folding",
                "abstract": "How protein folding determines structure."
            }))
            .await
            .unwrap();

        let answer = LibraryAskHandler
            .execute(json!({ "question": "what determines protein folding?", "top_k": 3 }))
            .await
            .unwrap();
        assert_eq!(answer["total"], 1);
        let terms = answer["matched"][0]["matched_terms"].as_array().unwrap();
        assert!(terms.contains(&json!("protein")));
        assert!(terms.contains(&json!("folding")));
        assert!(answer["note"]
            .as_str()
            .unwrap()
            .contains("no language model"));

        let empty = LibraryAskHandler
            .execute(json!({ "question": "zzqq xyzzy" }))
            .await
            .unwrap();
        assert_eq!(empty["total"], 0);
    }

    #[tokio::test]
    async fn missing_required_args_error() {
        // Error paths parse args before touching storage, so no env setup needed.
        let err = LibraryAddHandler.execute(json!({})).await.unwrap_err();
        assert!(err.contains("title"), "got: {err}");

        let err = LibraryAddHandler
            .execute(json!({ "title": "No Id" }))
            .await
            .unwrap_err();
        assert!(
            err.contains("paper_id") || err.contains("doi"),
            "got: {err}"
        );

        let err = LibraryAddHandler
            .execute(json!({ "paper": {"paper_id": 1} }))
            .await
            .unwrap_err();
        assert!(err.contains("Invalid 'paper'"), "got: {err}");

        let err = LibraryRemoveHandler.execute(json!({})).await.unwrap_err();
        assert!(err.contains("paper_id"), "got: {err}");

        let err = LibrarySearchHandler.execute(json!({})).await.unwrap_err();
        assert!(err.contains("query"), "got: {err}");

        let err = LibraryAskHandler.execute(json!({})).await.unwrap_err();
        assert!(err.contains("question"), "got: {err}");
    }
}
