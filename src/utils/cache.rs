//! Local caching for search results and other API responses.
//!
//! This module provides a file-based cache for storing search results,
//! citation lookups, and other API responses to reduce network calls.
//!
//! # Cache Structure
//!
//! ```text
//! ~/.cache/research-master/
//!   searches/
//!     <hash>.json
//!   citations/
//!     <hash>.json
//! ```
//!
//! Each cached item is a JSON file containing the cached data plus metadata.

use crate::config::{CacheConfig, Config};
use crate::models::{SearchQuery, SearchResponse};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

static CACHE_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Cache metadata stored with each cached item
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheMetadata {
    /// When the item was cached (Unix timestamp)
    cached_at: u64,

    /// When the item expires (Unix timestamp)
    expires_at: u64,

    /// Source ID that provided this data
    source: String,

    /// Query that was executed
    query: String,
}

/// Wrapper for cached search response
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedSearchResponse {
    /// Cache metadata
    metadata: CacheMetadata,

    /// The actual search response
    response: SearchResponse,
}

/// Result of a cache lookup
pub enum CacheResult<T> {
    /// Item was found and is valid
    Hit(T),

    /// Item was not found
    Miss,

    /// Item was found but has expired
    Expired,
}

/// Cache service for storing and retrieving cached data
#[derive(Debug, Clone)]
pub struct CacheService {
    /// Base cache directory
    base_dir: PathBuf,

    /// Search cache directory
    search_dir: PathBuf,

    /// Citation cache directory
    citation_dir: PathBuf,

    /// Configuration
    config: CacheConfig,
}

impl CacheService {
    /// Create a new cache service with default config
    pub fn new() -> Self {
        Self::from_config(Config::default().cache)
    }

    /// Create a new cache service with the given config
    pub fn from_config(config: CacheConfig) -> Self {
        let base_dir = config
            .directory
            .clone()
            .unwrap_or_else(crate::config::default_cache_dir);

        let search_dir = base_dir.join("searches");
        let citation_dir = base_dir.join("citations");

        Self {
            base_dir,
            search_dir,
            citation_dir,
            config,
        }
    }

    /// Initialize the cache directories
    pub fn initialize(&self) -> std::io::Result<()> {
        if self.config.enabled {
            fs::create_dir_all(&self.search_dir)?;
            fs::create_dir_all(&self.citation_dir)?;
            tracing::info!("Cache initialized at: {}", self.base_dir.display());
        } else {
            tracing::debug!("Cache is disabled");
        }
        Ok(())
    }

    /// Check if caching is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get the cache directory
    pub fn cache_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    /// Generate a cache key for a search query, including source-specific options.
    fn search_cache_key(&self, query: &SearchQuery, source: &str) -> String {
        let input = serde_json::to_vec(&(source, serde_json::to_value(query).unwrap_or_default()))
            .unwrap_or_default();
        format!("{:x}", md5::compute(input))
    }

    /// Generate a cache key for a citation lookup
    fn citation_cache_key(&self, paper_id: &str, source: &str, max_results: usize) -> String {
        let input = format!("{}|{}|{}", paper_id, source, max_results);
        let digest = md5::compute(input.as_bytes());
        format!("{:x}", digest)
    }

    /// Check if a cache entry is expired
    fn is_expired(&self, expires_at: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now >= expires_at
    }

    /// Read a cached search response
    pub fn get_search(&self, query: &SearchQuery, source: &str) -> CacheResult<SearchResponse> {
        if !self.is_enabled() {
            return CacheResult::Miss;
        }

        let key = self.search_cache_key(query, source);

        let cache_path = self.search_dir.join(&key);

        match self.read_cache_file::<CachedSearchResponse>(&cache_path) {
            Ok(cached) => {
                if self.is_expired(cached.metadata.expires_at) {
                    tracing::debug!("Cache expired for search: {}", key);
                    CacheResult::Expired
                } else {
                    tracing::debug!("Cache HIT for search: {}", key);
                    CacheResult::Hit(cached.response)
                }
            }
            Err(_) => {
                tracing::debug!("Cache MISS for search: {}", key);
                CacheResult::Miss
            }
        }
    }

