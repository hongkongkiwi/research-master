//! MCP tool handlers for persisted arXiv topic watches.

use std::sync::Arc;

use serde_json::{json, Value};

use super::tools::ToolHandler;
use crate::utils::evidence::{check_retraction, check_retraction_from};
use crate::utils::watch::{check_watch, WatchStore};
use crate::utils::HttpClient;

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Missing '{key}' parameter"))
}

/// Watch a topic on the arXiv feed.
#[derive(Debug)]
pub struct WatchTopicHandler;

#[async_trait::async_trait]
impl ToolHandler for WatchTopicHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let query = require_str(&args, "query")?;
        let store = WatchStore::new();
        let added = store.add(query);
        Ok(json!({
            "status": if added { "added" } else { "exists" },
            "query": query,
        }))
    }
}

/// List watched topics with their check state.
#[derive(Debug)]
pub struct ListWatchesHandler;

#[async_trait::async_trait]
impl ToolHandler for ListWatchesHandler {
    async fn execute(&self, _args: Value) -> Result<Value, String> {
        let store = WatchStore::new();
        let watches = store.list();
        let listed: Vec<Value> = watches
            .iter()
            .map(|w| {
                json!({
                    "query": w.query,
                    "created_at": w.created_at,
                    "last_checked": w.last_checked,
                    "last_seen_ids": w.last_seen_ids,
                })
            })
            .collect();
        Ok(json!({ "total": listed.len(), "watches": listed }))
    }
}

/// Hard cap on retraction lookups per check-watches run so a large feed
/// cannot hammer OpenAlex.
const RETRACTION_CHECK_LIMIT: usize = 5;

/// Annotate one serialized paper with `retracted: true` and `retraction_date`
/// when OpenAlex reports its DOI retracted. Negative results and lookup errors
/// stay silent so alerts carry only positive findings.
///
/// `base_url` overrides the OpenAlex works prefix (test seam); None uses the
/// real endpoint.
async fn annotate_if_retracted(
    paper: &mut Value,
    doi: &str,
    client: &HttpClient,
    base_url: Option<&str>,
) -> bool {
    let result = match base_url {
        Some(base) => check_retraction_from(base, client, doi).await,
        None => check_retraction(client, doi).await,
    };
    match result {
        Ok(info) if info.is_retracted => {
            if let Some(obj) = paper.as_object_mut() {
                obj.insert("retracted".to_string(), Value::Bool(true));
                if let Some(date) = info.retraction_date {
                    obj.insert("retraction_date".to_string(), Value::String(date));
                }
            }
            true
        }
        Ok(_) => false,
        Err(e) => {
            tracing::debug!("retraction check failed for {doi}: {e}");
            false
        }
    }
}

/// Check one watched topic (or all of them) for new arXiv papers.
///
/// The store is reloaded from disk per call and each check result is persisted,
/// so repeated calls only report papers that appeared since the previous check.
#[derive(Debug, Default)]
pub struct CheckAlertsHandler {
    /// HTTP client override for tests; None builds a normal client.
    pub client: Option<Arc<HttpClient>>,
}

