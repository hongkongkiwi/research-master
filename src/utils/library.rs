//! Local saved-papers library with JSONL persistence, collections, tags, and retrieval.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::Paper;

/// Whether [`PaperLibrary::add_paper`] inserted a new entry or refreshed an existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddedOrUpdated {
    Added,
    Updated,
}

/// Retraction update to apply after a lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetractionWrite {
    Overwrite(String, Option<String>),
    PreserveConfirmedOnFailure,
}

/// Result of a scoped retraction update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetractionWriteOutcome {
    Applied {
        status: String,
        date: Option<String>,
    },
    SkippedDoiMismatch,
    SkippedScopeNotFound,
}

/// A saved paper plus its library metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryEntry {
    /// Unix timestamp (seconds) of when the paper was first saved
    pub added_at: u64,
    pub collection: Option<String>,
    pub tags: Vec<String>,
    /// Free-form notes; searched alongside title and abstract
    #[serde(default)]
    pub notes: String,
    /// Retraction check outcome for the paper's DOI: `"retracted"`,
    /// `"not_retracted"`, or `"unknown"`; None when never checked
    #[serde(default)]
    pub retraction_status: Option<String>,
    /// When the paper was retracted, as reported by OpenAlex
    #[serde(default)]
    pub retraction_date: Option<String>,
    /// RFC3339 UTC time of the last retraction check
    #[serde(default)]
    pub retraction_checked_at: Option<String>,
    pub paper: Paper,
}

/// One retrieval hit from [`PaperLibrary::ask`].
#[derive(Debug, Clone, Serialize)]
pub struct AskMatch {
    pub entry: LibraryEntry,
    pub score: f64,
    /// Query terms responsible for this match
    pub matched_terms: Vec<String>,
}

/// Answer to a library question: the most relevant saved papers and why they matched.
///
/// This is retrieval only. No language model is involved and no answer text is generated.
#[derive(Debug, Clone, Serialize)]
pub struct AskAnswer {
    pub question: String,
    pub matched: Vec<AskMatch>,
}

/// Saved-papers store backed by one JSONL file with a mutex-protected in-memory cache.
///
/// Lines are `{added_at, collection, tags, notes, retraction fields, paper}`; corrupt
/// lines are skipped on load. Mutations rewrite the file atomically (temp file + rename).
#[derive(Debug, Clone)]
pub struct PaperLibrary {
    path: PathBuf,
    entries: std::sync::Arc<Mutex<Vec<LibraryEntry>>>,
}

/// Jaro-Winkler similarity a document token needs to match a query term.
const FUZZY_THRESHOLD: f64 = 0.9;

