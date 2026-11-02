//! Zotero Web API v3 client: save papers to and search a Zotero library.
//!
//! Endpoints (https://www.zotero.org/support/dev/web_api/v3/write_requests):
//!
//! - Create items: `POST /users/{id}/items` (or `/groups/{id}/items`) with a
//!   JSON array of filled item objects; the response's `success` object maps
//!   submitted-array indexes to new item keys, e.g. `{"success": {"0":
//!   "ABCD1234"}, "unchanged": {}, "failed": {}}`. Items that fail validation
//!   come back under `failed` with a per-item message.
//! - Keyword search: `GET /users/{id}/items?q=<query>&limit=<n>&format=json`
//!   (the `q` parameter matches title and creator fields by default).
//!
//! Both send `Zotero-API-Version: 3` and `Authorization: Bearer <key>`.
//!
//! Configuration, read at call time: `ZOTERO_API_KEY` (required; create one
//! at https://www.zotero.org/settings/keys) and exactly one library scope:
//! `ZOTERO_USER_ID` (personal library; takes precedence) or `ZOTERO_GROUP_ID`
//! (group library).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::models::Paper;
use crate::utils::HttpClient;

/// Zotero API v3 base URL; `RESEARCH_MASTER_ZOTERO_API_URL` overrides it
/// (test seam, mirroring `RESEARCH_MASTER_OPENALEX_WORKS_URL`).
pub const ZOTERO_API_URL: &str = "https://api.zotero.org";

/// Result of saving one paper: the key of the created Zotero item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoteroSaveResult {
    pub item_key: String,
}

/// One item from a Zotero search response; only the fields the tools
/// surface are deserialized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoteroItem {
    pub key: String,
    pub data: ZoteroItemData,
}

/// The `data` sub-object of a Zotero item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoteroItemData {
    #[serde(default, rename = "itemType")]
    pub item_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "DOI")]
    pub doi: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub creators: Vec<ZoteroCreator>,
}

/// One creator entry; Zotero uses firstName+lastName, or a single `name`
/// for single-field (typically institutional) names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoteroCreator {
    #[serde(default, rename = "creatorType")]
    pub creator_type: String,
    #[serde(default, rename = "firstName")]
    pub first_name: Option<String>,
    #[serde(default, rename = "lastName")]
    pub last_name: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

impl ZoteroCreator {
    /// Display name from either creator shape.
    pub fn display_name(&self) -> String {
        match (&self.first_name, &self.last_name) {
            (Some(first), Some(last)) => format!("{first} {last}"),
            (Some(first), None) => first.clone(),
            (None, Some(last)) => last.clone(),
            (None, None) => self.name.clone().unwrap_or_default(),
        }
    }
}

/// Which Zotero library requests target, resolved from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LibraryScope {
    User(String),
    Group(String),
}

impl LibraryScope {
    fn from_env() -> Result<Self, String> {
        match (
            non_empty_env("ZOTERO_USER_ID"),
            non_empty_env("ZOTERO_GROUP_ID"),
        ) {
            (Some(id), _) => Ok(Self::User(id)),
            (None, Some(id)) => Ok(Self::Group(id)),
            (None, None) => Err(
                "Zotero library not configured: set ZOTERO_USER_ID (personal library id) \
                 or ZOTERO_GROUP_ID (group id)"
                    .to_string(),
            ),
        }
    }