#[async_trait::async_trait]
impl ToolHandler for CheckAlertsHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(20);

        let client = match &self.client {
            Some(client) => Arc::clone(client),
            None => {
                Arc::new(HttpClient::new().map_err(|e| format!("cannot create HTTP client: {e}"))?)
            }
        };
        let store = WatchStore::new();

        let mut targets = match query.as_deref() {
            Some(q) => vec![store
                .get(q)
                .ok_or_else(|| format!("'{q}' is not watched"))?],
            None => store.list(),
        };
        if targets.is_empty() {
            return Ok(json!({
                "checked": 0,
                "watches": [],
                "note": "No topics are watched yet.",
            }));
        }

        let mut retraction_checks = 0usize;
        let mut retraction_skipped = 0usize;
        let mut reports = Vec::with_capacity(targets.len());
        for watch in &mut targets {
            match check_watch(&client, watch, max_results).await {
                Ok(papers) => {
                    store.update(watch);
                    let mut papers: Vec<Value> = papers
                        .iter()
                        .map(|p| {
                            serde_json::to_value(p)
                                .map_err(|e| format!("cannot serialize paper: {e}"))
                        })
                        .collect::<Result<_, _>>()?;
                    // Best-effort retraction flags on new DOI-bearing hits.
                    for paper in &mut papers {
                        let Some(doi) = paper
                            .get("doi")
                            .and_then(|v| v.as_str())
                            .filter(|d| !d.trim().is_empty())
                            .map(str::to_string)
                        else {
                            continue;
                        };
                        if retraction_checks >= RETRACTION_CHECK_LIMIT {
                            retraction_skipped += 1;
                            continue;
                        }
                        retraction_checks += 1;
                        annotate_if_retracted(paper, &doi, &client, None).await;
                    }
                    reports.push(json!({
                        "query": watch.query,
                        "last_checked": watch.last_checked,
                        "new_papers": papers,
                    }));
                }
                // The watch keeps its previous last_checked, so a failed check
                // is retried by the next call.
                Err(e) => reports.push(json!({
                    "query": watch.query,
                    "error": e.to_string(),
                })),
            }
        }
        let mut response = json!({ "checked": reports.len(), "watches": reports });
        if retraction_skipped > 0 {
            response["retraction_checks_skipped"] = json!(retraction_skipped);
        }
        Ok(response)
    }
}

/// Stop watching a topic.
#[derive(Debug)]
pub struct UnwatchTopicHandler;