impl PaperLibrary {
    /// Library at the default config location.
    ///
    /// Calls with the same resolved path share one in-memory store: two
    /// concurrent MCP tool calls would otherwise run competing
    /// read-modify-write cycles on the JSONL file and lose each other's
    /// entries. The cache is keyed by path so tests that override
    /// RESEARCH_MASTER_LIBRARY_PATH keep working. Tests that need isolated
    /// stores use [`PaperLibrary::from_path`].
    pub fn new() -> Self {
        let path = default_path();
        static LAST: std::sync::Mutex<Option<(PathBuf, PaperLibrary)>> =
            std::sync::Mutex::new(None);
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

    /// Library at an explicit path; used by tests and for alternate storage locations.
    pub fn from_path(path: PathBuf) -> Self {
        let entries = load_entries(&path);
        Self {
            path,
            entries: std::sync::Arc::new(Mutex::new(entries)),
        }
    }

    /// Save a paper, or refresh collection and tags when it is already saved.
    ///
    /// A paper may live in several collections: re-adding one that exists in a
    /// different collection adds another entry there, while re-adding within
    /// the same collection updates in place. Notes are preserved on update;
    /// use [`PaperLibrary::set_notes`] to change them.
    pub fn add_paper(
        &self,
        paper: Paper,
        collection: Option<String>,
        tags: Vec<String>,
    ) -> Result<AddedOrUpdated, String> {
        let has_id = !paper.paper_id.trim().is_empty();
        let has_doi = matches!(paper.doi.as_deref(), Some(d) if !d.trim().is_empty());
        if !has_id && !has_doi {
            return Err("paper has neither paper_id nor doi; cannot save it".to_string());
        }

        self.mutate(|entries| {
            let same_collection = |e: &LibraryEntry| e.collection == collection;
            match entries
                .iter_mut()
                .find(|e| same_paper(&e.paper, &paper) && same_collection(e))
            {
                Some(existing) => {
                    existing.paper = paper;
                    existing.tags = tags;
                    (true, AddedOrUpdated::Updated)
                }
                None => {
                    entries.push(LibraryEntry {
                        added_at: now(),
                        collection,
                        tags,
                        notes: String::new(),
                        retraction_status: None,
                        retraction_date: None,
                        retraction_checked_at: None,
                        paper,
                    });
                    (true, AddedOrUpdated::Added)
                }
            }
        })
    }

    /// Set free-form notes on every matching entry in scope. Returns false when none match.
    pub fn set_notes(
        &self,
        id: &str,
        collection: Option<&str>,
        notes: &str,
    ) -> Result<bool, String> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(false);
        }
        let id_lower = id.to_lowercase();
        let notes = notes.to_string();
        self.mutate(|entries| {
            let mut changed = false;
            for entry in entries
                .iter_mut()
                .filter(|e| in_scope(e, collection) && matches_id(e, id, &id_lower))
            {
                entry.notes = notes.clone();
                changed = true;
            }
            (changed, changed)
        })
    }

    /// Set retraction status on every matching entry in scope. Returns false when none match.
    pub fn set_retraction(
        &self,
        id: &str,
        collection: Option<&str>,
        status: &str,
        retraction_date: Option<&str>,
        checked_at: &str,
    ) -> Result<bool, String> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(false);
        }
        let id_lower = id.to_lowercase();
        let status = status.to_string();
        let retraction_date = retraction_date.map(str::to_string);
        let checked_at = checked_at.to_string();
        self.mutate(|entries| {
            let mut changed = false;
            for entry in entries
                .iter_mut()
                .filter(|e| in_scope(e, collection) && matches_id(e, id, &id_lower))
            {
                entry.retraction_status = Some(status.clone());
                entry.retraction_date = retraction_date.clone();
                entry.retraction_checked_at = Some(checked_at.clone());
                changed = true;
            }
            (changed, changed)
        })
    }

    /// Update one exact collection membership if its DOI still matches the lookup.
    /// `None` selects only the uncollected membership.
    pub fn set_retraction_scoped(
        &self,
        id: &str,
        collection: Option<&str>,
        expected_doi: Option<&str>,
        status: RetractionWrite,
        checked_at: String,
    ) -> Result<RetractionWriteOutcome, String> {
        let id = id.trim();
        let id_lower = id.to_lowercase();
        self.mutate(|entries| {
            let mut candidates = entries
                .iter_mut()
                .filter(|entry| {
                    !id.is_empty()
                        && entry.collection.as_deref() == collection
                        && matches_id(entry, id, &id_lower)
                })
                .peekable();
            if candidates.peek().is_none() {
                return (false, RetractionWriteOutcome::SkippedScopeNotFound);
            }
            let entry = candidates.find(|entry| {
                expected_doi.is_none_or(|doi| {
                    entry
                        .paper
                        .doi
                        .as_deref()
                        .map(str::trim)
                        .is_some_and(|current| current.eq_ignore_ascii_case(doi.trim()))
                })
            });
            let Some(entry) = entry else {
                return (false, RetractionWriteOutcome::SkippedDoiMismatch);
            };
            match status {
                RetractionWrite::Overwrite(status, date) => {
                    entry.retraction_status = Some(status);
                    entry.retraction_date = date;
                }
                RetractionWrite::PreserveConfirmedOnFailure => {
                    if entry.retraction_status.as_deref() != Some("retracted") {
                        entry.retraction_status = Some("unknown".to_string());
                        entry.retraction_date = None;
                    }
                }
            }
            entry.retraction_checked_at = Some(checked_at);
            (
                true,
                RetractionWriteOutcome::Applied {
                    status: entry.retraction_status.clone().expect("status set above"),
                    date: entry.retraction_date.clone(),
                },
            )
        })
    }

    /// Remove by paper_id or DOI, optionally scoped to one collection.
    pub fn remove_paper(&self, id: &str, collection: Option<&str>) -> bool {
        let id = id.trim();
        if id.is_empty() {
            return false;
        }
        let id_lower = id.to_lowercase();
        match self.mutate(|entries| {
            let before = entries.len();
            entries.retain(|e| !(in_scope(e, collection) && matches_id(e, id, &id_lower)));
            let removed = entries.len() != before;
            (removed, removed)
        }) {
            Ok(removed) => removed,
            Err(e) => {
                tracing::warn!("library remove failed: {e}");
                false
            }
        }
    }

    /// Saved papers, newest first, optionally scoped to one collection.
    pub fn list(&self, collection: Option<&str>) -> Vec<LibraryEntry> {
        let mut entries: Vec<LibraryEntry> = self
            .snapshot()
            .into_iter()
            .filter(|e| in_scope(e, collection))
            .collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.added_at));
        entries
    }

    /// Keyword search over title, abstract, notes, and tags.
    ///
    /// Terms match exactly or fuzzily (Jaro-Winkler); entries are ranked TF-IDF-ish
    /// and returned with their scores, best first. With `retracted_only`, only
    /// entries whose retraction status is `"retracted"` are considered.
    pub fn search(
        &self,
        query: &str,
        collection: Option<&str>,
        retracted_only: bool,
        limit: usize,
    ) -> Vec<(LibraryEntry, f64)> {
        let terms = tokenize(query);
        if terms.is_empty() || limit == 0 {
            return Vec::new();
        }
        let corpus: Vec<LibraryEntry> = self
            .snapshot()
            .into_iter()
            .filter(|e| in_scope(e, collection))
            .filter(|e| !retracted_only || e.retraction_status.as_deref() == Some("retracted"))
            .collect();
        let idf = idf_map(&corpus, &terms);
        let mut scored: Vec<(LibraryEntry, f64)> = corpus
            .into_iter()
            .map(|entry| {
                let (score, _) = score_entry(&entry, &terms, &idf);
                (entry, score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    /// Retrieve the most relevant saved papers for a question.
    ///
    /// Honest name on purpose: this ranks saved papers by term overlap and reports
    /// which query terms matched. No LLM call happens here and no answer is written.
    pub fn ask(&self, question: &str, collection: Option<&str>, top_k: usize) -> AskAnswer {
        let terms = tokenize(question);
        let mut matched = Vec::new();
        if !terms.is_empty() && top_k > 0 {
            let corpus: Vec<LibraryEntry> = self
                .snapshot()
                .into_iter()
                .filter(|e| in_scope(e, collection))
                .collect();
            let idf = idf_map(&corpus, &terms);
            for entry in &corpus {
                let (score, matched_terms) = score_entry(entry, &terms, &idf);
                if score > 0.0 {
                    matched.push(AskMatch {
                        entry: entry.clone(),
                        score,
                        matched_terms,
                    });
                }
            }
            matched.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            matched.truncate(top_k);
        }
        AskAnswer {
            question: question.to_string(),
            matched,
        }
    }

    /// Apply a mutation to a copy of the entries, persist it, then commit to memory.
    ///
    /// Disk changes land before the in-memory cache updates, so a failed write never
    /// leaves the cache ahead of the file.
    fn mutate<T, F>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&mut Vec<LibraryEntry>) -> (bool, T),
    {
        let mut guard = self
            .entries
            .lock()
            .map_err(|e| format!("library lock poisoned: {e}"))?;
        let mut next = guard.clone();
        let (changed, result) = f(&mut next);
        if changed {
            self.persist(&next)?;
            *guard = next;
        }
        Ok(result)
    }

    fn snapshot(&self) -> Vec<LibraryEntry> {
        self.entries
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn persist(&self, entries: &[LibraryEntry]) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cannot create library dir: {e}"))?;
        }
        let mut body = String::with_capacity(entries.len() * 512);
        for entry in entries {
            let line =
                serde_json::to_string(entry).map_err(|e| format!("cannot serialize entry: {e}"))?;
            body.push_str(&line);
            body.push('\n');
        }
        let tmp = tmp_path(&self.path);
        let mut file =
            File::create(&tmp).map_err(|e| format!("cannot write library temp file: {e}"))?;
        file.write_all(body.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("cannot write library temp file: {e}"))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("cannot replace library file: {e}"))
    }
}

