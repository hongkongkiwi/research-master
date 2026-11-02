//! Systematic-review screening sessions with PRISMA flow counts.
//!
//! A session is a fixed candidate list plus per-paper decisions; the store
//! persists sessions as JSONL and computes PRISMA-style counts on demand.
//! Also hosts abstract statistics extraction used to surface effect sizes,
//! confidence intervals, p-values and sample sizes to screening callers.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

/// Decision values accepted by [`ScreeningStore::record_decisions`].
pub const DECISION_VALUES: [&str; 3] = ["include", "exclude", "uncertain"];

/// One include/exclude/uncertain call on a candidate paper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub paper_id: String,
    /// One of `"include"`, `"exclude"`, `"uncertain"`
    pub decision: String,
    /// Free-form reason; for excludes it becomes the PRISMA exclusion category
    pub reason: String,
    /// RFC3339 UTC time of the latest decision
    pub decided_at: String,
}

/// One screening run over a fixed candidate list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub name: String,
    /// RFC3339 UTC creation time
    pub created_at: String,
    /// Search string behind the candidate list; empty for explicit lists
    pub query: String,
    pub inclusion_criteria: String,
    pub exclusion_criteria: String,
    /// Candidate papers as provided: paper_id/title/doi/abstract/published_date/paper_type/categories/source
    pub candidates: Vec<serde_json::Value>,
    /// Records dropped by deduplication before screening started
    pub duplicates_removed: usize,
    /// One decision per decided candidate; re-deciding replaces in place
    pub decisions: Vec<Decision>,
}

/// PRISMA-style flow counts for one session.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrismaSummary {
    /// Unique candidate records after deduplication
    pub identified: usize,
    pub duplicates_removed: usize,
    /// Candidates with a recorded decision
    pub screened: usize,
    pub included: usize,
    pub excluded: usize,
    /// Exclusion reason (lowercased, trimmed) -> count; blank reasons count as "unspecified"
    pub excluded_by_reason: HashMap<String, usize>,
    pub uncertain: usize,
    /// Candidates still awaiting a decision
    pub remaining: usize,
}

/// Counts from recording a batch of decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordOutcome {
    pub recorded: usize,
    /// Entries whose paper_id matched no candidate in the session
    pub ignored_unknown: usize,
}

/// Screening-session store backed by one JSONL file with a mutex-protected
/// in-memory cache.
///
/// Lines are `{id, name, ..., candidates, decisions}`; corrupt lines are
/// skipped on load. Mutations rewrite the file atomically (temp file +
/// rename), mirroring [`crate::utils::PaperLibrary`].
#[derive(Debug, Clone)]
pub struct ScreeningStore {
    path: PathBuf,
    sessions: Arc<Mutex<Vec<SessionRecord>>>,
}

/// Per-process session id sequence; combined with epoch millis for uniqueness.
static NEXT_SESSION_SEQ: AtomicU64 = AtomicU64::new(1);

