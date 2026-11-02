//! Persisted topic watches over the arXiv announcement feed.

use feed_rs::parser;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::{Paper, PaperBuilder, SourceType};
use crate::sources::SourceError;
use crate::utils::{api_retry_config, with_retry, HttpClient};

/// Base URL for arXiv PDFs
const ARXIV_PDF_URL: &str = "https://arxiv.org/pdf";

/// Default arXiv API endpoint; `RESEARCH_MASTER_ARXIV_API_BASE` overrides it
/// so tests can point at a mock server.
const DEFAULT_ARXIV_API_BASE: &str = "http://export.arxiv.org/api/query";

/// A watched topic plus its check state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watch {
    pub query: String,
    /// Unix timestamp (seconds) when the watch was added
    pub created_at: u64,
    /// Unix timestamp (seconds) of the last completed check; 0 means never checked
    #[serde(default)]
    pub last_checked: u64,
    /// arXiv IDs present in the most recent fetch
    #[serde(default)]
    pub last_seen_ids: Vec<String>,
}

/// Watched-topics store backed by one JSONL file with a mutex-protected
/// in-memory cache.
///
/// Lines are `{query, created_at, last_checked, last_seen_ids}`; corrupt lines
/// are skipped on load. Mutations rewrite the file atomically (temp file +
/// rename), mirroring the paper library.
#[derive(Debug, Clone)]
pub struct WatchStore {
    path: PathBuf,
    watches: std::sync::Arc<Mutex<Vec<Watch>>>,
}

impl WatchStore {
    /// Watch store at the default config location.
    ///
    /// Process-wide singleton (same rationale as PaperLibrary::new): sharing
    /// one cached store prevents concurrent MCP tool calls from clobbering
    /// each other's JSONL updates. Tests use from_path instead.
    pub fn new() -> Self {
        let path = default_path();
        static LAST: std::sync::Mutex<Option<(PathBuf, WatchStore)>> = std::sync::Mutex::new(None);
        let mut last = LAST.lock().unwrap_or_else(|p| p.into_inner());
        if last
            .as_ref()
            .map(|(cached, _)| cached != &path)
            .unwrap_or(true)
        {
            *last = Some((path.clone(), Self::from_path(path.clone())));
        }
        last.as_ref().expect("cached store").1.clone()
    }

    /// Watch store at an explicit path; used by tests and for alternate storage.
    pub fn from_path(path: PathBuf) -> Self {
        let watches = load_watches(&path);
        Self {
            path,
            watches: std::sync::Arc::new(Mutex::new(watches)),
        }
    }

    /// Add a topic; false when it is already watched (case-insensitive) or empty.
    pub fn add(&self, query: &str) -> bool {
        let query = query.trim();
        if query.is_empty() {
            return false;
        }
        let query = query.to_string();
        match self.mutate(|watches| {
            if watches.iter().any(|w| w.query.eq_ignore_ascii_case(&query)) {
                return (false, false);
            }
            watches.push(Watch {
                query,
                created_at: now(),
                last_checked: 0,
                last_seen_ids: Vec::new(),
            });
            (true, true)
        }) {
            Ok(added) => added,
            Err(e) => {
                tracing::warn!("watch add failed: {e}");
                false
            }
        }
    }

    /// Remove a topic; false when it was not watched (case-insensitive match).
    pub fn remove(&self, query: &str) -> bool {
        let query = query.trim();
        if query.is_empty() {
            return false;
        }
        match self.mutate(|watches| {
            let before = watches.len();
            watches.retain(|w| !w.query.eq_ignore_ascii_case(query));
            let removed = watches.len() != before;
            (removed, removed)
        }) {
            Ok(removed) => removed,
            Err(e) => {
                tracing::warn!("watch remove failed: {e}");
                false
            }
        }
    }

