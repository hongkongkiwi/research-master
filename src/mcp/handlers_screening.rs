//! MCP tool handlers for systematic-review screening sessions.

use std::sync::Arc;

use futures_util::future;
use serde_json::{json, Value};

use super::tools::ToolHandler;
use crate::utils::screening::{extract_statistics, ScreeningStore};

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

fn limit_of(args: &Value, default: usize, cap: usize) -> usize {
    args.get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| (v as usize).min(cap))
        .unwrap_or(default)
}

fn max_results_of(args: &Value) -> usize {
    args.get("max_results")
        .and_then(|v| v.as_u64())
        .map(|v| (v as usize).min(200))
        .unwrap_or(50)
}

/// Candidate copy with `extracted_statistics` pulled from its title + abstract.
fn with_extracted_statistics(candidate: &Value) -> Value {
    let mut obj = match candidate {
        Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    let text = [
        candidate.get("abstract").and_then(Value::as_str),
        candidate.get("title").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    obj.insert(
        "extracted_statistics".to_string(),
        Value::Array(
            extract_statistics(&text)
                .into_iter()
                .map(Value::String)
                .collect(),
        ),
    );
    Value::Object(obj)
}

/// Explicit candidate list; each entry needs non-empty string `paper_id` and `title`.
fn candidates_from_papers(papers: &Value) -> Result<Vec<Value>, String> {
    let entries = papers
        .as_array()
        .ok_or("'papers' must be an array of paper objects")?;
    let mut candidates = Vec::with_capacity(entries.len());
    let mut seen = std::collections::HashSet::new();
    for (i, entry) in entries.iter().enumerate() {
        let has_id = entry
            .get("paper_id")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
        let has_title = entry
            .get("title")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
        if !has_id || !has_title {
            return Err(format!(
                "papers[{i}] must be an object with non-empty 'paper_id' and 'title' strings"
            ));
        }
        if seen.insert(entry["paper_id"].as_str().unwrap()) {
            candidates.push(entry.clone());
        }
    }
    Ok(candidates)
}

/// Fan out a search across all sources (SearchPapersHandler approach) and
/// return deduplicated candidate objects plus the number of duplicates removed.
async fn search_candidates(
    sources: &[Arc<dyn crate::sources::Source>],
    query: &str,
    max_results: usize,
) -> Result<(Vec<Value>, usize, Vec<Value>), String> {
    let search_query = Arc::new(crate::models::SearchQuery::new(query).max_results(max_results));
    let futures: Vec<_> = sources
        .iter()
        .filter(|s| s.supports_search())
        .map(|source| {
            let source = Arc::clone(source);
            let sq = Arc::clone(&search_query);
            async move {
                let cache =
                    crate::utils::CacheService::from_config(crate::config::get_config().cache);
                if let crate::utils::CacheResult::Hit(response) = cache.get_search(&sq, source.id())
                {
                    return Ok(response.papers);
                }
                match source.search(&sq).await {
                    Ok(response) => {
                        cache.set_search(source.id(), &sq, &response);
                        Ok(response.papers)
                    }
                    Err(e) => {
                        tracing::warn!("Screening search failed for {}: {}", source.id(), e);
                        Err((source.id().to_string(), e.to_string()))
                    }
                }
            }
        })
        .collect();
    let results = future::join_all(futures).await;
    let mut papers = Vec::new();
    let mut failures = Vec::new();
    for result in results {
        match result {
            Ok(found) => papers.extend(found),
            Err((id, error)) => failures.push(json!({"source": id, "error": error})),
        }
    }
    // Empty with failures is not the same as "no results": surface the
    // distinction or callers cannot tell a silent outage from a miss.
    if papers.is_empty() && !failures.is_empty() {
        return Err(format!(
            "All sources failed: {}",
            serde_json::to_string(&failures).unwrap_or_default()
        ));
    }
    if let Err(error) = crate::utils::HistoryService::new().add_search(query, None) {
        tracing::warn!("Could not record search history: {}", error);
    }
    let identified = papers.len();
    let papers = crate::utils::deduplicate_papers(papers, crate::utils::DuplicateStrategy::First);
    let duplicates_removed = identified - papers.len();
    let mut candidates = Vec::with_capacity(papers.len());
    for paper in papers {
        let value =
            serde_json::to_value(&paper).map_err(|e| format!("cannot serialize candidate: {e}"))?;
        candidates.push(value);
    }
    Ok((candidates, duplicates_removed, failures))
}

/// Start a screening session from a source query or an explicit candidate list.
#[derive(Debug)]
pub struct ScreeningStartHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for ScreeningStartHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let inclusion = require_str(&args, "inclusion_criteria")?.to_string();
        let exclusion = require_str(&args, "exclusion_criteria")?.to_string();
        let name = non_empty_str(&args, "name")
            .unwrap_or("screening")
            .to_string();
        let query_arg = non_empty_str(&args, "query");
        let papers_arg = args.get("papers");
        let (candidates, duplicates_removed, query, failures) = match (query_arg, papers_arg) {
            (Some(_), Some(_)) => {
                return Err("Provide either 'query' or 'papers', not both".to_string());
            }
            (None, None) => {
                return Err(
                    "Provide 'query' (search string) or 'papers' (candidate list)".to_string(),
                );
            }
            (Some(query), None) => {
                let max_results = max_results_of(&args);
                let (candidates, duplicates_removed, failures) =
                    search_candidates(&self.sources, query, max_results).await?;
                (candidates, duplicates_removed, query.to_string(), failures)
            }
            (None, Some(papers)) => {
                let candidates = candidates_from_papers(papers)?;
                let duplicates = papers.as_array().unwrap().len() - candidates.len();
                (candidates, duplicates, String::new(), Vec::new())
            }
        };
        if candidates.is_empty() {
            return Err(
                "No candidate papers identified; widen the query or provide papers".to_string(),
            );
        }

        let saved_name = name.clone();
        let (session_id, summary, first_batch) = tokio::task::spawn_blocking(move || {
            let store = ScreeningStore::new();
            let session_id = store.create_session(
                &saved_name,
                &query,
                &inclusion,
                &exclusion,
                candidates,
                duplicates_removed,
            )?;
            let summary = store.summary(&session_id)?.ok_or_else(|| {
                format!("session '{session_id}' disappeared immediately after creation")
            })?;
            let first_batch: Vec<Value> = store
                .undecided(&session_id, 5)?
                .iter()
                .map(with_extracted_statistics)
                .collect();
            Ok::<_, String>((session_id, summary, first_batch))
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(json!({
            "session_id": session_id,
            "name": name,
            "identified": summary.identified,
            "duplicates_removed": summary.duplicates_removed,
            "to_screen": summary.remaining,
            "first_batch": first_batch,
            "source_failures": failures,
        }))
    }
}

/// Fetch the next undecided candidates in a screening session.
#[derive(Debug)]
pub struct ScreeningNextHandler;

#[async_trait::async_trait]
impl ToolHandler for ScreeningNextHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let session_id = require_str(&args, "session_id")?.to_string();
        let limit = limit_of(&args, 10, 50);
        let saved_id = session_id.clone();
        let (batch, remaining) = tokio::task::spawn_blocking(move || {
            let store = ScreeningStore::new();
            let batch: Vec<Value> = store
                .undecided(&saved_id, limit)?
                .iter()
                .map(with_extracted_statistics)
                .collect();
            let remaining = store
                .summary(&saved_id)?
                .ok_or_else(|| format!("no screening session with id '{saved_id}'"))?
                .remaining;
            Ok::<_, String>((batch, remaining))
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(json!({
            "session_id": session_id,
            "batch": batch,
            "remaining": remaining,
        }))
    }
}

/// Record include/exclude/uncertain decisions for candidates.
#[derive(Debug)]
pub struct ScreeningDecideHandler;

#[async_trait::async_trait]
impl ToolHandler for ScreeningDecideHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let session_id = require_str(&args, "session_id")?.to_string();
        let entries = args
            .get("decisions")
            .and_then(Value::as_array)
            .ok_or("Missing 'decisions' parameter (array of {paper_id, decision, reason})")?;
        let mut parsed = Vec::with_capacity(entries.len());
        for (i, entry) in entries.iter().enumerate() {
            let paper_id = entry
                .get("paper_id")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    format!("decisions[{i}] is missing a non-empty string 'paper_id'")
                })?;
            let decision = entry
                .get("decision")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| format!("decisions[{i}] is missing a string 'decision'"))?;
            let reason = entry
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            parsed.push((paper_id.to_string(), decision.to_string(), reason));
        }

        let saved_id = session_id.clone();
        let (outcome, summary) = tokio::task::spawn_blocking(move || {
            let store = ScreeningStore::new();
            let outcome = store.record_decisions(&saved_id, parsed)?;
            let summary = store
                .summary(&saved_id)?
                .ok_or_else(|| format!("no screening session with id '{saved_id}'"))?;
            Ok::<_, String>((outcome, summary))
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(json!({
            "session_id": session_id,
            "recorded": outcome.recorded,
            "ignored_unknown": outcome.ignored_unknown,
            "summary": summary,
        }))
    }
}