impl ScreeningStore {
    /// Store at the default config location.
    ///
    /// Calls with the same resolved path share one in-memory store: two
    /// concurrent MCP tool calls would otherwise run competing
    /// read-modify-write cycles on the JSONL file and lose each other's
    /// decisions. The cache is keyed by path so tests that override
    /// RESEARCH_MASTER_SCREENINGS_PATH keep working. Tests that need isolated
    /// stores use [`ScreeningStore::from_path`].
    pub fn new() -> Self {
        let path = default_path();
        static LAST: std::sync::Mutex<Option<(PathBuf, ScreeningStore)>> =
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

    /// Store at an explicit path; used by tests and for alternate storage locations.
    pub fn from_path(path: PathBuf) -> Self {
        let sessions = load_sessions(&path);
        Self {
            path,
            sessions: Arc::new(Mutex::new(sessions)),
        }
    }

    /// Create a session over the given candidates and return its id.
    pub fn create_session(
        &self,
        name: &str,
        query: &str,
        inclusion_criteria: &str,
        exclusion_criteria: &str,
        candidates: Vec<serde_json::Value>,
        duplicates_removed: usize,
    ) -> Result<String, String> {
        let id = new_session_id();
        self.mutate(|sessions| {
            sessions.push(SessionRecord {
                id: id.clone(),
                name: name.to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                query: query.to_string(),
                inclusion_criteria: inclusion_criteria.to_string(),
                exclusion_criteria: exclusion_criteria.to_string(),
                candidates,
                duplicates_removed,
                decisions: Vec::new(),
            });
            (true, id.clone())
        })
    }

    /// Record decisions against a session's candidates.
    ///
    /// A decision value outside [`DECISION_VALUES`] is an error; a paper_id
    /// that matches no candidate is ignored and reported. Re-deciding a
    /// candidate replaces its earlier decision so a paper never counts twice.
    pub fn record_decisions(
        &self,
        session_id: &str,
        decisions: Vec<(String, String, String)>,
    ) -> Result<RecordOutcome, String> {
        for (paper_id, decision, _) in &decisions {
            if !DECISION_VALUES.contains(&decision.as_str()) {
                return Err(format!(
                    "Invalid decision '{decision}' for '{paper_id}': use include, exclude, or uncertain"
                ));
            }
        }
        let decided_at = chrono::Utc::now().to_rfc3339();
        self.mutate(|sessions| {
            let Some(session) = sessions.iter_mut().find(|s| s.id == session_id) else {
                return (false, None);
            };
            let known: HashSet<&str> = session.candidates.iter().filter_map(candidate_id).collect();
            let mut outcome = RecordOutcome {
                recorded: 0,
                ignored_unknown: 0,
            };
            for (paper_id, decision, reason) in decisions {
                if !known.contains(paper_id.as_str()) {
                    outcome.ignored_unknown += 1;
                    continue;
                }
                match session
                    .decisions
                    .iter_mut()
                    .find(|d| d.paper_id == paper_id)
                {
                    Some(existing) => {
                        existing.decision = decision;
                        existing.reason = reason;
                        existing.decided_at = decided_at.clone();
                    }
                    None => session.decisions.push(Decision {
                        paper_id,
                        decision,
                        reason,
                        decided_at: decided_at.clone(),
                    }),
                }
                outcome.recorded += 1;
            }
            (true, Some(outcome))
        })?
        .ok_or_else(|| format!("no screening session with id '{session_id}'"))
    }

    /// Undecided candidates in candidate order, up to `limit`.
    pub fn undecided(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, String> {
        let sessions = self.snapshot();
        let Some(session) = sessions.iter().find(|s| s.id == session_id) else {
            return Err(format!("no screening session with id '{session_id}'"));
        };
        let decided: HashSet<&str> = session
            .decisions
            .iter()
            .map(|d| d.paper_id.as_str())
            .collect();
        Ok(session
            .candidates
            .iter()
            .filter(|c| candidate_id(c).is_some_and(|id| !decided.contains(id)))
            .take(limit)
            .cloned()
            .collect())
    }

    /// PRISMA flow counts for a session; `None` when the id is unknown.
    pub fn summary(&self, session_id: &str) -> Result<Option<PrismaSummary>, String> {
        let sessions = self.snapshot();
        let Some(session) = sessions.iter().find(|s| s.id == session_id) else {
            return Ok(None);
        };
        let mut included = 0;
        let mut excluded = 0;
        let mut uncertain = 0;
        let mut by_reason: HashMap<String, usize> = HashMap::new();
        for decision in &session.decisions {
            match decision.decision.as_str() {
                "include" => included += 1,
                "exclude" => {
                    excluded += 1;
                    let key = normalize_reason(&decision.reason);
                    *by_reason.entry(key).or_insert(0) += 1;
                }
                // record_decisions rejects anything else
                _ => uncertain += 1,
            }
        }
        Ok(Some(PrismaSummary {
            identified: session.candidates.len(),
            duplicates_removed: session.duplicates_removed,
            screened: session.decisions.len(),
            included,
            excluded,
            excluded_by_reason: by_reason,
            uncertain,
            remaining: session
                .candidates
                .len()
                .saturating_sub(session.decisions.len()),
        }))
    }

    /// Apply a mutation to a copy of the sessions, persist it, then commit to memory.
    ///
    /// Disk changes land before the in-memory cache updates, so a failed write
    /// never leaves the cache ahead of the file.
    fn mutate<T, F>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&mut Vec<SessionRecord>) -> (bool, T),
    {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|e| format!("screening store lock poisoned: {e}"))?;
        let mut next = guard.clone();
        let (changed, result) = f(&mut next);
        if changed {
            self.persist(&next)?;
            *guard = next;
        }
        Ok(result)
    }