#[async_trait::async_trait]
impl ToolHandler for UnwatchTopicHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let query = require_str(&args, "query")?;
        let store = WatchStore::new();
        let removed = store.remove(query);
        Ok(json!({ "removed": removed, "query": query }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes env access: the watches path override is process-wide.
    /// A tokio Mutex because the guard must span await points in async tests.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Point WatchStore::new() at a fresh temp dir for one test.
    async fn scoped_watches_path(name: &str) -> tokio::sync::MutexGuard<'static, ()> {
        let guard = ENV_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!(
            "research-master-watch-handler-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("RESEARCH_MASTER_WATCHES_PATH", &dir);
        guard
    }

    fn sample_feed() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
            <title>arXiv Query Results</title>
            <id>http://arxiv.org/api/query</id>
            <updated>2026-01-02T10:00:00Z</updated>
            <entry>
                <id>http://arxiv.org/abs/2601.00001v1</id>
                <title>Watched Paper One</title>
                <summary>First abstract.</summary>
                <published>2026-01-01T09:00:00Z</published>
                <updated>2026-01-02T10:00:00Z</updated>
                <author><name>A. Author</name></author>
            </entry>
            <entry>
                <id>http://arxiv.org/abs/2601.00002v2</id>
                <title>Watched Paper Two</title>
                <summary>Second abstract.</summary>
                <published>2025-05-01T09:00:00Z</published>
                <updated>2025-06-01T10:00:00Z</updated>
                <author><name>B. Author</name></author>
            </entry>
        </feed>
        "#
    }

    /// A client with no proxy and no rate limiting, so requests always reach
    /// the local mockito server.
    fn test_client() -> Arc<HttpClient> {
        Arc::new(
            HttpClient::with_proxy("research-master-tests", None, None, 0)
                .expect("test HTTP client"),
        )
    }

    #[tokio::test]
    async fn watch_list_unwatch_roundtrip_via_handlers() {
        let _guard = scoped_watches_path("roundtrip").await;

        let added = WatchTopicHandler
            .execute(json!({ "query": "Sparse Attention" }))
            .await
            .unwrap();
        assert_eq!(added["status"], "added");
        assert_eq!(added["query"], "Sparse Attention");

        let duplicate = WatchTopicHandler
            .execute(json!({ "query": "sparse attention" }))
            .await
            .unwrap();
        assert_eq!(duplicate["status"], "exists");

        let listed = ListWatchesHandler.execute(json!({})).await.unwrap();
        assert_eq!(listed["total"], 1);
        assert_eq!(listed["watches"][0]["query"], "Sparse Attention");
        assert_eq!(listed["watches"][0]["last_checked"], 0);

        let removed = UnwatchTopicHandler
            .execute(json!({ "query": "SPARSE ATTENTION" }))
            .await
            .unwrap();
        assert_eq!(removed["removed"], true);

        let listed = ListWatchesHandler.execute(json!({})).await.unwrap();
        assert_eq!(listed["total"], 0);
    }

    #[tokio::test]
    async fn check_alerts_reports_new_papers_once() {
        let guard = scoped_watches_path("check-alerts").await;
        let mut server = mockito::Server::new_async().await;
        std::env::set_var(
            "RESEARCH_MASTER_ARXIV_API_BASE",
            format!("{}/api/query", server.url()),
        );

        let feed_mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/atom+xml")
            .with_body(sample_feed())
            .expect(2)
            .create_async()
            .await;

        WatchTopicHandler
            .execute(json!({ "query": "attention" }))
            .await
            .unwrap();

        let handler = CheckAlertsHandler {
            client: Some(test_client()),
        };

        // First check: a never-checked watch reports every entry as new.
        let first = handler.execute(json!({ "max_results": 5 })).await.unwrap();
        assert_eq!(first["checked"], 1);
        let new_papers = first["watches"][0]["new_papers"]
            .as_array()
            .unwrap_or_else(|| panic!("watches[0] report: {}", first["watches"][0]));
        assert_eq!(new_papers.len(), 2);
        assert_eq!(new_papers[0]["paper_id"], "2601.00001");

        // An unwatched query is rejected instead of silently returning nothing.
        let err = handler
            .execute(json!({ "query": "never watched" }))
            .await
            .unwrap_err();
        assert!(err.contains("not watched"), "got: {err}");

        // Second check: the first call persisted last_checked, so nothing is new.
        let second = handler.execute(json!({})).await.unwrap();
        let new_papers = second["watches"][0]["new_papers"]
            .as_array()
            .expect("new_papers array");
        assert!(new_papers.is_empty());

        let listed = ListWatchesHandler.execute(json!({})).await.unwrap();
        assert!(listed["watches"][0]["last_checked"].as_u64().unwrap() > 0);
        assert_eq!(
            listed["watches"][0]["last_seen_ids"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        feed_mock.assert_async().await;
        // The base URL points at a now-dead server port; drop it for other tests.
        std::env::remove_var("RESEARCH_MASTER_ARXIV_API_BASE");
        drop(guard);
    }

    #[tokio::test]
    async fn annotate_if_retracted_flags_positive_lookups_only() {
        let mut server = mockito::Server::new_async().await;
        let base = format!("{}/works/doi:", server.url());
        let retracted_mock = server
            .mock("GET", "/works/doi:10.1000%2Fbad-paper")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"id":"https://openalex.org/W1","title":"Bad Paper",
                    "is_retracted":true,"retraction_date":"2025-03-01"}"#,
            )
            .expect(1)
            .create_async()
            .await;
        let clean_mock = server
            .mock("GET", "/works/doi:10.1000%2Fgood-paper")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"id":"https://openalex.org/W2","title":"Good Paper",
                    "is_retracted":false,"retraction_date":null}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let client = test_client();

        let mut flagged = json!({
            "paper_id": "2601.00001",
            "title": "Bad Paper",
            "doi": "10.1000/bad-paper"
        });
        assert!(
            annotate_if_retracted(&mut flagged, "10.1000/bad-paper", &client, Some(&base)).await
        );
        assert_eq!(flagged["retracted"], true);
        assert_eq!(flagged["retraction_date"], "2025-03-01");

        // A not-retracted result stays silent.
        let mut clean = json!({
            "paper_id": "2601.00002",
            "title": "Good Paper",
            "doi": "10.1000/good-paper"
        });
        assert!(
            !annotate_if_retracted(&mut clean, "10.1000/good-paper", &client, Some(&base)).await
        );
        assert!(clean.get("retracted").is_none());
        assert!(clean.get("retraction_date").is_none());

        retracted_mock.assert_async().await;
        clean_mock.assert_async().await;
    }

    #[tokio::test]
    async fn missing_required_args_error() {
        let err = WatchTopicHandler.execute(json!({})).await.unwrap_err();
        assert!(err.contains("query"), "got: {err}");

        let err = WatchTopicHandler
            .execute(json!({ "query": "   " }))
            .await
            .unwrap_err();
        assert!(err.contains("query"), "got: {err}");

        let err = UnwatchTopicHandler.execute(json!({})).await.unwrap_err();
        assert!(err.contains("query"), "got: {err}");
    }
}
