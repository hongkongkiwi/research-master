//! MCP tool handlers for Zotero: save papers to and search a Zotero library.
//!
//! Configuration lives in the zotero module: `ZOTERO_API_KEY` plus
//! `ZOTERO_USER_ID` or `ZOTERO_GROUP_ID`; `RESEARCH_MASTER_ZOTERO_API_URL`
//! overrides the API base URL (test seam).

use std::sync::Arc;

use serde_json::{json, Value};

use super::tools::ToolHandler;
use crate::models::Paper;
use crate::utils::zotero::ZoteroClient;

/// Resolve a paper by id through one source: `get_by_id`, then `get_by_doi`
/// for DOI-shaped ids, then a one-result search. Detection mirrors
/// GetPaperHandler via the shared `auto_detect_source`.
async fn resolve_paper(
    sources: &Arc<Vec<Arc<dyn crate::sources::Source>>>,
    paper_id: &str,
    source_override: Option<&str>,
) -> Result<Paper, String> {
    let source = match source_override.filter(|s| !s.trim().is_empty()) {
        Some(source_id) => sources
            .iter()
            .find(|s| s.id() == source_id)
            .cloned()
            .ok_or_else(|| format!("Source '{source_id}' not found"))?,
        None => crate::mcp::unified_tools::auto_detect_source(sources, paper_id)?,
    };

    if let Ok(paper) = source.get_by_id(paper_id).await {
        return Ok(paper);
    }
    if paper_id.starts_with("10.") {
        if let Ok(paper) = source.get_by_doi(paper_id).await {
            return Ok(paper);
        }
    }
    let search_query = crate::models::SearchQuery::new(paper_id).max_results(1);
    let response = source
        .search(&search_query)
        .await
        .map_err(|e| e.to_string())?;
    response
        .papers
        .into_iter()
        .next()
        .ok_or_else(|| format!("Paper '{paper_id}' not found in {}", source.id()))
}

/// Save a paper to the configured Zotero library.
#[derive(Debug)]
pub struct ZoteroSaveHandler {
    pub sources: Arc<Vec<Arc<dyn crate::sources::Source>>>,
}

#[async_trait::async_trait]
impl ToolHandler for ZoteroSaveHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let paper_id = args
            .get("paper_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or("Missing 'paper_id' parameter")?;
        let source_override = args.get("source").and_then(|v| v.as_str());

        let paper = resolve_paper(&self.sources, paper_id, source_override).await?;
        let client = ZoteroClient::new()?;
        let saved = client.save_paper(&paper).await?;
        Ok(json!({
            "saved": true,
            "zotero_item_key": saved.item_key,
            "title": paper.title,
            "doi": paper.doi,
        }))
    }
}

/// Keyword search over the configured Zotero library.
#[derive(Debug)]
pub struct ZoteroSearchHandler;

#[async_trait::async_trait]
impl ToolHandler for ZoteroSearchHandler {
    async fn execute(&self, args: Value) -> Result<Value, String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or("Missing 'query' parameter")?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(10)
            .min(50);

        self.search(query, limit, ZoteroClient::shared()).await
    }
}