    /// Cache a search response
    pub fn set_search(&self, source: &str, query: &SearchQuery, response: &SearchResponse) {
        if !self.is_enabled() {
            return;
        }

        let key = self.search_cache_key(query, source);
        let cache_path = self.search_dir.join(&key);

        let cached = CachedSearchResponse {
            metadata: CacheMetadata {
                cached_at: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                expires_at: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    + self.config.search_ttl_seconds,
                source: source.to_string(),
                query: query.query.clone(),
            },
            response: response.clone(),
        };

        if let Err(e) = self.write_cache_file(&cache_path, &cached) {
            tracing::warn!("Failed to cache search result: {}", e);
        } else {
            tracing::debug!("Cached search result: {}", key);
        }
    }

    /// Read a cached citation lookup
    pub fn get_citations(
        &self,
        paper_id: &str,
        source: &str,
        max_results: usize,
    ) -> CacheResult<SearchResponse> {
        if !self.is_enabled() {
            return CacheResult::Miss;
        }

        let key = self.citation_cache_key(paper_id, source, max_results);
        let cache_path = self.citation_dir.join(&key);

        match self.read_cache_file::<CachedSearchResponse>(&cache_path) {
            Ok(cached) => {
                if self.is_expired(cached.metadata.expires_at) {
                    tracing::debug!("Cache expired for citations: {}", key);
                    CacheResult::Expired
                } else {
                    tracing::debug!("Cache HIT for citations: {}", key);
                    CacheResult::Hit(cached.response)
                }
            }
            Err(_) => {
                tracing::debug!("Cache MISS for citations: {}", key);
                CacheResult::Miss
            }
        }
    }

    /// Cache a citation lookup response
    pub fn set_citations(
        &self,
        source: &str,
        paper_id: &str,
        max_results: usize,
        response: &SearchResponse,
    ) {
        if !self.is_enabled() {
            return;
        }

        let key = self.citation_cache_key(paper_id, source, max_results);
        let cache_path = self.citation_dir.join(&key);

        let cached = CachedSearchResponse {
            metadata: CacheMetadata {
                cached_at: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                expires_at: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    + self.config.citation_ttl_seconds,
                source: source.to_string(),
                query: format!("citations for {}", paper_id),
            },
            response: response.clone(),
        };

        if let Err(e) = self.write_cache_file(&cache_path, &cached) {
            tracing::warn!("Failed to cache citations: {}", e);
        } else {
            tracing::debug!("Cached citations: {}", key);
        }
    }

    /// Read a cached file and deserialize it
    fn read_cache_file<T: for<'de> Deserialize<'de>>(
        &self,
        path: &Path,
    ) -> Result<T, std::io::Error> {
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Serialize and write a cached file
    fn write_cache_file<T: Serialize>(&self, path: &Path, data: &T) -> Result<(), std::io::Error> {
        let content = serde_json::to_vec_pretty(data)?;
        let _guard = CACHE_WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fs::create_dir_all(path.parent().expect("cache entry has a parent"))?;
        let budget = (self.config.max_size_mb as u64).saturating_mul(1024 * 1024);
        if content.len() as u64 > budget {
            return Err(std::io::Error::other("cache entry exceeds size limit"));
        }
        let mut files = Vec::new();
        for dir in [&self.search_dir, &self.citation_dir] {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries {
                    let entry = entry?;
                    let metadata = entry.metadata()?;
                    if metadata.is_file() && entry.path() != path {
                        files.push((
                            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                            metadata.len(),
                            entry.path(),
                        ));
                    }
                }
            }
        }
        files.sort_by_key(|entry| entry.0);
        let mut size: u64 = files.iter().map(|entry| entry.1).sum();
        for (_, len, file) in files {
            if size.saturating_add(content.len() as u64) <= budget {
                break;
            }
            fs::remove_file(file)?;
            size -= len;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, content)?;
        fs::rename(tmp, path)
    }