impl Default for PaperLibrary {
    fn default() -> Self {
        Self::new()
    }
}

/// Library location; `RESEARCH_MASTER_LIBRARY_PATH` overrides the config default.
fn default_path() -> PathBuf {
    // RESEARCH_MASTER_LIBRARY_PATH names the directory holding library.jsonl,
    // mirroring CacheConfig.directory semantics.
    let base = if let Ok(path) = std::env::var("RESEARCH_MASTER_LIBRARY_PATH") {
        if !path.trim().is_empty() {
            return PathBuf::from(path).join("library.jsonl");
        }
        None
    } else {
        None
    };
    base.unwrap_or_else(|| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("research-master")
            .join("library.jsonl")
    })
}

fn load_entries(path: &Path) -> Vec<LibraryEntry> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };
    bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| match serde_json::from_slice::<LibraryEntry>(line) {
            Ok(entry) => Some(entry),
            Err(e) => {
                tracing::warn!("skipping corrupt library line: {e}");
                None
            }
        })
        .collect()
}

/// Match DOI when present, or source-local ID when neither paper has a DOI.
fn same_paper(a: &Paper, b: &Paper) -> bool {
    let doi_a = a.doi.as_deref().map(str::trim).filter(|d| !d.is_empty());
    let doi_b = b.doi.as_deref().map(str::trim).filter(|d| !d.is_empty());
    match (doi_a, doi_b) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
        (None, None) => {
            a.source == b.source
                && !a.paper_id.trim().is_empty()
                && a.paper_id.trim() == b.paper_id.trim()
        }
        _ => false,
    }
}

