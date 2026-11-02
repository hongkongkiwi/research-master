//! Configuration file support for research-master.
//!
//! This module provides TOML configuration file parsing with support
//! for environment variable overrides.
//!
//! # Configuration File Format
//!
//! ```toml
//! [api_keys]
//! semantic_scholar = "your-api-key"
//! core = "your-core-api-key"
//!
//! [downloads]
//! default_path = "./downloads"
//! organize_by_source = true
//! max_file_size_mb = 100
//!
//! [rate_limits]
//! default_requests_per_second = 5.0
//! max_concurrent_requests = 10
//!
//! [sources]
//! enabled_sources = "arxiv,semantic,openalex"
//! disabled_sources = ""
//!
//! [[source_rates]]
//! source = "semantic"
//! requests_per_second = 0.5
//!
//! [[source_rates]]
//! source = "arxiv"
//! requests_per_second = 5.0
//!
//! [cache]
//! enabled = true
//! directory = "~/.cache/research-master"
//! search_ttl_seconds = 1800
//! citation_ttl_seconds = 900
//! max_size_mb = 500
//!
//! [proxy]
//! http = "http://proxy:8080"
//! https = "https://proxy:8080"
//! no_proxy = "localhost,127.0.0.1"
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration file structure
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    /// API keys section
    #[serde(default)]
    pub api_keys: ApiKeysConfig,

    /// Downloads section
    #[serde(default)]
    pub downloads: DownloadsConfig,

    /// Rate limits section
    #[serde(default)]
    pub rate_limits: RateLimitsConfig,

    /// Sources section
    #[serde(default)]
    pub sources: SourcesConfig,

    /// Per-source rate limits
    #[serde(default)]
    pub source_rates: Vec<SourceRateConfig>,

    /// Cache section
    #[serde(default)]
    pub cache: CacheConfig,

    /// Proxy section
    #[serde(default)]
    pub proxy: ProxyConfig,

    /// Logging section
    #[serde(default)]
    pub logging: LoggingConfig,
}

/// API keys configuration
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ApiKeysConfig {
    #[serde(default)]
    pub semantic_scholar: Option<String>,

    #[serde(default)]
    pub core: Option<String>,
}

/// Downloads configuration
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DownloadsConfig {
    #[serde(default = "default_download_path")]
    pub default_path: PathBuf,

    #[serde(default = "default_true")]
    pub organize_by_source: bool,

    #[serde(default = "default_max_file_size")]
    pub max_file_size_mb: usize,
}

fn default_download_path() -> PathBuf {
    PathBuf::from("./downloads")
}

fn default_true() -> bool {
    true
}

fn default_max_file_size() -> usize {
    100
}

/// Rate limits configuration
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RateLimitsConfig {
    #[serde(default = "default_rps")]
    pub default_requests_per_second: f32,

    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_requests: usize,
}

fn default_rps() -> f32 {
    5.0
}

fn default_max_concurrent() -> usize {
    10
}

/// Sources configuration
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SourcesConfig {
    #[serde(default)]
    pub enabled_sources: Option<String>,

    #[serde(default)]
    pub disabled_sources: Option<String>,
}

/// Per-source rate limit configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRateConfig {
    pub source: String,
    pub requests_per_second: f32,
}

/// Cache configuration
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub directory: Option<PathBuf>,

    #[serde(default = "default_search_ttl")]
    pub search_ttl_seconds: u64,

    #[serde(default = "default_citation_ttl")]
    pub citation_ttl_seconds: u64,

    #[serde(default = "default_max_cache_size")]
    pub max_size_mb: usize,
}

fn default_search_ttl() -> u64 {
    1800 // 30 minutes
}

fn default_citation_ttl() -> u64 {
    900 // 15 minutes
}

fn default_max_cache_size() -> usize {
    500
}

/// Proxy settings consumed by the HTTP client configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxyConfig {
    #[serde(default)]
    pub http: Option<String>,

    #[serde(default)]
    pub https: Option<String>,

    #[serde(default)]
    pub no_proxy: Option<String>,
}

/// Logging configuration
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,

    #[serde(default)]
    pub format: Option<String>,
}

fn default_log_level() -> String {
    "info".to_string()
}

impl ConfigFile {
    /// Load configuration from a TOML file
    pub fn load(path: &PathBuf) -> Result<Self, ConfigFileError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| ConfigFileError::Io(e.to_string()))?;

        toml::from_str(&content).map_err(|e| ConfigFileError::Parse(e.to_string()))
    }

    /// Save configuration to a TOML file
    pub fn save(&self, path: &PathBuf) -> Result<(), ConfigFileError> {
        let content =
            toml::to_string_pretty(self).map_err(|e| ConfigFileError::Serialize(e.to_string()))?;

        std::fs::write(path, content).map_err(|e| ConfigFileError::Io(e.to_string()))
    }

    /// Create default configuration
    #[allow(clippy::should_implement_trait)]
    pub fn create_default() -> Self {
        Self {
            api_keys: ApiKeysConfig::default(),
            downloads: DownloadsConfig::default(),
            rate_limits: RateLimitsConfig::default(),
            sources: SourcesConfig::default(),
            source_rates: Vec::new(),
            cache: CacheConfig::default(),
            proxy: ProxyConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

/// Configuration file errors
#[derive(Debug, thiserror::Error)]
pub enum ConfigFileError {
    #[error("IO error: {0}")]
    Io(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Serialize error: {0}")]
    Serialize(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_config_file_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let toml_content = r#"
[api_keys]
semantic_scholar = "test-key"
core = "core-key"

[downloads]
default_path = "/tmp/downloads"
organize_by_source = true
max_file_size_mb = 200

[rate_limits]
default_requests_per_second = 3.0
max_concurrent_requests = 5

[sources]
enabled_sources = "arxiv,semantic"

[[source_rates]]
source = "semantic"
requests_per_second = 0.5

[cache]
enabled = true
max_size_mb = 1000

[logging]
level = "debug"
"#;

        let mut file = File::create(&path).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();

        let config = ConfigFile::load(&path).unwrap();

        assert_eq!(
            config.api_keys.semantic_scholar,
            Some("test-key".to_string())
        );
        assert_eq!(config.api_keys.core, Some("core-key".to_string()));
        assert_eq!(config.downloads.max_file_size_mb, 200);
        assert_eq!(config.rate_limits.default_requests_per_second, 3.0);
        assert_eq!(
            config.sources.enabled_sources,
            Some("arxiv,semantic".to_string())
        );
        assert_eq!(config.source_rates.len(), 1);
        assert_eq!(config.source_rates[0].source, "semantic");
        assert_eq!(config.source_rates[0].requests_per_second, 0.5);
        assert!(config.cache.enabled);
        assert_eq!(config.cache.max_size_mb, 1000);
    }

    #[test]
    fn test_config_file_save_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut config = ConfigFile::default();
        config.api_keys.semantic_scholar = Some("saved-key".to_string());
        config.rate_limits.default_requests_per_second = 2.0;

        config.save(&path).unwrap();

        let loaded = ConfigFile::load(&path).unwrap();
        assert_eq!(
            loaded.api_keys.semantic_scholar,
            Some("saved-key".to_string())
        );
        assert_eq!(loaded.rate_limits.default_requests_per_second, 2.0);
    }

    #[test]
    fn test_config_file_nonexistent() {
        let path = PathBuf::from("/nonexistent/config.toml");
        let result = ConfigFile::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_file_invalid_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("invalid.toml");

        std::fs::write(&path, "invalid = toml = content").unwrap();

        let result = ConfigFile::load(&path);
        assert!(result.is_err());
    }
}
