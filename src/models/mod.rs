//! Core data models for research papers and search operations.
//!
//! This module provides the primary data structures used throughout the library:
//!
//! - [`Paper`]: A unified representation of a research paper from any source
//! - [`PaperBuilder`]: Fluent builder for constructing Paper objects
//! - [`SearchQuery`]: Search parameters with builder-style API
//! - [`SearchResponse`]: Search results with metadata
//! - [`DownloadRequest`]/[`DownloadResult`]: Paper download operations
//! - [`ReadRequest`]/[`ReadResult`]: PDF text extraction operations
//! - [`CitationRequest`]: Citation and reference lookup
//! - [`SourceType`]: Enum of all supported research sources
//!
//! # Examples
//!
//! ```rust
//! use research_master::models::{Paper, PaperBuilder, SourceType, SearchQuery};
//!
//! // Create a paper using the builder
//! let paper = PaperBuilder::new(
//!     "2301.12345",
//!     "My Paper Title",
//!     "https://example.com/paper",
//!     SourceType::Arxiv
//! )
//! .authors("Jane Doe; John Smith")
//! .abstract_text("This is the abstract.")
//! .doi("10.1234/example.1234")
//! .build();
//!
//! // Create a search query
//! let query = SearchQuery::new("machine learning")
//!     .max_results(20)
//!     .year("2020-");
//! ```

mod paper;
mod search;

pub use paper::{Paper, PaperBuilder, SourceType};
pub use search::{
    year_bounds, BatchDownloadRequest, BatchDownloadResult, CitationRequest, DownloadRequest,
    DownloadResult, LicensePolicy, ReadRequest, ReadResult, SearchQuery, SearchResponse, SortBy,
    SortOrder,
};