    fn snapshot(&self) -> Vec<SessionRecord> {
        self.sessions
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn persist(&self, sessions: &[SessionRecord]) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cannot create screenings dir: {e}"))?;
        }
        let mut body = String::with_capacity(sessions.len() * 1024);
        for session in sessions {
            let line = serde_json::to_string(session)
                .map_err(|e| format!("cannot serialize session: {e}"))?;
            body.push_str(&line);
            body.push('\n');
        }
        let tmp = tmp_path(&self.path);
        let mut file =
            File::create(&tmp).map_err(|e| format!("cannot write screenings temp file: {e}"))?;
        file.write_all(body.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("cannot write screenings temp file: {e}"))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("cannot replace screenings file: {e}"))
    }
}

impl Default for ScreeningStore {
    fn default() -> Self {
        Self::new()
    }
}

/// The non-empty `paper_id` string of a candidate object.
fn candidate_id(candidate: &serde_json::Value) -> Option<&str> {
    candidate
        .get("paper_id")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// Exclusion-reason bucket key: lowercased and trimmed, blanks become "unspecified".
fn normalize_reason(reason: &str) -> String {
    let trimmed = reason.trim().to_lowercase();
    if trimmed.is_empty() {
        "unspecified".to_string()
    } else {
        trimmed
    }
}

/// Store location; `RESEARCH_MASTER_SCREENINGS_PATH` overrides the config default.
fn default_path() -> PathBuf {
    // RESEARCH_MASTER_SCREENINGS_PATH names the directory holding screenings.jsonl,
    // mirroring RESEARCH_MASTER_LIBRARY_PATH semantics.
    let base = if let Ok(path) = std::env::var("RESEARCH_MASTER_SCREENINGS_PATH") {
        if !path.trim().is_empty() {
            return PathBuf::from(path).join("screenings.jsonl");
        }
        None
    } else {
        None
    };
    base.unwrap_or_else(|| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("research-master")
            .join("screenings.jsonl")
    })
}

/// Session id from UTC date, epoch millis and a process-wide counter.
fn new_session_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seq = NEXT_SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    let date = chrono::Utc::now().format("%Y%m%d");
    format!("s-{date}-{millis}-{seq}")
}