    /// Clear all cached data
    pub fn clear_all(&self) -> std::io::Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        let _ = fs::remove_dir_all(&self.base_dir);
        self.initialize()?;
        tracing::info!("Cache cleared");
        Ok(())
    }

    /// Clear only search cache
    pub fn clear_searches(&self) -> std::io::Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        let _ = fs::remove_dir_all(&self.search_dir);
        fs::create_dir_all(&self.search_dir)?;
        tracing::info!("Search cache cleared");
        Ok(())
    }

    /// Clear only citation cache
    pub fn clear_citations(&self) -> std::io::Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        let _ = fs::remove_dir_all(&self.citation_dir);
        fs::create_dir_all(&self.citation_dir)?;
        tracing::info!("Citation cache cleared");
        Ok(())
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        if !self.is_enabled() {
            return CacheStats::disabled();
        }

        let search_count = self.search_dir.read_dir().map(|e| e.count()).unwrap_or(0);
        let citation_count = self.citation_dir.read_dir().map(|e| e.count()).unwrap_or(0);

        let search_size = self
            .dir_size(&self.search_dir)
            .map(|s| s / 1024)
            .unwrap_or(0); // KB
        let citation_size = self
            .dir_size(&self.citation_dir)
            .map(|s| s / 1024)
            .unwrap_or(0); // KB

        CacheStats {
            enabled: true,
            cache_dir: self.base_dir.clone(),
            search_count,
            citation_count,
            search_size_kb: search_size,
            citation_size_kb: citation_size,
            total_size_kb: search_size + citation_size,
            ttl_search: Duration::from_secs(self.config.search_ttl_seconds),
            ttl_citations: Duration::from_secs(self.config.citation_ttl_seconds),
        }
    }

    /// Calculate the total size of a directory
    #[allow(clippy::only_used_in_recursion)]
    fn dir_size(&self, path: &Path) -> Result<u64, std::io::Error> {
        let mut size = 0;
        if let Ok(entries) = path.read_dir() {
            for entry in entries.flatten() {
                size += if entry.path().is_dir() {
                    self.dir_size(&entry.path()).unwrap_or(0)
                } else {
                    entry.metadata().map(|m| m.len()).unwrap_or(0)
                };
            }
        }
        Ok(size)
    }
}

impl Default for CacheService {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the cache
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Whether caching is enabled
    pub enabled: bool,

    /// Cache directory path
    pub cache_dir: PathBuf,

    /// Number of cached search results
    pub search_count: usize,

    /// Number of cached citation lookups
    pub citation_count: usize,

    /// Size of search cache in KB
    pub search_size_kb: u64,

    /// Size of citation cache in KB
    pub citation_size_kb: u64,

    /// Total size in KB
    pub total_size_kb: u64,

    /// TTL for search results
    pub ttl_search: Duration,

    /// TTL for citation results
    pub ttl_citations: Duration,
}