impl ZoteroSearchHandler {
    async fn search(
        &self,
        query: &str,
        limit: usize,
        client: &ZoteroClient,
    ) -> Result<Value, String> {
        let items = client.search_items(query, limit).await?;
        let library = client.library_kind()?;
        let items: Result<Vec<Value>, String> = items
            .into_iter()
            .map(|item| {
                serde_json::to_value(&item).map_err(|e| format!("cannot serialize item: {e}"))
            })
            .collect();
        let items = items?;
        let count = items.len();
        Ok(json!({
            "items": items,
            "count": count,
            "library": library,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{Source, SourceCapabilities, SourceError};

    use crate::test_support::{EnvSnapshot, ENV_LOCK};

    const ZOTERO_ENV_VARS: [&str; 4] = [
        "ZOTERO_API_KEY",
        "ZOTERO_USER_ID",
        "ZOTERO_GROUP_ID",
        "RESEARCH_MASTER_ZOTERO_API_URL",
    ];

    /// A source that resolves one known paper by id; enough to exercise the
    /// handler's get_by_id path without network access.
    #[derive(Debug)]
    struct FixedSource {
        paper: Paper,
    }

    impl FixedSource {
        fn new(paper: Paper) -> Self {
            Self { paper }
        }
    }

    #[async_trait::async_trait]
    impl Source for FixedSource {
        fn id(&self) -> &str {
            "semantic"
        }

        fn name(&self) -> &str {
            "Semantic Scholar"
        }

        fn capabilities(&self) -> SourceCapabilities {
            SourceCapabilities::SEARCH | SourceCapabilities::DOI_LOOKUP
        }

        async fn get_by_id(&self, id: &str) -> Result<Paper, SourceError> {
            if id == self.paper.paper_id {
                Ok(self.paper.clone())
            } else {
                Err(SourceError::NotFound(format!("no paper '{id}'")))
            }
        }
    }

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

    fn sample_paper() -> Paper {
        let mut paper = Paper::new(
            "attn-1".to_string(),
            "Attention Is All You Need".to_string(),
            "https://arxiv.org/abs/1706.03762".to_string(),
            crate::models::SourceType::SemanticScholar,
        );
        paper.doi = Some("10.1234/attn".to_string());
        paper
    }

    fn zotero_env(server_url: String) {
        std::env::set_var("ZOTERO_API_KEY", "test-key");
        std::env::set_var("ZOTERO_USER_ID", "12345");
        std::env::remove_var("ZOTERO_GROUP_ID");
        std::env::set_var("RESEARCH_MASTER_ZOTERO_API_URL", server_url);
    }

    fn clear_zotero_env() {
        for var in ZOTERO_ENV_VARS {
            std::env::remove_var(var);
        }
    }

    #[tokio::test]
    async fn save_handler_resolves_paper_and_reports_key() {
        let _guard = ENV_LOCK.lock().await;
        let _env = EnvSnapshot::new(&[
            "ZOTERO_API_KEY",
            "ZOTERO_USER_ID",
            "ZOTERO_GROUP_ID",
            "RESEARCH_MASTER_ZOTERO_API_URL",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]);
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/users/12345/items")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"success":{"0":"KEY98765"},"unchanged":{},"failed":{}}"#)
            .expect(1)
            .create_async()
            .await;
        let saved_proxies = clear_proxy_env();
        zotero_env(server.url());

        let handler = ZoteroSaveHandler {
            sources: Arc::new(vec![Arc::new(FixedSource::new(sample_paper()))]),
        };
        let result = handler
            .execute(json!({ "paper_id": "attn-1" }))
            .await
            .expect("save");

        assert_eq!(result["saved"], true);
        assert_eq!(result["zotero_item_key"], "KEY98765");
        assert_eq!(result["title"], "Attention Is All You Need");
        assert_eq!(result["doi"], "10.1234/attn");
        mock.assert_async().await;
        restore_proxy_env(saved_proxies);
        clear_zotero_env();
    }

    #[tokio::test]
    async fn save_handler_resolution_failure_is_honest() {
        let _guard = ENV_LOCK.lock().await;
        let _env = EnvSnapshot::new(&[
            "ZOTERO_API_KEY",
            "ZOTERO_USER_ID",
            "ZOTERO_GROUP_ID",
            "RESEARCH_MASTER_ZOTERO_API_URL",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]);
        // No sources at all: auto-detection fails before any Zotero call,
        // so no env setup is needed beyond the lock.
        let handler = ZoteroSaveHandler {
            sources: Arc::new(Vec::new()),
        };
        let err = handler
            .execute(json!({ "paper_id": "attn-1" }))
            .await
            .expect_err("no sources must fail");

        assert!(err.contains("Could not auto-detect source"), "got: {err}");
    }

    #[tokio::test]
    async fn search_handler_returns_items_and_library_kind() {
        let _guard = ENV_LOCK.lock().await;
        let _env = EnvSnapshot::new(&[
            "ZOTERO_API_KEY",
            "ZOTERO_USER_ID",
            "ZOTERO_GROUP_ID",
            "RESEARCH_MASTER_ZOTERO_API_URL",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]);
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/users/12345/items?q=attention&limit=10&format=json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[{"key":"ABCD1234","version":1,
                     "data":{"key":"ABCD1234","version":1,
                         "itemType":"journalArticle",
                         "title":"Attention Is All You Need",
                         "creators":[{"creatorType":"author","firstName":"Ashish","lastName":"Vaswani"}],
                         "DOI":"10.1234/attn","date":"2017-06-12"}}]"#,
            )
            .expect(1)
            .create_async()
            .await;
        let saved_proxies = clear_proxy_env();
        zotero_env(server.url());

        let client = ZoteroClient::new().unwrap();
        let result = ZoteroSearchHandler
            .search("attention", 10, &client)
            .await
            .expect("search");

        assert_eq!(result["count"], 1);
        assert_eq!(result["library"], "user");
        assert_eq!(result["items"][0]["key"], "ABCD1234");
        assert_eq!(
            result["items"][0]["data"]["creators"][0]["firstName"],
            "Ashish"
        );
        mock.assert_async().await;
        restore_proxy_env(saved_proxies);
        clear_zotero_env();
    }

    #[tokio::test]
    async fn search_handler_requires_query() {
        let err = ZoteroSearchHandler.execute(json!({})).await.unwrap_err();
        assert!(err.contains("query"), "got: {err}");
    }

    #[tokio::test]
    async fn save_handler_requires_paper_id() {
        let handler = ZoteroSaveHandler {
            sources: Arc::new(Vec::new()),
        };
        let err = handler.execute(json!({})).await.unwrap_err();
        assert!(err.contains("paper_id"), "got: {err}");
    }
}