fn load_sessions(path: &Path) -> Vec<SessionRecord> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };
    bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| match serde_json::from_slice::<SessionRecord>(line) {
            Ok(session) => Some(session),
            Err(e) => {
                tracing::warn!("skipping corrupt screening line: {e}");
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

/// Compiled statistics patterns, built once and reused per call.
struct StatsPatterns {
    effect_size: Regex,
    correlation: Regex,
    ratio: Regex,
    confidence_interval: Regex,
    p_value: Regex,
    sample_size: Regex,
}

fn stats_patterns() -> &'static StatsPatterns {
    static PATTERNS: OnceLock<StatsPatterns> = OnceLock::new();
    PATTERNS.get_or_init(|| StatsPatterns {
        // Cohen's d / Hedges' g: "d = 0.45", "g = -0.32"
        effect_size: Regex::new(r"\b[dg]\s*[=:]\s*-?\d+(?:\.\d+)?").unwrap(),
        // Pearson r: "r = .32", "r = 0.32" (leading-digit dot allowed)
        correlation: Regex::new(r"\br\s*[=:]\s*-?\.?\d+(?:\.\d+)?").unwrap(),
        // Odds/risk/hazard ratio: "OR = 1.23", "RR=2.1", "HR 0.85"; decimal required so years ("OR 2019") do not match
        ratio: Regex::new(r"\b(?:OR|RR|HR)\s*[=:]?\s*\d+\.\d+").unwrap(),
        // 95% CI: "95% CI [1.1, 2.3]", "95% CI: 1.1-2.3", "95% CI 1.1 to 2.3";
        // only "]" closes, so an unbracketed CI before ")" stops at the number
        confidence_interval: Regex::new(
            r"95\s*%\s*(?:CI|CrI)\s*:?\s*[\[(]?[-+]?\d+(?:\.\d+)?\s*(?:,|to|–|-)\s*[-+]?\d+(?:\.\d+)?\]?",
        )
        .unwrap(),
        // p-value: "p = .01", "p < 0.001", "p=0.04"
        p_value: Regex::new(r"\b[pP]\s*(?:=|<|≤)\s*\.?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?").unwrap(),
        // Sample size: "N = 234", "n = 45"
        sample_size: Regex::new(r"\b[nN]\s*=\s*(?:\d{1,3}(?:,\d{3})+|\d+)").unwrap(),
    })
}

/// Extract headline statistics (effect sizes, ratios, CIs, p-values, sample
/// sizes) from abstract-style text. Returns each full matched string,
/// deduplicated in source order, capped at 20.
pub fn extract_statistics(text: &str) -> Vec<String> {
    // Typeset abstracts use U+2212 for minus; normalize so the numeric
    // patterns match negative values.
    let text = text.replace('\u{2212}', "-");
    let patterns = stats_patterns();
    let mut matches = Vec::new();
    for pattern in [
        &patterns.effect_size,
        &patterns.correlation,
        &patterns.ratio,
        &patterns.confidence_interval,
        &patterns.p_value,
        &patterns.sample_size,
    ] {
        for m in pattern.find_iter(&text) {
            if std::ptr::eq(pattern, &patterns.p_value)
                || std::ptr::eq(pattern, &patterns.sample_size)
            {
                let rest = &text[m.end()..];
                let mut chars = rest.chars();
                if chars.next().is_some_and(|c| {
                    c.is_alphanumeric()
                        || ((c == ',' || c == '.')
                            && chars.next().is_some_and(|next| next.is_ascii_digit()))
                }) {
                    continue;
                }
            }
            matches.push((m.start(), m.as_str().trim().to_string()));
        }
    }
    matches.sort_by_key(|(position, _)| *position);
    let mut found = Vec::new();
    for (_, value) in matches {
        if !found.contains(&value) {
            found.push(value);
        }
        if found.len() == 20 {
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "research-master-screening-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("screenings.jsonl")
    }

    fn candidate(id: &str) -> serde_json::Value {
        json!({ "paper_id": id, "title": format!("Paper {id}"), "abstract": "" })
    }

    fn ids(candidates: &[serde_json::Value]) -> Vec<String> {
        candidates
            .iter()
            .filter_map(candidate_id)
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn roundtrip_records_decisions_and_counts_summary() {
        let path = temp_path("roundtrip");
        let store = ScreeningStore::from_path(path.clone());
        let id = store
            .create_session(
                "review",
                "mindfulness rct",
                "adults",
                "animal studies",
                vec![
                    candidate("p1"),
                    candidate("p2"),
                    candidate("p3"),
                    candidate("p4"),
                ],
                2,
            )
            .unwrap();

        let outcome = store
            .record_decisions(
                &id,
                vec![
                    ("p1".into(), "include".into(), "Meets criteria".into()),
                    ("p2".into(), "exclude".into(), "Wrong population".into()),
                    ("p3".into(), "exclude".into(), "wrong population".into()),
                    ("p404".into(), "exclude".into(), "not a candidate".into()),
                ],
            )
            .unwrap();
        assert_eq!(
            outcome,
            RecordOutcome {
                recorded: 3,
                ignored_unknown: 1
            }
        );

        let summary = store.summary(&id).unwrap().unwrap();
        assert_eq!(summary.identified, 4);
        assert_eq!(summary.duplicates_removed, 2);
        assert_eq!(summary.screened, 3);
        assert_eq!(summary.included, 1);
        assert_eq!(summary.excluded, 2);
        assert_eq!(
            summary.excluded_by_reason,
            HashMap::from([("wrong population".to_string(), 2)])
        );
        assert_eq!(summary.uncertain, 0);
        assert_eq!(summary.remaining, 1);

        // Re-deciding a candidate replaces instead of double-counting.
        store
            .record_decisions(
                &id,
                vec![("p1".into(), "uncertain".into(), "unclear".into())],
            )
            .unwrap();
        let summary = store.summary(&id).unwrap().unwrap();
        assert_eq!(summary.screened, 3);
        assert_eq!(summary.included, 0);
        assert_eq!(summary.uncertain, 1);

        assert!(store.summary("missing").unwrap().is_none());
        assert!(store
            .record_decisions(&id, vec![("p1".into(), "maybe".into(), String::new())])
            .is_err());
        assert!(store
            .record_decisions(
                "missing",
                vec![("p1".into(), "include".into(), String::new())]
            )
            .is_err());

        // State survives a reload from disk.
        let reloaded = ScreeningStore::from_path(path);
        assert_eq!(reloaded.summary(&id).unwrap().unwrap(), summary);
    }

    #[test]
    fn undecided_preserves_candidate_order_and_limit() {
        let store = ScreeningStore::from_path(temp_path("undecided"));
        let id = store
            .create_session(
                "n",
                "q",
                "i",
                "e",
                vec![
                    candidate("p1"),
                    candidate("p2"),
                    candidate("p3"),
                    candidate("p4"),
                    candidate("p5"),
                ],
                0,
            )
            .unwrap();
        store
            .record_decisions(&id, vec![("p2".into(), "exclude".into(), "no".into())])
            .unwrap();

        let all = store.undecided(&id, 10).unwrap();
        assert_eq!(ids(&all), vec!["p1", "p3", "p4", "p5"]);
        let limited = store.undecided(&id, 2).unwrap();
        assert_eq!(ids(&limited), vec!["p1", "p3"]);
        assert!(store.undecided("missing", 5).is_err());
    }

    #[test]
    fn rct_abstract_yields_effect_sizes_ci_p_and_n() {
        let text = "The intervention improved outcomes significantly (d = 0.45, \
                    95% CI [0.12, 0.78], p < 0.001; N = 234). The subgroup analysis \
                    (n = 45) showed a smaller effect (g = -0.32, p = .01). The main \
                    result remained robust (p < 0.001).";
        let stats = extract_statistics(text);
        for expected in [
            "d = 0.45",
            "g = -0.32",
            "95% CI [0.12, 0.78]",
            "p < 0.001",
            "p = .01",
            "N = 234",
            "n = 45",
        ] {
            assert!(
                stats.contains(&expected.to_string()),
                "missing {expected} in {stats:?}"
            );
        }
        assert_eq!(
            stats.iter().filter(|s| **s == "p < 0.001").count(),
            1,
            "duplicates should collapse: {stats:?}"
        );
    }

    #[test]
    fn epi_abstract_yields_ratios_ci_p_and_r() {
        let text = "Exposure was associated with disease (OR = 1.23, 95% CI: 1.1-2.3), \
                    and the trend held for smokers (RR=2.1) and non-smokers (HR 0.85, \
                    p=0.04). Correlation with dose was moderate (r = .32).";
        let stats = extract_statistics(text);
        for expected in [
            "OR = 1.23",
            "95% CI: 1.1-2.3",
            "RR=2.1",
            "HR 0.85",
            "p=0.04",
            "r = .32",
        ] {
            assert!(
                stats.contains(&expected.to_string()),
                "missing {expected} in {stats:?}"
            );
        }
    }

    #[test]
    fn statistics_keep_full_numbers_and_source_order() {
        assert_eq!(
            extract_statistics("N = 1,234; p = 1e-6; d = 0.5; p = 1E-5"),
            vec!["N = 1,234", "p = 1e-6", "d = 0.5", "p = 1E-5"]
        );
        assert!(extract_statistics("p = 1e-; N = 1,23").is_empty());
    }

    #[test]
    fn empty_text_yields_no_statistics() {
        assert!(extract_statistics("").is_empty());
        assert!(extract_statistics("We reviewed the literature on this topic.").is_empty());
    }
}