fn matches_id(entry: &LibraryEntry, id: &str, id_lower: &str) -> bool {
    if entry.paper.paper_id.trim() == id {
        return true;
    }
    match &entry.paper.doi {
        Some(doi) => doi.trim().to_lowercase() == id_lower,
        None => false,
    }
}

fn in_scope(entry: &LibraryEntry, collection: Option<&str>) -> bool {
    match collection {
        None => true,
        Some(c) => entry.collection.as_deref() == Some(c),
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

fn doc_tokens(entry: &LibraryEntry) -> Vec<String> {
    let mut text = String::new();
    text.push_str(&entry.paper.title);
    text.push(' ');
    text.push_str(&entry.paper.r#abstract);
    text.push(' ');
    text.push_str(&entry.notes);
    for tag in &entry.tags {
        text.push(' ');
        text.push_str(tag);
    }
    tokenize(&text)
}

fn token_matches(doc_token: &str, term: &str) -> bool {
    if doc_token == term {
        return true;
    }
    // Fuzzy matching on short tokens is noise (rna vs dna); require some length.
    doc_token.len() >= 4
        && term.len() >= 4
        && strsim::jaro_winkler(doc_token, term) >= FUZZY_THRESHOLD
}

fn term_count(entry: &LibraryEntry, term: &str) -> usize {
    doc_tokens(entry)
        .iter()
        .filter(|tok| token_matches(tok, term))
        .count()
}

/// Smoothed IDF per query term over the corpus being searched.
fn idf_map(corpus: &[LibraryEntry], terms: &[String]) -> HashMap<String, f64> {
    let n = corpus.len().max(1) as f64;
    terms
        .iter()
        .map(|term| {
            let df = corpus.iter().filter(|e| term_count(e, term) > 0).count() as f64;
            (term.clone(), ((n + 1.0) / (df + 1.0)).ln() + 1.0)
        })
        .collect()
}

fn score_entry(
    entry: &LibraryEntry,
    terms: &[String],
    idf: &HashMap<String, f64>,
) -> (f64, Vec<String>) {
    let tokens = doc_tokens(entry);
    let total = tokens.len().max(1) as f64;
    let mut score = 0.0;
    let mut matched = Vec::new();
    for term in terms {
        let hits = tokens.iter().filter(|tok| token_matches(tok, term)).count() as f64;
        if hits > 0.0 {
            if let Some(weight) = idf.get(term) {
                score += hits / total * weight;
            }
            matched.push(term.clone());
        }
    }
    (score, matched)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "research-master-library-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("library.jsonl")
    }

    fn paper(id: &str, title: &str, doi: Option<&str>) -> Paper {
        let mut paper = Paper::new(
            id.to_string(),
            title.to_string(),
            "https://example.com".to_string(),
            crate::models::SourceType::Arxiv,
        );
        paper.doi = doi.map(str::to_string);
        paper
    }

    #[test]
    fn add_list_remove_roundtrip() {
        let path = temp_path("roundtrip");
        let library = PaperLibrary::from_path(path.clone());
        assert!(library.list(None).is_empty());

        assert_eq!(
            library
                .add_paper(
                    paper("a1", "Alpha Paper", None),
                    Some("ml".into()),
                    vec!["tag1".into()]
                )
                .unwrap(),
            AddedOrUpdated::Added
        );
        library
            .add_paper(paper("b2", "Beta Paper", None), None, Vec::new())
            .unwrap();

        assert_eq!(library.list(None).len(), 2);
        assert_eq!(library.list(Some("ml")).len(), 1);
        assert_eq!(library.list(Some("ml"))[0].paper.paper_id, "a1");

        assert!(library.remove_paper("a1", None));
        assert!(!library.remove_paper("a1", None));
        assert_eq!(library.list(None).len(), 1);

        // State survives a reload from disk.
        let reloaded = PaperLibrary::from_path(path);
        assert_eq!(reloaded.list(None).len(), 1);
        assert_eq!(reloaded.list(None)[0].paper.paper_id, "b2");
    }

    #[test]
    fn re_add_updates_collection_and_tags() {
        let library = PaperLibrary::from_path(temp_path("re-add"));
        library
            .add_paper(
                paper("a1", "Alpha", None),
                Some("old".into()),
                vec!["x".into()],
            )
            .unwrap();
        library.set_notes("a1", Some("old"), "read later").unwrap();

        // Re-add within the same collection updates in place.
        let outcome = library
            .add_paper(
                paper("a1", "Alpha (v2)", None),
                Some("old".into()),
                vec!["y".into()],
            )
            .unwrap();
        assert_eq!(outcome, AddedOrUpdated::Updated);
        let old_entries = library.list(Some("old"));
        assert_eq!(old_entries.len(), 1);
        assert_eq!(old_entries[0].paper.title, "Alpha (v2)");
        assert_eq!(old_entries[0].tags, vec!["y".to_string()]);
        assert_eq!(old_entries[0].notes, "read later");

        // A different collection gains its own membership.
        let outcome = library
            .add_paper(
                paper("a1", "Alpha (v2)", None),
                Some("new".into()),
                Vec::new(),
            )
            .unwrap();
        assert_eq!(outcome, AddedOrUpdated::Added);
        assert_eq!(library.list(None).len(), 2);
        assert_eq!(library.list(Some("old")).len(), 1);
        assert_eq!(library.list(Some("new")).len(), 1);
    }

    #[test]
    fn doi_dedupe_is_case_insensitive() {
        let library = PaperLibrary::from_path(temp_path("doi"));
        library
            .add_paper(paper("a1", "Alpha", Some("10.1000/AbC")), None, Vec::new())
            .unwrap();
        let outcome = library
            .add_paper(paper("b2", "Beta", Some("10.1000/abc")), None, Vec::new())
            .unwrap();
        assert_eq!(outcome, AddedOrUpdated::Updated);
        assert_eq!(library.list(None).len(), 1);
    }

    #[test]
    fn remove_by_doi() {
        let library = PaperLibrary::from_path(temp_path("remove-doi"));
        library
            .add_paper(paper("a1", "Alpha", Some("10.1000/x")), None, Vec::new())
            .unwrap();
        assert!(library.remove_paper("10.1000/X", None));
        assert!(library.list(None).is_empty());
    }

    #[test]
    fn remove_scoped_to_collection() {
        let library = PaperLibrary::from_path(temp_path("remove-scope"));
        library
            .add_paper(paper("a1", "Alpha", None), Some("ml".into()), Vec::new())
            .unwrap();
        // Same paper saved in a different collection stays untouched.
        library
            .add_paper(
                paper("a1", "Alpha", None),
                Some("reading".into()),
                Vec::new(),
            )
            .unwrap();
        assert!(library.remove_paper("a1", Some("ml")));
        let entries = library.list(None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].collection.as_deref(), Some("reading"));
    }

    #[test]
    fn search_ranks_matching_document_first() {
        let library = PaperLibrary::from_path(temp_path("search"));
        let mut attn = paper("attn", "Attention Is All You Need", None);
        attn.r#abstract =
            "The dominant transformer architecture for sequence modeling.".to_string();
        library.add_paper(attn, None, Vec::new()).unwrap();
        library
            .add_paper(
                paper("crispr", "CRISPR Gene Editing", None),
                None,
                Vec::new(),
            )
            .unwrap();

        let results = library.search("transformer", None, false, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.paper.paper_id, "attn");
        assert!(results[0].1 > 0.0);

        // Fuzzy: plural form still finds the singular-only document.
        let fuzzy = library.search("transformers", None, false, 10);
        assert_eq!(fuzzy.len(), 1);
        assert_eq!(fuzzy[0].0.paper.paper_id, "attn");

        let limited = library.search("the", None, false, 1);
        assert!(limited.len() <= 1);
        assert!(library.search("zzzznotfound", None, false, 10).is_empty());
    }

    #[test]
    fn search_matches_notes_and_tags() {
        let library = PaperLibrary::from_path(temp_path("search-fields"));
        library
            .add_paper(
                paper("a1", "Untitled Work", None),
                None,
                vec!["survey-material".into()],
            )
            .unwrap();
        library
            .set_notes("a1", None, "relevant to scaling laws")
            .unwrap();

        let by_note = library.search("scaling", None, false, 10);
        assert_eq!(by_note.len(), 1);
        let by_tag = library.search("survey", None, false, 10);
        assert_eq!(by_tag.len(), 1);
    }

    #[test]
    fn corrupt_lines_are_skipped_and_repaired() {
        let path = temp_path("corrupt");
        let library = PaperLibrary::from_path(path.clone());
        library
            .add_paper(paper("a1", "Alpha", None), None, Vec::new())
            .unwrap();
        drop(library);

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{not json").unwrap();
        writeln!(file, "garbage line").unwrap();
        drop(file);

        let library = PaperLibrary::from_path(path.clone());
        assert_eq!(library.list(None).len(), 1);

        library
            .add_paper(paper("b2", "Beta", None), None, Vec::new())
            .unwrap();
        let library = PaperLibrary::from_path(path);
        assert_eq!(library.list(None).len(), 2);
    }

    #[test]
    fn retraction_fields_persist_and_default_on_old_json() {
        let path = temp_path("retraction");
        let library = PaperLibrary::from_path(path.clone());
        library
            .add_paper(paper("a1", "Alpha", Some("10.1000/x")), None, Vec::new())
            .unwrap();
        library
            .set_retraction(
                "a1",
                None,
                "retracted",
                Some("2024-06-01"),
                "2026-11-02T00:00:00+00:00",
            )
            .unwrap();
        let entry = &library.list(None)[0];
        assert_eq!(entry.retraction_status.as_deref(), Some("retracted"));
        assert_eq!(entry.retraction_date.as_deref(), Some("2024-06-01"));

        // State survives a reload from disk.
        let reloaded = PaperLibrary::from_path(path);
        let entry = &reloaded.list(None)[0];
        assert_eq!(entry.retraction_status.as_deref(), Some("retracted"));
        assert!(entry.retraction_checked_at.is_some());

        // An old JSONL line without retraction fields loads with None defaults.
        let stored_paper = serde_json::to_string(&paper("b2", "Beta", None)).unwrap();
        let old_line = format!(
            r#"{{"added_at":1700000000,"collection":null,"tags":[],"notes":"","paper":{stored_paper}}}"#
        );
        let entry: LibraryEntry = serde_json::from_str(&old_line).unwrap();
        assert_eq!(entry.retraction_status, None);
        assert_eq!(entry.retraction_date, None);
        assert_eq!(entry.retraction_checked_at, None);
    }

    #[test]
    fn scoped_retraction_rejects_wrong_doi_without_changes() {
        let path = temp_path("retraction-mismatch");
        let library = PaperLibrary::from_path(path.clone());
        library
            .add_paper(
                paper("a1", "Alpha", Some("10.1000/current")),
                None,
                Vec::new(),
            )
            .unwrap();
        let before = fs::read(&path).unwrap();
        assert_eq!(
            library
                .set_retraction_scoped(
                    "a1",
                    None,
                    Some("10.1000/old"),
                    RetractionWrite::Overwrite("retracted".into(), None),
                    "now".into()
                )
                .unwrap(),
            RetractionWriteOutcome::SkippedDoiMismatch
        );
        assert_eq!(fs::read(path).unwrap(), before);
        assert_eq!(library.list(None)[0].retraction_status, None);
    }

    #[test]
    fn scoped_retraction_uncollected_leaves_collection_untouched() {
        let library = PaperLibrary::from_path(temp_path("retraction-scope"));
        for collection in [None, Some("reading".to_string())] {
            library
                .add_paper(
                    paper("a1", "Alpha", Some("10.1000/x")),
                    collection,
                    Vec::new(),
                )
                .unwrap();
        }
        assert!(matches!(
            library
                .set_retraction_scoped(
                    "a1",
                    None,
                    Some("10.1000/x"),
                    RetractionWrite::Overwrite("retracted".into(), None),
                    "now".into()
                )
                .unwrap(),
            RetractionWriteOutcome::Applied { .. }
        ));
        let entries = library.list(None);
        assert_eq!(
            entries
                .iter()
                .find(|e| e.collection.is_none())
                .unwrap()
                .retraction_status
                .as_deref(),
            Some("retracted")
        );
        assert_eq!(
            entries
                .iter()
                .find(|e| e.collection.is_some())
                .unwrap()
                .retraction_status,
            None
        );
    }

    #[test]
    fn scoped_retraction_failure_preserves_confirmed_retraction() {
        let path = temp_path("retraction-preserve");
        let library = PaperLibrary::from_path(path.clone());
        library
            .add_paper(paper("a1", "Alpha", Some("10.1000/x")), None, Vec::new())
            .unwrap();
        library
            .set_retraction("a1", None, "retracted", Some("2024-06-01"), "before")
            .unwrap();
        assert_eq!(
            library
                .set_retraction_scoped(
                    "a1",
                    None,
                    Some("10.1000/x"),
                    RetractionWrite::PreserveConfirmedOnFailure,
                    "after".into()
                )
                .unwrap(),
            RetractionWriteOutcome::Applied {
                status: "retracted".into(),
                date: Some("2024-06-01".into())
            }
        );
        let entry = &PaperLibrary::from_path(path).list(None)[0];
        assert_eq!(entry.retraction_status.as_deref(), Some("retracted"));
        assert_eq!(entry.retraction_date.as_deref(), Some("2024-06-01"));
        assert_eq!(entry.retraction_checked_at.as_deref(), Some("after"));
    }

    #[test]
    fn search_can_restrict_to_retracted_entries() {
        let library = PaperLibrary::from_path(temp_path("retracted-only"));
        library
            .add_paper(
                paper("bad", "Retracted Study", Some("10.1000/bad")),
                None,
                Vec::new(),
            )
            .unwrap();
        library
            .add_paper(
                paper("ok", "Solid Replication Study", None),
                None,
                Vec::new(),
            )
            .unwrap();
        library
            .set_retraction(
                "10.1000/bad",
                None,
                "retracted",
                None,
                "2026-11-02T00:00:00+00:00",
            )
            .unwrap();

        let all = library.search("study", None, false, 10);
        assert_eq!(all.len(), 2);
        let only_retracted = library.search("study", None, true, 10);
        assert_eq!(only_retracted.len(), 1);
        assert_eq!(only_retracted[0].0.paper.paper_id, "bad");
    }

    #[test]
    fn ask_reports_matched_terms() {
        let library = PaperLibrary::from_path(temp_path("ask"));
        library
            .add_paper(paper("p1", "Protein Folding", None), None, Vec::new())
            .unwrap();
        library
            .set_notes("p1", None, "protein folding drives misfolding disease")
            .unwrap();
        library
            .add_paper(paper("p2", "Coastal Erosion", None), None, Vec::new())
            .unwrap();

        let answer = library.ask("how does protein folding work?", None, 5);
        assert_eq!(answer.question, "how does protein folding work?");
        assert_eq!(answer.matched.len(), 1);
        assert_eq!(answer.matched[0].entry.paper.paper_id, "p1");
        assert!(answer.matched[0]
            .matched_terms
            .contains(&"protein".to_string()));
        assert!(answer.matched[0]
            .matched_terms
            .contains(&"folding".to_string()));

        assert!(library.ask("zzqq xyzzy", None, 5).matched.is_empty());
        assert!(library
            .ask("protein", Some("nonexistent"), 5)
            .matched
            .is_empty());
    }

    #[test]
    fn reject_paper_without_identifier() {
        let library = PaperLibrary::from_path(temp_path("no-id"));
        let mut nameless = Paper::new(
            String::new(),
            "No Id".to_string(),
            String::new(),
            crate::models::SourceType::Other("local".to_string()),
        );
        nameless.doi = Some(String::new());
        assert!(library.add_paper(nameless, None, Vec::new()).is_err());
    }
}
