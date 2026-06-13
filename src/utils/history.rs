//! History tracking for searches and downloads.
//!
//! This module provides simple history tracking stored in the config directory.

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// History entry type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HistoryEntryType {
    /// Search query
    Search,
    /// Paper download
    Download,
    /// Paper viewed/read
    View,
}

/// A single history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Type of entry
    pub entry_type: HistoryEntryType,
    /// Timestamp (Unix epoch)
    pub timestamp: u64,
    /// Query or paper ID
    pub query: String,
    /// Source (if applicable)
    pub source: Option<String>,
    /// Paper title (for downloads/views)
    pub title: Option<String>,
    /// Additional details
    pub details: Option<String>,
}

/// History service
#[derive(Debug, Clone)]
pub struct HistoryService {
    /// History file path
    path: PathBuf,
}

impl HistoryService {
    /// Create a new history service
    pub fn new() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
            .join("research-master");
        let path = config_dir.join("history.jsonl");
        Self { path }
    }

    /// Ensure history file exists
    fn ensure_file(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !self.path.exists() {
            File::create(&self.path)?;
        }
        Ok(())
    }

    /// Add a search entry
    pub fn add_search(&self, query: &str, source: Option<&str>) -> io::Result<()> {
        self.ensure_file()?;
        let entry = HistoryEntry {
            entry_type: HistoryEntryType::Search,
            timestamp: now(),
            query: query.to_string(),
            source: source.map(|s| s.to_string()),
            title: None,
            details: None,
        };
        self.append_entry(&entry)
    }

    /// Add a download entry
    pub fn add_download(
        &self,
        paper_id: &str,
        source: &str,
        title: Option<&str>,
        path: Option<&str>,
    ) -> io::Result<()> {
        self.ensure_file()?;
        let entry = HistoryEntry {
            entry_type: HistoryEntryType::Download,
            timestamp: now(),
            query: paper_id.to_string(),
            source: Some(source.to_string()),
            title: title.map(|s| s.to_string()),
            details: path.map(|s| s.to_string()),
        };
        self.append_entry(&entry)
    }

    /// Add a view entry
    pub fn add_view(&self, paper_id: &str, source: &str, title: Option<&str>) -> io::Result<()> {
        self.ensure_file()?;
        let entry = HistoryEntry {
            entry_type: HistoryEntryType::View,
            timestamp: now(),
            query: paper_id.to_string(),
            source: Some(source.to_string()),
            title: title.map(|s| s.to_string()),
            details: None,
        };
        self.append_entry(&entry)
    }

    /// Append an entry to the history file
    fn append_entry(&self, entry: &HistoryEntry) -> io::Result<()> {
        let mut file = fs::OpenOptions::new().append(true).open(&self.path)?;
        let json = serde_json::to_string(entry)?;
        writeln!(file, "{}", json)?;
        Ok(())
    }

    /// Read history entries
    pub fn read_entries(&self, limit: usize) -> io::Result<Vec<HistoryEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut entries: Vec<HistoryEntry> = Vec::new();

        for line in reader.lines().take(limit * 2) {
            let line = line?;
            if let Ok(entry) = serde_json::from_str(&line) {
                entries.push(entry);
            }
        }

        // Reverse to get newest first, then take limit
        entries.reverse();
        entries.truncate(limit);

        Ok(entries)
    }

    /// Filter entries by type
    pub fn filter_entries(
        &self,
        entries: &[HistoryEntry],
        entry_type: HistoryEntryType,
    ) -> Vec<HistoryEntry> {
        entries
            .iter()
            .filter(|e| e.entry_type == entry_type)
            .cloned()
            .collect()
    }

    /// Clear history
    pub fn clear(&self) -> io::Result<()> {
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        self.ensure_file()?;
        Ok(())
    }

    /// Get history file path (for external access)
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Default for HistoryService {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current timestamp
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_service() -> (TempDir, HistoryService) {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("history.jsonl");
        let service = HistoryService { path };
        (temp_dir, service)
    }

    #[test]
    fn test_history_service_creation_with_temp_path() {
        let (temp_dir, service) = test_service();
        let expected_path = temp_dir.path().join("history.jsonl");

        assert_eq!(service.path(), expected_path.as_path());
        assert!(!service.path().exists());
        assert!(service.read_entries(10).unwrap().is_empty());
    }

    #[test]
    fn test_add_search_and_read_back_entries() {
        let (_temp_dir, service) = test_service();
        let before = now();

        service
            .add_search("quantum computing", Some("arxiv"))
            .unwrap();

        let entries = service.read_entries(10).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.entry_type, HistoryEntryType::Search);
        assert!(entry.timestamp >= before);
        assert_eq!(entry.query, "quantum computing");
        assert_eq!(entry.source.as_deref(), Some("arxiv"));
        assert_eq!(entry.title, None);
        assert_eq!(entry.details, None);
    }

    #[test]
    fn test_add_download_with_title_and_path_details() {
        let (_temp_dir, service) = test_service();

        service
            .add_download(
                "2301.00001",
                "arxiv",
                Some("A Test Paper"),
                Some("/tmp/a-test-paper.pdf"),
            )
            .unwrap();

        let entries = service.read_entries(10).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.entry_type, HistoryEntryType::Download);
        assert_eq!(entry.query, "2301.00001");
        assert_eq!(entry.source.as_deref(), Some("arxiv"));
        assert_eq!(entry.title.as_deref(), Some("A Test Paper"));
        assert_eq!(entry.details.as_deref(), Some("/tmp/a-test-paper.pdf"));
    }

    #[test]
    fn test_add_view_with_title() {
        let (_temp_dir, service) = test_service();

        service
            .add_view("10.1234/example", "crossref", Some("Viewed Paper"))
            .unwrap();

        let entries = service.read_entries(10).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.entry_type, HistoryEntryType::View);
        assert_eq!(entry.query, "10.1234/example");
        assert_eq!(entry.source.as_deref(), Some("crossref"));
        assert_eq!(entry.title.as_deref(), Some("Viewed Paper"));
        assert_eq!(entry.details, None);
    }

    #[test]
    fn test_read_entries_limit_parameter() {
        let (_temp_dir, service) = test_service();

        service.add_search("first", None).unwrap();
        service.add_search("second", None).unwrap();
        service.add_search("third", None).unwrap();
        service.add_search("fourth", None).unwrap();

        let entries = service.read_entries(2).unwrap();
        let queries: Vec<&str> = entries.iter().map(|entry| entry.query.as_str()).collect();

        assert_eq!(entries.len(), 2);
        assert_eq!(queries, vec!["fourth", "third"]);
    }

    #[test]
    fn test_filter_entries_by_type() {
        let (_temp_dir, service) = test_service();

        service
            .add_search("neural networks", Some("arxiv"))
            .unwrap();
        service
            .add_download("2301.00002", "arxiv", Some("Download"), None)
            .unwrap();
        service
            .add_view("2301.00003", "arxiv", Some("View"))
            .unwrap();

        let entries = service.read_entries(10).unwrap();
        let searches = service.filter_entries(&entries, HistoryEntryType::Search);
        let downloads = service.filter_entries(&entries, HistoryEntryType::Download);
        let views = service.filter_entries(&entries, HistoryEntryType::View);

        assert_eq!(searches.len(), 1);
        assert_eq!(searches[0].query, "neural networks");
        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].query, "2301.00002");
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].query, "2301.00003");
    }

    #[test]
    fn test_clear_functionality() {
        let (_temp_dir, service) = test_service();

        service.add_search("to be cleared", None).unwrap();
        assert_eq!(service.read_entries(10).unwrap().len(), 1);
        assert!(service.path().exists());

        service.clear().unwrap();

        assert!(service.path().exists());
        assert!(service.read_entries(10).unwrap().is_empty());
        assert_eq!(std::fs::metadata(service.path()).unwrap().len(), 0);
    }

    #[test]
    fn test_multiple_entries_are_returned_newest_first() {
        let (_temp_dir, service) = test_service();

        service.add_search("oldest", None).unwrap();
        service
            .add_download("middle", "arxiv", Some("Middle"), Some("/tmp/middle.pdf"))
            .unwrap();
        service
            .add_view("newest", "pubmed", Some("Newest"))
            .unwrap();

        let entries = service.read_entries(10).unwrap();
        let queries: Vec<&str> = entries.iter().map(|entry| entry.query.as_str()).collect();
        let types: Vec<HistoryEntryType> = entries
            .iter()
            .map(|entry| entry.entry_type.clone())
            .collect();

        assert_eq!(queries, vec!["newest", "middle", "oldest"]);
        assert_eq!(
            types,
            vec![
                HistoryEntryType::View,
                HistoryEntryType::Download,
                HistoryEntryType::Search,
            ]
        );
    }

    #[test]
    fn test_history_entry_struct_field_access() {
        let entry = HistoryEntry {
            entry_type: HistoryEntryType::Download,
            timestamp: 1_700_000_000,
            query: "paper-id".to_string(),
            source: Some("semantic".to_string()),
            title: Some("Accessible Fields".to_string()),
            details: Some("/papers/accessible-fields.pdf".to_string()),
        };

        assert_eq!(entry.entry_type, HistoryEntryType::Download);
        assert_eq!(entry.timestamp, 1_700_000_000);
        assert_eq!(entry.query, "paper-id");
        assert_eq!(entry.source.as_deref(), Some("semantic"));
        assert_eq!(entry.title.as_deref(), Some("Accessible Fields"));
        assert_eq!(
            entry.details.as_deref(),
            Some("/papers/accessible-fields.pdf")
        );
    }

    #[test]
    fn test_default_implementation_matches_new() {
        let default_service = HistoryService::default();
        let new_service = HistoryService::new();

        assert_eq!(default_service.path(), new_service.path());
        assert_eq!(default_service.path().file_name().unwrap(), "history.jsonl");
    }
}