    /// The watch for a query, or None when it is not watched.
    pub fn get(&self, query: &str) -> Option<Watch> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        self.snapshot()
            .into_iter()
            .find(|w| w.query.eq_ignore_ascii_case(query))
    }

    /// All watches in creation order.
    pub fn list(&self) -> Vec<Watch> {
        self.snapshot()
    }

    /// Persist new check state for an existing watch; false when unknown.
    pub fn update(&self, watch: &Watch) -> bool {
        let watch = watch.clone();
        match self.mutate(|watches| {
            match watches.iter_mut().find(|w| {
                w.query.eq_ignore_ascii_case(&watch.query) && w.created_at == watch.created_at
            }) {
                Some(existing) => {
                    *existing = watch;
                    (true, true)
                }
                None => (false, false),
            }
        }) {
            Ok(updated) => updated,
            Err(e) => {
                tracing::warn!("watch update failed: {e}");
                false
            }
        }
    }

    /// Apply a mutation to a copy of the watches, persist it, then commit to memory.
    ///
    /// Disk changes land before the in-memory cache updates, so a failed write
    /// never leaves the cache ahead of the file.
    fn mutate<T, F>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&mut Vec<Watch>) -> (bool, T),
    {
        let mut guard = self
            .watches
            .lock()
            .map_err(|e| format!("watch lock poisoned: {e}"))?;
        let mut next = guard.clone();
        let (changed, result) = f(&mut next);
        if changed {
            self.persist(&next)?;
            *guard = next;
        }
        Ok(result)
    }

    fn snapshot(&self) -> Vec<Watch> {
        self.watches
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn persist(&self, watches: &[Watch]) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cannot create watch dir: {e}"))?;
        }
        let mut body = String::with_capacity(watches.len() * 256);
        for watch in watches {
            let line =
                serde_json::to_string(watch).map_err(|e| format!("cannot serialize watch: {e}"))?;
            body.push_str(&line);
            body.push('\n');
        }
        let tmp = tmp_path(&self.path);
        let mut file =
            File::create(&tmp).map_err(|e| format!("cannot write watch temp file: {e}"))?;
        file.write_all(body.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("cannot write watch temp file: {e}"))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("cannot replace watch file: {e}"))
    }
}

impl Default for WatchStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Watch-store location; `RESEARCH_MASTER_WATCHES_PATH` overrides the config default.
fn default_path() -> PathBuf {
    // RESEARCH_MASTER_WATCHES_PATH names the directory holding watches.jsonl,
    // mirroring RESEARCH_MASTER_LIBRARY_PATH semantics.
    let base = if let Ok(path) = std::env::var("RESEARCH_MASTER_WATCHES_PATH") {
        if !path.trim().is_empty() {
            return PathBuf::from(path).join("watches.jsonl");
        }
        None
    } else {
        None
    };
    base.unwrap_or_else(|| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("research-master")
            .join("watches.jsonl")
    })
}

fn load_watches(path: &Path) -> Vec<Watch> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };
    bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| match serde_json::from_slice::<Watch>(line) {
            Ok(watch) => Some(watch),
            Err(e) => {
                tracing::warn!("skipping corrupt watch line: {e}");
                None
            }
        })
        .collect()
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// arXiv API base URL; `RESEARCH_MASTER_ARXIV_API_BASE` overrides the default.
fn arxiv_api_base() -> String {
    std::env::var("RESEARCH_MASTER_ARXIV_API_BASE")
        .ok()
        .filter(|base| !base.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_ARXIV_API_BASE.to_string())
}

