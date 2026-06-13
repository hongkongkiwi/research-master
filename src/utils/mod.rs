//! Utility modules supporting research operations.
//!
//! This module provides utility functions and types used throughout the library:
//!
//! - [`deduplicate_papers`]: Remove duplicate papers from results using DOI matching and title similarity
//! - [`find_duplicates`]: Find duplicates without modifying the original list
//! - [`DuplicateStrategy`]: Strategy for handling duplicates (KeepFirst, KeepLast, Mark)
//! - [`HttpClient`]: HTTP client with built-in rate limiting
//! - [`RateLimitedRequestBuilder`]: Builder for rate-limited HTTP requests
//! - [`extract_text`]: Extract text content from PDF files
//! - [`has_poppler`]: Check if PDF extraction is available (requires poppler)
//! - [`PdfExtractError`]: Errors that can occur during PDF extraction
//! - [`RetryConfig`]: Configuration for retry logic with exponential backoff
//! - [`with_retry`]: Execute an operation with automatic retry on transient errors
//!
//! # Deduplication
//!
//! ```
//! use research_master::utils::DuplicateStrategy;
//!
//! // Example: deduplicate_papers takes papers and a strategy
//! let strategy = DuplicateStrategy::First;
//! assert_eq!(strategy, DuplicateStrategy::First);
//! ```
//!
//! # HTTP Client with Rate Limiting
//!
//! The HTTP client provides built-in rate limiting using the governor crate.
//! Each source can be configured with different rate limits via environment
//! variables (e.g., `SEMANTIC_SCHOLAR_RPM` for requests per minute).
//!
//! # Retry with Backoff
//!
//! ```rust
//! use research_master::utils::RetryConfig;
//!
//! let config = RetryConfig::default();
//! assert_eq!(config.max_attempts, 3);
//! ```

mod cache;
mod circuit_breaker;
mod cite;
mod dedup;
mod display;
mod history;
mod http;
mod pdf;
mod progress;
mod retry;
mod streaming;
mod update;
mod validate;

pub use streaming::{
    collect_papers, filter_by_year, paper_stream, ConcurrentPaperStream, FilterByYearStream,
    SkipStream, TakeStream,
};

pub use cache::{CacheResult, CacheService, CacheStats};
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerManager, CircuitResult, CircuitState};
pub use cite::{format_citation, get_structured_citation, CitationStyle, StructuredCitation};
pub use dedup::{deduplicate_papers, fast_deduplicate_papers, find_duplicates, DuplicateStrategy};
pub use display::{
    calculate_column_widths, calculate_dynamic_column_widths, format_authors, format_source,
    format_title, format_year, get_paper_table_columns, is_terminal, terminal_height,
    terminal_info, terminal_width, truncate_at_word, truncate_with_ellipsis, ColumnConfig,
    Terminal,
};
pub use history::{HistoryEntry, HistoryEntryType, HistoryService};
pub use http::{
    apply_cli_proxy_args, create_proxy_config_from_cli, get_user_agent, HttpClient, ProxyConfig,
    RateLimitedRequestBuilder, USER_AGENT_ENV_VAR,
};
pub use pdf::{
    extract_text, extract_text_simple, get_extraction_info, has_poppler, has_tesseract,
    ExtractionInfo, ExtractionMethod, PdfExtractError,
};
pub use progress::{ProgressReporter, SharedProgress};
pub use retry::{
    api_retry_config, strict_rate_limit_retry_config, with_retry, with_retry_detailed, RetryConfig,
    RetryResult, TransientError,
};
pub use update::{
    cleanup_temp_files, compute_sha256, detect_installation, download_and_extract_asset,
    fetch_and_verify_sha256, fetch_latest_release, fetch_sha256_signature, find_asset_for_platform,
    get_current_target, get_update_instructions, replace_binary, verify_gpg_signature,
    verify_sha256, InstallationMethod, ReleaseAsset, ReleaseInfo,
};
pub use validate::{
    sanitize_filename, sanitize_paper_id, validate_doi, validate_url, ValidationError,
};