impl CacheStats {
    /// Return stats indicating cache is disabled
    fn disabled() -> Self {
        Self {
            enabled: false,
            cache_dir: PathBuf::new(),
            search_count: 0,
            citation_count: 0,
            search_size_kb: 0,
            citation_size_kb: 0,
            total_size_kb: 0,
            ttl_search: Duration::ZERO,
            ttl_citations: Duration::ZERO,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_cache_config() -> CacheConfig {
        CacheConfig {
            enabled: true,
            directory: None,
            search_ttl_seconds: 60, // 1 minute for tests
            citation_ttl_seconds: 30,
            max_size_mb: 10,
        }
    }

    #[tokio::test]
    async fn test_cache_search() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = test_cache_config();
        config.directory = Some(temp_dir.path().to_path_buf());

        let cache = CacheService::from_config(config);
        cache.initialize().unwrap();

        let response =
            SearchResponse::new(vec![], "test_source".to_string(), "test query".to_string());

        // Create a query to use for both setting and getting cache
        let query = SearchQuery::new("test query");

        // Cache a search
        cache.set_search("test_source", &query, &response);

        // Should be a hit
        match cache.get_search(&query, "test_source") {
            CacheResult::Hit(r) => {
                assert_eq!(r.source, "test_source");
                assert_eq!(r.query, "test query");
            }
            _ => panic!("Expected cache hit"),
        }

        // Different query should be a miss
        let query2 = SearchQuery::new("different query");
        match cache.get_search(&query2, "test_source") {
            CacheResult::Miss => {}
            _ => panic!("Expected cache miss for different query"),
        }

        cache.clear_all().unwrap();
    }

    #[test]
    fn search_cache_key_includes_query_options() {
        let cache = CacheService::from_config(test_cache_config());
        let base = SearchQuery::new("same search");
        let first = cache.search_cache_key(&base, "arxiv");
        assert_ne!(
            first,
            cache.search_cache_key(&base.clone().fetch_details(false), "arxiv")
        );
        assert_ne!(
            first,
            cache.search_cache_key(&base.clone().sort_by(crate::models::SortBy::Date), "arxiv")
        );
        assert_ne!(
            first,
            cache.search_cache_key(&base.clone().filter("type", "paper"), "arxiv")
        );
        assert_ne!(first, cache.search_cache_key(&base, "semantic"));
    }

    #[test]
    fn cache_keys_are_stable_and_citations_use_requested_limit() {
        let dir = TempDir::new().unwrap();
        let mut config = test_cache_config();
        config.directory = Some(dir.path().join("fresh"));
        let cache = CacheService::from_config(config);
        let a = SearchQuery::new("q").filter("a", "1").filter("b", "2");
        let b = SearchQuery::new("q").filter("b", "2").filter("a", "1");
        assert_eq!(
            cache.search_cache_key(&a, "s"),
            cache.search_cache_key(&b, "s")
        );
        let response = SearchResponse::new(vec![], "s".to_string(), "q".to_string());
        cache.set_search("s", &a, &response);
        assert!(matches!(cache.get_search(&b, "s"), CacheResult::Hit(_)));
        cache.set_citations("s", "id", 10, &response);
        assert!(matches!(
            cache.get_citations("id", "s", 10),
            CacheResult::Hit(_)
        ));
        assert!(matches!(
            cache.get_citations("id", "s", 0),
            CacheResult::Miss
        ));
    }

    #[tokio::test]
    async fn test_cache_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let config = CacheConfig {
            enabled: false,
            directory: Some(temp_dir.path().to_path_buf()),
            ..test_cache_config()
        };

        let cache = CacheService::from_config(config);

        let response =
            SearchResponse::new(vec![], "test_source".to_string(), "test query".to_string());

        let query = SearchQuery::new("test query");

        // Cache should be ignored when disabled
        cache.set_search("test_source", &query, &response);

        match cache.get_search(&query, "test_source") {
            CacheResult::Miss => {}
            _ => panic!("Expected cache miss when disabled"),
        }
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let temp_dir = TempDir::new().unwrap();
        let config = CacheConfig {
            enabled: true,
            directory: Some(temp_dir.path().to_path_buf()),
            search_ttl_seconds: 0, // Immediate expiration for testing
            citation_ttl_seconds: 0,
            max_size_mb: 10,
        };

        let cache = CacheService::from_config(config);
        cache.initialize().unwrap();

        let response =
            SearchResponse::new(vec![], "test_source".to_string(), "test query".to_string());

        let query = SearchQuery::new("test query");

        cache.set_search("test_source", &query, &response);

        match cache.get_search(&query, "test_source") {
            CacheResult::Expired => {}
            _ => panic!("Expected cache expired"),
        }
    }
}