/// Fetch the arXiv feed for one watch and return the papers updated since the
/// last check (all of them when the watch was never checked).
///
/// Newness is an updated date strictly after `last_checked`. On success the
/// watch's `last_checked` becomes now and `last_seen_ids` the IDs seen in this
/// fetch; on error the watch is untouched so the next call retries it. The
/// caller persists the mutated watch via [`WatchStore::update`].
pub async fn check_watch(
    client: &HttpClient,
    watch: &mut Watch,
    max_results: usize,
) -> Result<Vec<Paper>, SourceError> {
    let max_results = max_results.min(200); // arXiv accepts at most 200 per request
    let url = format!(
        "{}?search_query=all:{}&sortBy=lastUpdatedDate&sortOrder=descending&max_results={}",
        arxiv_api_base(),
        urlencoding::encode(watch.query.trim()),
        max_results,
    );

    let body = with_retry(api_retry_config(), || {
        let url = url.clone();
        async move {
            let response = client
                .get(&url)
                .header("Accept", "application/atom+xml")
                .send()
                .await
                .map_err(|e| {
                    SourceError::Network(format!("Failed to fetch arXiv watch feed: {}", e))
                })?;

            if !response.status().is_success() {
                return Err(SourceError::Api(format!(
                    "arXiv API returned status: {}",
                    response.status()
                )));
            }

            let bytes = response
                .bytes()
                .await
                .map_err(|e| SourceError::Network(format!("Failed to read response: {}", e)))?;

            String::from_utf8(bytes.to_vec())
                .map_err(|e| SourceError::Parse(format!("arXiv feed is not UTF-8: {}", e)))
        }
    })
    .await?;

    let papers = parse_feed(&body)?;
    let (fresh, seen) = fresh_papers(&papers, watch.last_checked);
    // A full page may omit older papers, so retry the same window next time.
    if max_results > 0 && papers.len() < max_results {
        watch.last_checked = now();
    }
    watch.last_seen_ids = seen;
    Ok(fresh)
}

/// Parse an arXiv Atom feed into papers, mapping entries the same way as the
/// arXiv source. Separate from fetching so tests can feed raw XML.
pub fn parse_feed(xml: &str) -> Result<Vec<Paper>, SourceError> {
    let feed = parser::parse(xml.as_bytes())
        .map_err(|e| SourceError::Parse(format!("Failed to parse Atom feed: {}", e)))?;
    feed.entries.iter().map(entry_to_paper).collect()
}

/// Split fetched papers into (new papers, seen IDs); a paper is new when its
/// updated date is strictly after the cutoff.
fn fresh_papers(papers: &[Paper], last_checked: u64) -> (Vec<Paper>, Vec<String>) {
    let mut fresh = Vec::new();
    let mut seen = Vec::new();
    for paper in papers {
        seen.push(paper.paper_id.clone());
        if rfc3339_ts(paper.updated_date.as_deref()) > last_checked {
            fresh.push(paper.clone());
        }
    }
    (fresh, seen)
}

/// Unix seconds for an RFC 3339 timestamp; 0 when missing or unparseable.
fn rfc3339_ts(value: Option<&str>) -> u64 {
    value
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|d| d.timestamp().max(0) as u64)
        .unwrap_or(0)
}