    fn path(&self) -> String {
        match self {
            Self::User(id) => format!("/users/{id}"),
            Self::Group(id) => format!("/groups/{id}"),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::User(_) => "user",
            Self::Group(_) => "group",
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn require_api_key() -> Result<String, String> {
    non_empty_env("ZOTERO_API_KEY").ok_or_else(|| {
        "ZOTERO_API_KEY not set. Create an API key at https://www.zotero.org/settings/keys"
            .to_string()
    })
}

/// Zotero Web API v3 client. Configuration is read per call so tests and
/// long-running servers pick up env changes without a restart.
#[derive(Debug, Clone)]
pub struct ZoteroClient {
    base_url: String,
    http: HttpClient,
}

impl ZoteroClient {
    /// Shared production client, retaining its rate limiter across requests.
    pub fn shared() -> &'static ZoteroClient {
        static CLIENT: std::sync::OnceLock<ZoteroClient> = std::sync::OnceLock::new();
        CLIENT.get_or_init(ZoteroClient::default)
    }

    /// Client against the production API, honoring the base-URL env override.
    pub fn new() -> Result<Self, String> {
        let base_url = non_empty_env("RESEARCH_MASTER_ZOTERO_API_URL")
            .unwrap_or_else(|| ZOTERO_API_URL.to_string());
        let http = HttpClient::new().map_err(|e| e.to_string())?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        })
    }

    /// Client against an explicit base URL (test seam for mockito).
    pub fn with_base_url(base_url: String, http: HttpClient) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        }
    }

    /// "user" or "group" for the currently configured library.
    pub fn library_kind(&self) -> Result<&'static str, String> {
        LibraryScope::from_env().map(|scope| scope.kind())
    }

    /// Save one paper as a Zotero journalArticle item; returns its item key.
    pub async fn save_paper(&self, paper: &Paper) -> Result<ZoteroSaveResult, String> {
        let key = require_api_key()?;
        let scope = LibraryScope::from_env()?;
        let url = format!("{}{}/items", self.base_url, scope.path());
        // The write protocol takes a JSON array; one item in, `success["0"]`
        // carries the new key back out.
        let body = json!([item_json(paper)]);
        let response = self
            .http
            .post(&url)
            .header("Zotero-API-Version", "3")
            .bearer_auth(key.as_str())
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Zotero save request failed: {e}"))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!(
                "Zotero save failed with status {status}: {}",
                body_snippet(&text).replace(&key, "<redacted>")
            ));
        }
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| format!("cannot parse Zotero save response: {e}"))?;
        match parsed
            .get("success")
            .and_then(|s| s.get("0"))
            .and_then(Value::as_str)
        {
            Some(item_key) => Ok(ZoteroSaveResult {
                item_key: item_key.to_string(),
            }),
            None => {
                let failed = parsed.get("failed").cloned().unwrap_or(Value::Null);
                Err(format!(
                    "Zotero accepted the request but created no item (failed: {})",
                    failed.to_string().replace(&key, "<redacted>")
                ))
            }
        }
    }

    /// Keyword search over the configured library's items, best relevance first.
    pub async fn search_items(&self, query: &str, limit: usize) -> Result<Vec<ZoteroItem>, String> {
        let key = require_api_key()?;
        let scope = LibraryScope::from_env()?;
        let url = format!(
            "{}{}/items?q={}&limit={}&format=json",
            self.base_url,
            scope.path(),
            urlencoding::encode(query),
            limit
        );
        let response = self
            .http
            .get(&url)
            .header("Zotero-API-Version", "3")
            .bearer_auth(key.as_str())
            .send()
            .await
            .map_err(|e| format!("Zotero search request failed: {e}"))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!(
                "Zotero search failed with status {status}: {}",
                body_snippet(&text).replace(&key, "<redacted>")
            ));
        }
        serde_json::from_str(&text).map_err(|e| format!("cannot parse Zotero search response: {e}"))
    }
}

impl Default for ZoteroClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default Zotero client")
    }
}