/// PRISMA flow summary for a screening session.
#[derive(Debug)]
pub struct ScreeningSummaryHandler;

#[async_trait::async_trait]
impl ToolHandler for ScreeningSummaryHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let session_id = require_str(&args, "session_id")?.to_string();
        let saved_id = session_id.clone();
        let summary = tokio::task::spawn_blocking(move || {
            ScreeningStore::new()
                .summary(&saved_id)?
                .ok_or_else(|| format!("no screening session with id '{saved_id}'"))
        })
        .await
        .map_err(|e| e.to_string())??;
        let summary_text = format!(
            "identified {} → screened {} → excluded {} → included {}, {} uncertain, {} remaining",
            summary.identified,
            summary.screened,
            summary.excluded,
            summary.included,
            summary.uncertain,
            summary.remaining
        );
        Ok(json!({
            "session_id": session_id,
            "identified": summary.identified,
            "duplicates_removed": summary.duplicates_removed,
            "screened": summary.screened,
            "included": summary.included,
            "excluded": summary.excluded,
            "excluded_by_reason": summary.excluded_by_reason,
            "uncertain": summary.uncertain,
            "remaining": summary.remaining,
            "summary_text": summary_text,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point ScreeningStore::new() at a fresh temp dir for one test.
    async fn scoped_screening_path(name: &str) -> tokio::sync::MutexGuard<'static, ()> {
        let guard = crate::test_support::ENV_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!(
            "research-master-screening-handler-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("RESEARCH_MASTER_SCREENINGS_PATH", &dir);
        guard
    }

    fn empty_sources() -> Arc<Vec<Arc<dyn crate::sources::Source>>> {
        Arc::new(Vec::<Arc<dyn crate::sources::Source>>::new())
    }

    #[tokio::test]
    async fn start_next_decide_summary_roundtrip() {
        let _guard = scoped_screening_path("roundtrip").await;

        let started = ScreeningStartHandler {
            sources: empty_sources(),
        }
        .execute(json!({
            "inclusion_criteria": "RCTs in adults",
            "exclusion_criteria": "animal studies",
            "name": "demo",
            "papers": [
                { "paper_id": "p1", "title": "Trial One",
                  "abstract": "d = 0.45, 95% CI [0.12, 0.78], p < 0.001, N = 234" },
                { "paper_id": "p2", "title": "Trial Two", "abstract": "OR = 1.23, p=0.04" },
                { "paper_id": "p3", "title": "Trial Three" }
            ]
        }))
        .await
        .unwrap();
        assert_eq!(started["name"], "demo");
        assert_eq!(started["identified"], 3);
        assert_eq!(started["duplicates_removed"], 0);
        assert_eq!(started["to_screen"], 3);
        let first_batch = started["first_batch"].as_array().unwrap();
        assert_eq!(first_batch.len(), 3);
        assert!(first_batch[0]["extracted_statistics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap() == "d = 0.45"));
        let session_id = started["session_id"].as_str().unwrap().to_string();

        let next = ScreeningNextHandler
            .execute(json!({ "session_id": session_id, "limit": 2 }))
            .await
            .unwrap();
        assert_eq!(next["session_id"], session_id.as_str());
        assert_eq!(next["batch"].as_array().unwrap().len(), 2);
        assert_eq!(next["remaining"], 3);

        let decided = ScreeningDecideHandler
            .execute(json!({
                "session_id": session_id,
                "decisions": [
                    { "paper_id": "p1", "decision": "include", "reason": "RCT in adults" },
                    { "paper_id": "p2", "decision": "exclude", "reason": "Not an RCT" },
                    { "paper_id": "p404", "decision": "exclude", "reason": "unknown paper" }
                ]
            }))
            .await
            .unwrap();
        assert_eq!(decided["recorded"], 2);
        assert_eq!(decided["ignored_unknown"], 1);
        assert_eq!(decided["summary"]["screened"], 2);
        assert_eq!(decided["summary"]["included"], 1);
        assert_eq!(decided["summary"]["excluded_by_reason"]["not an rct"], 1);

        let summary = ScreeningSummaryHandler
            .execute(json!({ "session_id": session_id }))
            .await
            .unwrap();
        assert_eq!(summary["identified"], 3);
        assert_eq!(summary["screened"], 2);
        assert_eq!(summary["remaining"], 1);
        assert!(summary["summary_text"]
            .as_str()
            .unwrap()
            .contains("identified 3"));

        // State went through the real store file, so a new handler call sees it.
        let after = ScreeningNextHandler
            .execute(json!({ "session_id": session_id }))
            .await
            .unwrap();
        assert_eq!(after["remaining"], 1);
    }

    #[tokio::test]
    async fn explicit_duplicates_are_removed_first_wins() {
        let _guard = scoped_screening_path("duplicates").await;
        let result = ScreeningStartHandler {
            sources: empty_sources(),
        }
        .execute(
            json!({"inclusion_criteria":"a", "exclusion_criteria":"b", "papers":[
                {"paper_id":"p", "title":"First"}, {"paper_id":"p", "title":"Second"}
            ]}),
        )
        .await
        .unwrap();
        assert_eq!(result["duplicates_removed"], 1);
        assert_eq!(result["to_screen"], 1);
        assert_eq!(result["first_batch"][0]["title"], "First");
    }

    #[tokio::test]
    async fn start_arg_validation_errors() {
        let _guard = scoped_screening_path("validation").await;
        let handler = ScreeningStartHandler {
            sources: empty_sources(),
        };

        let err = handler
            .execute(json!({ "exclusion_criteria": "b" }))
            .await
            .unwrap_err();
        assert!(err.contains("inclusion_criteria"), "got: {err}");

        let err = handler
            .execute(json!({ "inclusion_criteria": "a" }))
            .await
            .unwrap_err();
        assert!(err.contains("exclusion_criteria"), "got: {err}");

        let err = handler
            .execute(json!({ "inclusion_criteria": "a", "exclusion_criteria": "b" }))
            .await
            .unwrap_err();
        assert!(
            err.contains("query") && err.contains("papers"),
            "got: {err}"
        );

        let err = handler
            .execute(json!({
                "inclusion_criteria": "a",
                "exclusion_criteria": "b",
                "query": "q",
                "papers": [ { "paper_id": "p", "title": "t" } ]
            }))
            .await
            .unwrap_err();
        assert!(err.contains("not both"), "got: {err}");

        let err = handler
            .execute(json!({
                "inclusion_criteria": "a",
                "exclusion_criteria": "b",
                "papers": [ { "paper_id": "p" } ]
            }))
            .await
            .unwrap_err();
        assert!(
            err.contains("papers[0]") && err.contains("title"),
            "got: {err}"
        );

        let err = handler
            .execute(json!({
                "inclusion_criteria": "a",
                "exclusion_criteria": "b",
                "papers": []
            }))
            .await
            .unwrap_err();
        assert!(err.contains("No candidate papers"), "got: {err}");

        let err = ScreeningDecideHandler
            .execute(json!({
                "session_id": "s-x",
                "decisions": [ { "paper_id": "p1", "decision": "maybe" } ]
            }))
            .await
            .unwrap_err();
        assert!(err.contains("Invalid decision"), "got: {err}");

        let err = ScreeningSummaryHandler
            .execute(json!({ "session_id": "s-never-created" }))
            .await
            .unwrap_err();
        assert!(err.contains("no screening session"), "got: {err}");

        let err = ScreeningStartHandler {
            sources: empty_sources(),
        }
        .execute(json!({}))
        .await
        .unwrap_err();
        assert!(err.contains("inclusion_criteria"), "got: {err}");
    }
}