/// Parse one arXiv Atom feed entry into a Paper; mirrors the arXiv source.
fn entry_to_paper(entry: &feed_rs::model::Entry) -> Result<Paper, SourceError> {
    // Extract paper ID from the entry URL, e.g. http://arxiv.org/abs/2301.12345v1
    let paper_id = entry
        .id
        .split("/abs/")
        .last()
        .and_then(|s| s.split('v').next())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| SourceError::Parse("Missing paper ID".to_string()))?
        .to_string();

    let title = entry
        .title
        .as_ref()
        .map(|t| t.content.as_str())
        .unwrap_or("");

    let authors = entry
        .authors
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join("; ");

    let abstract_text = entry
        .summary
        .as_ref()
        .map(|s| s.content.as_str())
        .unwrap_or("");

    let published_date = entry.published.map(|d| d.to_rfc3339());
    let updated_date = entry.updated.map(|d| d.to_rfc3339());

    let categories = entry
        .categories
        .iter()
        .map(|c| c.term.as_str().to_string())
        .collect::<Vec<_>>()
        .join(";");

    Ok(
        PaperBuilder::new(paper_id.clone(), title, entry.id.clone(), SourceType::Arxiv)
            .authors(authors)
            .abstract_text(abstract_text)
            .published_date(published_date.unwrap_or_default())
            .updated_date(updated_date.unwrap_or_default())
            .pdf_url(format!("{}/{}.pdf", ARXIV_PDF_URL, paper_id))
            .categories(categories)
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn add_remove_and_duplicate_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let store = WatchStore::from_path(tmp.path().join("watches.jsonl"));
        assert!(store.list().is_empty());

        assert!(store.add("transformers"));
        // Duplicate detection is case-insensitive and whitespace-trimmed.
        assert!(!store.add("Transformers"));
        assert!(!store.add("  transformers  "));
        assert!(!store.add("   "));
        assert_eq!(store.list().len(), 1);

        assert!(store.add("protein folding"));
        let listed = store.list();
        assert_eq!(listed.len(), 2);
        // Insertion order is the created order.
        assert_eq!(listed[0].query, "transformers");
        assert_eq!(listed[1].query, "protein folding");
        assert!(listed[0].created_at <= listed[1].created_at);
        assert_eq!(listed[0].last_checked, 0);
        assert!(listed[0].last_seen_ids.is_empty());

        assert!(store.remove("PROTEIN FOLDING"));
        assert!(!store.remove("protein folding"));
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn persistence_roundtrip_via_from_path() {
        let tmp = tempfile::tempdir().unwrap();
        // A missing parent directory is created on first write.
        let path = tmp.path().join("nested").join("watches.jsonl");
        let store = WatchStore::from_path(path.clone());
        assert!(store.add("quantum error correction"));

        let mut watch = store.get("Quantum Error Correction").unwrap();
        watch.last_checked = 1_767_268_800;
        watch.last_seen_ids = vec!["2601.00001".to_string(), "2601.00002".to_string()];
        assert!(store.update(&watch));
        assert!(!store.update(&Watch {
            query: "ghost".to_string(),
            created_at: 0,
            last_checked: 0,
            last_seen_ids: Vec::new(),
        }));

        let reloaded = WatchStore::from_path(path);
        let listed = reloaded.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].query, "quantum error correction");
        assert!(listed[0].created_at > 0);
        assert_eq!(listed[0].last_checked, 1_767_268_800);
        assert_eq!(
            listed[0].last_seen_ids,
            vec!["2601.00001".to_string(), "2601.00002".to_string()]
        );
    }

    #[test]
    fn corrupt_lines_are_skipped_and_repaired() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("watches.jsonl");
        let store = WatchStore::from_path(path.clone());
        assert!(store.add("transformers"));
        drop(store);

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{not json").unwrap();
        writeln!(file, "garbage line").unwrap();
        drop(file);

        let store = WatchStore::from_path(path.clone());
        assert_eq!(store.list().len(), 1);

        // The next mutation rewrites the file without the corrupt lines.
        assert!(store.add("crispr"));
        let store = WatchStore::from_path(path);
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn parse_feed_maps_arxiv_entries() {
        let papers = parse_feed(sample_feed()).unwrap();
        assert_eq!(papers.len(), 2);
        assert_eq!(papers[0].paper_id, "2601.00001");
        assert_eq!(papers[0].title, "Watched Paper One");
        assert_eq!(papers[0].authors, "A. Author");
        assert_eq!(papers[0].r#abstract, "First abstract.");
        assert_eq!(papers[0].source, SourceType::Arxiv);
        assert!(papers[0]
            .pdf_url
            .as_deref()
            .unwrap_or_default()
            .ends_with("2601.00001.pdf"));
        // The version suffix is stripped from IDs.
        assert_eq!(papers[1].paper_id, "2601.00002");
    }

    #[test]
    fn parse_feed_rejects_garbage() {
        assert!(parse_feed("this is not xml").is_err());
    }

    #[test]
    fn fresh_papers_filter_is_strictly_newer() {
        let papers = parse_feed(sample_feed()).unwrap();

        // Never checked (cutoff 0): everything counts as new.
        let (fresh, seen) = fresh_papers(&papers, 0);
        assert_eq!(fresh.len(), 2);
        assert_eq!(seen, vec!["2601.00001", "2601.00002"]);

        // A cutoff between the two updated dates keeps only the newer entry.
        let cutoff = rfc3339_ts(Some("2026-01-01T12:00:00Z"));
        let (fresh, _) = fresh_papers(&papers, cutoff);
        let ids: Vec<&str> = fresh.iter().map(|p| p.paper_id.as_str()).collect();
        assert_eq!(ids, vec!["2601.00001"]);

        // Strictly greater: a cutoff equal to a paper's own updated date excludes it.
        let updated = rfc3339_ts(papers[0].updated_date.as_deref());
        assert!(updated > 0);
        let (fresh, _) = fresh_papers(&papers, updated);
        assert!(fresh.is_empty());
    }
}