/// Map a Paper to one Zotero journalArticle item. Empty and absent fields are
/// omitted: the write API expects strings, not nulls.
fn item_json(paper: &Paper) -> Value {
    let mut item = Map::new();
    item.insert("itemType".to_string(), json!("journalArticle"));
    item.insert("title".to_string(), json!(paper.title));
    let creators: Vec<Value> = paper
        .authors
        .split(';')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(creator_json)
        .collect();
    item.insert("creators".to_string(), json!(creators));
    let mut put = |field: &str, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            item.insert(field.to_string(), json!(v));
        }
    };
    put("abstractNote", Some(paper.r#abstract.clone()));
    put("DOI", paper.doi.clone());
    put("url", Some(paper.url.clone()));
    put("date", paper.published_date.clone());
    if let Some(venue) = paper
        .extra
        .as_ref()
        .and_then(|extra| extra.get("venue"))
        .and_then(Value::as_str)
    {
        put("publicationTitle", Some(venue.to_string()));
    }
    let tags: Vec<Value> = paper
        .keywords
        .as_deref()
        .map(|k| {
            k.split(';')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(|t| json!({ "tag": t }))
                .collect()
        })
        .unwrap_or_default();
    if !tags.is_empty() {
        item.insert("tags".to_string(), json!(tags));
    }
    Value::Object(item)
}

/// Split one author name into a Zotero creator. "Family, Given" and
/// "Given ... Family" both map to the two-field form; a single token becomes
/// a single-field creator (`name`), the documented shape Zotero validates.
fn creator_json(name: &str) -> Value {
    if let Some((family, given)) = name.split_once(',') {
        return json!({
            "creatorType": "author",
            "lastName": family.trim(),
            "firstName": given.trim(),
        });
    }
    match name.rsplit_once(' ') {
        Some((given, family)) => json!({
            "creatorType": "author",
            "lastName": family.trim(),
            "firstName": given.trim(),
        }),
        None => json!({
            "creatorType": "author",
            "name": name.trim(),
        }),
    }
}

/// First 200 characters of a response body for error messages.
fn body_snippet(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.len() <= 200 {
        return trimmed.to_string();
    }
    let mut cut = 200;
    while !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}...", &trimmed[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes Zotero env access. A tokio Mutex because the guard must
    /// span await points in async tests (same pattern as handlers_library).
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    const ZOTERO_ENV_VARS: [&str; 4] = [
        "ZOTERO_API_KEY",
        "ZOTERO_USER_ID",
        "ZOTERO_GROUP_ID",
        "RESEARCH_MASTER_ZOTERO_API_URL",
    ];

    /// Ambient proxy vars would route mockito-bound requests through a proxy;
    /// clear them while the env guard is held (same trick as the evidence
    /// module's test client).
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

    fn paper() -> Paper {
        let mut paper = Paper::new(
            "2401.00001".to_string(),
            "Sparse Attention Survey".to_string(),
            "https://arxiv.org/abs/2401.00001".to_string(),
            crate::models::SourceType::Arxiv,
        );
        paper.authors = "A. Author; B. Author".to_string();
        paper.r#abstract = "A survey of sparse attention methods.".to_string();
        paper.doi = Some("10.1000/sparse".to_string());
        paper.published_date = Some("2024-01-01".to_string());
        paper.keywords = Some("attention;transformers".to_string());
        paper.extra = Some(
            [(
                "venue".to_string(),
                Value::String("Journal of Surveys".to_string()),
            )]
            .into_iter()
            .collect(),
        );
        paper
    }

    fn test_client(base: String) -> ZoteroClient {
        ZoteroClient::with_base_url(
            base,
            HttpClient::without_rate_limit("research-master-tests").expect("test HTTP client"),
        )
    }

    #[test]
    fn creators_comma_form_splits_family_given() {
        let creator = creator_json("Vaswani, Ashish");
        assert_eq!(creator["lastName"], "Vaswani");
        assert_eq!(creator["firstName"], "Ashish");
        assert_eq!(creator["creatorType"], "author");
    }

    #[test]
    fn creators_multiword_given_uses_last_token_as_family() {
        let creator = creator_json("Maria de la Cruz");
        assert_eq!(creator["lastName"], "Cruz");
        assert_eq!(creator["firstName"], "Maria de la");
    }

    #[test]
    fn creators_single_token_becomes_single_field_name() {
        let creator = creator_json("Madonna");
        assert_eq!(creator["name"], "Madonna");
        assert!(creator.get("lastName").is_none());
    }

    #[tokio::test]
    async fn save_posts_item_array_and_parses_key() {
        let _guard = ENV_LOCK.lock().await;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/users/12345/items")
            .match_header("Zotero-API-Version", "3")
            .match_header("Authorization", "Bearer test-key")
            .match_body(mockito::Matcher::JsonString(
                r#"[{"itemType":"journalArticle",
                    "title":"Sparse Attention Survey",
                    "creators":[
                        {"creatorType":"author","lastName":"Author","firstName":"A."},
                        {"creatorType":"author","lastName":"Author","firstName":"B."}
                    ],
                    "abstractNote":"A survey of sparse attention methods.",
                    "DOI":"10.1000/sparse",
                    "url":"https://arxiv.org/abs/2401.00001",
                    "date":"2024-01-01",
                    "publicationTitle":"Journal of Surveys",
                    "tags":[{"tag":"attention"},{"tag":"transformers"}]}]"#
                    .to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"success":{"0":"ABCD1234"},"unchanged":{},"failed":{}}"#)
            .expect(1)
            .create_async()
            .await;
        let saved_proxies = clear_proxy_env();
        std::env::set_var("ZOTERO_API_KEY", "test-key");
        std::env::set_var("ZOTERO_USER_ID", "12345");
        for var in ZOTERO_ENV_VARS.iter().skip(3) {
            std::env::remove_var(var);
        }

        let client = test_client(server.url());
        let result = client.save_paper(&paper()).await.expect("save");

        assert_eq!(result.item_key, "ABCD1234");
        mock.assert_async().await;
        restore_proxy_env(saved_proxies);
        for var in ZOTERO_ENV_VARS {
            std::env::remove_var(var);
        }
    }

    #[tokio::test]
    async fn save_targets_group_library_when_group_id_set() {
        let _guard = ENV_LOCK.lock().await;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/groups/99/items")
            .with_status(200)
            .with_body(r#"{"success":{"0":"GRP00001"},"unchanged":{},"failed":{}}"#)
            .expect(1)
            .create_async()
            .await;
        let saved_proxies = clear_proxy_env();
        std::env::set_var("ZOTERO_API_KEY", "test-key");
        std::env::set_var("ZOTERO_GROUP_ID", "99");
        std::env::remove_var("ZOTERO_USER_ID");
        for var in ZOTERO_ENV_VARS.iter().skip(3) {
            std::env::remove_var(var);
        }

        let client = test_client(server.url());
        let result = client.save_paper(&paper()).await.expect("save");

        assert_eq!(result.item_key, "GRP00001");
        mock.assert_async().await;
        restore_proxy_env(saved_proxies);
        for var in ZOTERO_ENV_VARS {
            std::env::remove_var(var);
        }
    }

    #[tokio::test]
    async fn save_error_includes_status_and_body_snippet() {
        let _guard = ENV_LOCK.lock().await;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/users/12345/items")
            .with_status(403)
            .with_body("Forbidden")
            .expect(1)
            .create_async()
            .await;
        let saved_proxies = clear_proxy_env();
        std::env::set_var("ZOTERO_API_KEY", "bad-key");
        std::env::set_var("ZOTERO_USER_ID", "12345");
        for var in ZOTERO_ENV_VARS.iter().skip(3) {
            std::env::remove_var(var);
        }

        let client = test_client(server.url());
        let err = client
            .save_paper(&paper())
            .await
            .expect_err("403 must fail");

        assert!(err.contains("403"), "got: {err}");
        assert!(err.contains("Forbidden"), "got: {err}");
        mock.assert_async().await;
        restore_proxy_env(saved_proxies);
        for var in ZOTERO_ENV_VARS {
            std::env::remove_var(var);
        }
    }

    #[tokio::test]
    async fn save_without_api_key_names_key_url() {
        let _guard = ENV_LOCK.lock().await;
        let saved_proxies = clear_proxy_env();
        for var in ZOTERO_ENV_VARS {
            std::env::remove_var(var);
        }

        let client = test_client("http://127.0.0.1:1".to_string());
        let err = client.save_paper(&paper()).await.expect_err("no key");

        assert!(err.contains("ZOTERO_API_KEY"), "got: {err}");
        assert!(
            err.contains("https://www.zotero.org/settings/keys"),
            "got: {err}"
        );
        restore_proxy_env(saved_proxies);
    }

    #[tokio::test]
    async fn save_without_library_scope_names_both_env_vars() {
        let _guard = ENV_LOCK.lock().await;
        let saved_proxies = clear_proxy_env();
        for var in ZOTERO_ENV_VARS {
            std::env::remove_var(var);
        }
        std::env::set_var("ZOTERO_API_KEY", "test-key");

        let client = test_client("http://127.0.0.1:1".to_string());
        let err = client.save_paper(&paper()).await.expect_err("no scope");

        assert!(err.contains("ZOTERO_USER_ID"), "got: {err}");
        assert!(err.contains("ZOTERO_GROUP_ID"), "got: {err}");
        restore_proxy_env(saved_proxies);
        for var in ZOTERO_ENV_VARS {
            std::env::remove_var(var);
        }
    }

    #[tokio::test]
    async fn search_parses_items_both_creator_shapes() {
        let _guard = ENV_LOCK.lock().await;
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/users/12345/items?q=attention&limit=2&format=json")
            .match_header("Zotero-API-Version", "3")
            .match_header("Authorization", "Bearer test-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[{"key":"ABCD1234","version":120,
                     "library":{"type":"user","id":12345,"name":"me"},
                     "data":{"key":"ABCD1234","version":120,
                         "itemType":"journalArticle",
                         "title":"Attention Is All You Need",
                         "creators":[{"creatorType":"author","firstName":"Ashish","lastName":"Vaswani"}],
                         "DOI":"10.1234/attn","date":"2017-06-12",
                         "tags":[{"tag":"transformers"}]}},
                    {"key":"EFGH5678","version":7,
                     "data":{"key":"EFGH5678","version":7,"itemType":"book",
                         "title":"A Report by an Institution",
                         "creators":[{"creatorType":"author","name":"World Health Organization"}],
                         "date":"2020"}}]"#,
            )
            .expect(1)
            .create_async()
            .await;
        let saved_proxies = clear_proxy_env();
        std::env::set_var("ZOTERO_API_KEY", "test-key");
        std::env::set_var("ZOTERO_USER_ID", "12345");
        for var in ZOTERO_ENV_VARS.iter().skip(3) {
            std::env::remove_var(var);
        }

        let client = test_client(server.url());
        let items = client.search_items("attention", 2).await.expect("search");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].key, "ABCD1234");
        assert_eq!(items[0].data.title, "Attention Is All You Need");
        assert_eq!(items[0].data.item_type, "journalArticle");
        assert_eq!(items[0].data.doi, "10.1234/attn");
        assert_eq!(items[0].data.date, "2017-06-12");
        assert_eq!(items[0].data.creators[0].display_name(), "Ashish Vaswani");
        assert_eq!(items[1].key, "EFGH5678");
        assert_eq!(
            items[1].data.creators[0].display_name(),
            "World Health Organization"
        );
        mock.assert_async().await;
        restore_proxy_env(saved_proxies);
        for var in ZOTERO_ENV_VARS {
            std::env::remove_var(var);
        }
    }

    #[tokio::test]
    async fn library_kind_reports_user_then_group() {
        let _guard = ENV_LOCK.lock().await;
        let saved_proxies = clear_proxy_env();
        for var in ZOTERO_ENV_VARS {
            std::env::remove_var(var);
        }
        std::env::set_var("ZOTERO_API_KEY", "test-key");

        std::env::set_var("ZOTERO_USER_ID", "12345");
        let client = test_client("http://127.0.0.1:1".to_string());
        assert_eq!(client.library_kind().unwrap(), "user");

        std::env::set_var("ZOTERO_GROUP_ID", "99");
        assert_eq!(client.library_kind().unwrap(), "user");

        std::env::remove_var("ZOTERO_USER_ID");
        assert_eq!(client.library_kind().unwrap(), "group");

        restore_proxy_env(saved_proxies);
        for var in ZOTERO_ENV_VARS {
            std::env::remove_var(var);
        }
    }
}
