//! Research source plugins with extensible trait-based architecture.
//!
//! This module defines the [`Source`] trait that all research sources implement.
//! New sources can be added by implementing this trait and registering them with
//! the [`SourceRegistry`]. Also, sources can be selectively enabled or disabled at
//! runtime using environment variables or a configuration file.
//!
//! # Feature Flags
//!
//! Individual sources can be disabled at compile time using Cargo features:
//!
//! - `arxiv` - Enable arXiv source (default: enabled)
//! - `pubmed` - Enable PubMed source (default: enabled)
//! - `biorxiv` - Enable bioRxiv source (default: enabled)
//! - `medrxiv` - Enable medRxiv source (default: enabled)
//! - `semantic` - Enable Semantic Scholar source (default: enabled)
//! - `openalex` - Enable OpenAlex source (default: enabled)
//! - `crossref` - Enable CrossRef source (default: enabled)
//! - `iacr` - Enable IACR ePrint source (default: enabled)
//! - `pmc` - Enable PMC source (default: enabled)
//! - `hal` - Enable HAL source (default: enabled)
//! - `dblp` - Enable DBLP source (default: enabled)
//! - `ssrn` - Enable SSRN source (default: enabled)
//! - `dimensions` - Enable Dimensions source (default: enabled)
//! - `ieee_xplore` - Enable IEEE Xplore source (default: enabled)
//! - `core` - Enable CORE source (default: enabled)
//! - `zenodo` - Enable Zenodo source (default: enabled)
//! - `unpaywall` - Enable Unpaywall source (default: enabled)
//! - `mdpi` - Enable MDPI source (default: enabled)
//! - `jstor` - Enable JSTOR source (default: enabled)
//! - `scispace` - Enable SciSpace source (default: enabled)
//! - `acm` - Enable ACM Digital Library source (default: enabled)
//! - `citeseerx` - Enable CiteSeerX source (default: enabled)
//! - `connected_papers` - Enable Connected Papers source (default: enabled)
//! - `doaj` - Enable DOAJ source (default: enabled)
//! - `worldwidescience` - Enable WorldWideScience source (default: enabled)
//! - `osf` - Enable OSF Preprints source (default: enabled)
//! - `base` - Enable BASE source (default: enabled)
//! - `springer` - Enable Springer source (default: enabled)
//! - `google_scholar` - Enable Google Scholar source (default: disabled, requires GOOGLE_SCHOLAR_ENABLED=true)
//!
//! # Feature Groups
//!
//! - `core` - arxiv, pubmed, semantic
//! - `preprints` - arxiv, biorxiv, medrxiv
//! - `full` - All sources (default)
//!
//! # Runtime Source Configuration
//!
//! All sources are compiled in by default. Use these environment variables to control which
//! sources are used at runtime:
//!
//! ## Default Disabled Sources (Slow Sources)
//!
//! Some sources are disabled by default due to high latency. Set this to enable them:
//!
//! - `RESEARCH_MASTER_DEFAULT_DISABLED_SOURCES` - Comma-separated list of slow sources to disable by default
//!   (e.g., "biorxiv,pmc,pubmed")
//!
//!   **Default**: "biorxiv,pmc,pubmed" (these sources take 3-15 seconds per request)
//!
//! ## Explicit Enable/Disable
//!
//! - `RESEARCH_MASTER_ENABLED_SOURCES` - Only use these sources (e.g., "arxiv,semantic,openalex")
//! - `RESEARCH_MASTER_DISABLED_SOURCES` - Never use these sources (e.g., "dblp,jstor")
//!
//! ## Priority Rules
//!
//! 1. If `ENABLED_SOURCES` is set, only those sources are used
//! 2. If `ENABLED_SOURCES` is not set, sources in `DEFAULT_DISABLED_SOURCES` are excluded
//! 3. `DISABLED_SOURCES` always takes precedence - those sources are never used
//!
//! ## Examples
//!
//! ```bash
//! # Use only fast sources (default behavior)
//! export RESEARCH_MASTER_DEFAULT_DISABLED_SOURCES="biorxiv,pmc,pubmed"
//!
//! # Enable slow sources explicitly
//! export RESEARCH_MASTER_ENABLED_SOURCES="arxiv,semantic,openalex,biorxiv,pmc,pubmed"
//!
//! # Disable a fast source you don't need
//! export RESEARCH_MASTER_DISABLED_SOURCES="jstor,dblp"
//!
//! # Use specific sources only
//! export RESEARCH_MASTER_ENABLED_SOURCES="arxiv,semantic"
//! ```
//!
//! # Compile-Time vs Runtime
//!
//! Use **compile-time features** to reduce binary size by excluding sources entirely.
//! Use **runtime configuration** to keep all sources compiled but control which are used.
//!
//! ```bash
//! # Compile-time: exclude slow sources from binary
//! cargo build --no-default-features --features "core,openalex,crossref,arxiv"
//!
//! # Runtime: include all sources but disable slow ones by default
//! cargo build
//! export RESEARCH_MASTER_DEFAULT_DISABLED_SOURCES="biorxiv,pmc,pubmed"
//! ```

#[cfg(feature = "source-acm")]
mod acm;
#[cfg(feature = "source-arxiv")]
mod arxiv;
#[cfg(feature = "source-base")]
mod base;
#[cfg(feature = "source-biorxiv")]
mod biorxiv;
#[cfg(feature = "source-citeseerx")]
mod citeseerx;
#[cfg(feature = "source-connected_papers")]
mod connected_papers;
#[cfg(feature = "source-core-repo")]
mod core;
#[cfg(feature = "source-crossref")]
mod crossref;
#[cfg(feature = "source-dblp")]
mod dblp;
#[cfg(feature = "source-dimensions")]
mod dimensions;
#[cfg(feature = "source-doaj")]
mod doaj;
#[cfg(feature = "source-europe_pmc")]
mod europe_pmc;
#[cfg(feature = "source-google_scholar")]
mod google_scholar;
#[cfg(feature = "source-hal")]
mod hal;
#[cfg(feature = "source-iacr")]
mod iacr;
#[cfg(feature = "source-ieee_xplore")]
mod ieee_xplore;
#[cfg(feature = "source-jstor")]
mod jstor;
#[cfg(feature = "source-mdpi")]
mod mdpi;
#[cfg(feature = "source-medrxiv")]
mod medrxiv;
#[cfg(feature = "source-openaire")]
mod openaire;
#[cfg(feature = "source-openalex")]
mod openalex;
#[cfg(feature = "source-orcid")]
mod orcid;
#[cfg(feature = "source-osf")]
mod osf;
#[cfg(feature = "source-pmc")]
mod pmc;
#[cfg(feature = "source-pubmed")]
mod pubmed;
mod registry;
#[cfg(feature = "source-scispace")]
mod scispace;
#[cfg(feature = "source-semantic")]
mod semantic;
#[cfg(feature = "source-springer")]
mod springer;
#[cfg(feature = "source-ssrn")]
mod ssrn;
#[cfg(feature = "source-unpaywall")]
mod unpaywall;
#[cfg(feature = "source-worldwidescience")]
mod worldwidescience;
#[cfg(feature = "source-zenodo")]
mod zenodo;

#[cfg(test)]
pub mod mock;

#[cfg(test)]
pub use mock::MockSource;
pub use registry::{SourceCapabilities, SourceRegistry};

use crate::models::{
    CitationRequest, DownloadRequest, DownloadResult, Paper, ReadRequest, ReadResult, SearchQuery,
    SearchResponse,
};
use async_trait::async_trait;

/// The Source trait defines the interface for all research source plugins.
///
/// # Implementing a New Source
///
/// To add a new research source:
///
/// 1. Create a new struct that implements `Source`
/// 2. Implement the required methods (at minimum `id`, `name`, and `search`)
/// 3. Implement optional methods if the source supports them
/// 4. Add the source to `SourceRegistry::new()` or register it dynamically
#[async_trait]
pub trait Source: Send + Sync + std::fmt::Debug {
    /// Unique identifier for this source (used in tool names, e.g., "arxiv", "pubmed")
    fn id(&self) -> &str;

    /// Human-readable name of this source
    fn name(&self) -> &str;

    /// Describe the capabilities of this source
    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH
    }

    /// Whether this source supports search
    fn supports_search(&self) -> bool {
        self.capabilities().contains(SourceCapabilities::SEARCH)
    }

    /// Whether this source supports downloading PDFs
    fn supports_download(&self) -> bool {
        self.capabilities().contains(SourceCapabilities::DOWNLOAD)
    }

    /// Whether this source supports reading/parsing PDFs
    fn supports_read(&self) -> bool {
        self.capabilities().contains(SourceCapabilities::READ)
    }

    /// Whether this source supports citation/reference lookup
    fn supports_citations(&self) -> bool {
        self.capabilities().contains(SourceCapabilities::CITATIONS)
    }

    /// Whether this source supports lookup by DOI
    fn supports_doi_lookup(&self) -> bool {
        self.capabilities().contains(SourceCapabilities::DOI_LOOKUP)
    }

    /// Whether this source supports author search
    fn supports_author_search(&self) -> bool {
        self.capabilities()
            .contains(SourceCapabilities::AUTHOR_SEARCH)
    }

    // ========== SEARCH METHODS ==========

    /// Search for papers matching the query
    async fn search(&self, _query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        Err(SourceError::NotImplemented)
    }

    /// Search for papers by a specific author
    async fn search_by_author(
        &self,
        _author: &str,
        _max_results: usize,
        _year: Option<&str>,
    ) -> Result<SearchResponse, SourceError> {
        Err(SourceError::NotImplemented)
    }

    // ========== DOWNLOAD METHODS ==========

    /// Download a paper's PDF to the specified path
    async fn download(&self, _request: &DownloadRequest) -> Result<DownloadResult, SourceError> {
        Err(SourceError::NotImplemented)
    }

    // ========== READ METHODS ==========

    /// Read and extract text from a paper's PDF
    async fn read(&self, _request: &ReadRequest) -> Result<ReadResult, SourceError> {
        Err(SourceError::NotImplemented)
    }

    // ========== CITATION METHODS ==========

    /// Get papers that cite this paper
    async fn get_citations(
        &self,
        _request: &CitationRequest,
    ) -> Result<SearchResponse, SourceError> {
        Err(SourceError::NotImplemented)
    }

    /// Get papers referenced by this paper
    async fn get_references(
        &self,
        _request: &CitationRequest,
    ) -> Result<SearchResponse, SourceError> {
        Err(SourceError::NotImplemented)
    }

    /// Get related papers
    async fn get_related(&self, _request: &CitationRequest) -> Result<SearchResponse, SourceError> {
        Err(SourceError::NotImplemented)
    }

    // ========== LOOKUP METHODS ==========

    /// Get a paper by its DOI
    async fn get_by_doi(&self, _doi: &str) -> Result<Paper, SourceError> {
        Err(SourceError::NotImplemented)
    }

    /// Get a paper by its ID (source-specific)
    async fn get_by_id(&self, _id: &str) -> Result<Paper, SourceError> {
        Err(SourceError::NotImplemented)
    }

    /// Validate that a paper ID is correctly formatted for this source
    fn validate_id(&self, _id: &str) -> Result<(), SourceError> {
        Ok(())
    }
}

/// Errors that can occur when interacting with a source
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// The requested operation is not implemented for this source
    #[error("Operation not implemented for this source")]
    NotImplemented,

    /// Network or HTTP error
    #[error("Network error: {0}")]
    Network(String),

    /// Parsing error (XML, JSON, HTML, etc.)
    #[error("Parse error: {0}")]
    Parse(String),

    /// Invalid request parameters
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Rate limit exceeded
    #[error("Rate limit exceeded")]
    RateLimit,

    /// Paper not found
    #[error("Paper not found: {0}")]
    NotFound(String),

    /// API error from the source
    #[error("API error: {0}")]
    Api(String),

    /// IO error (file system)
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Other error
    #[error("Error: {0}")]
    Other(String),
}

impl From<reqwest::Error> for SourceError {
    fn from(err: reqwest::Error) -> Self {
        SourceError::Network(err.to_string())
    }
}

impl From<serde_json::Error> for SourceError {
    fn from(err: serde_json::Error) -> Self {
        SourceError::Parse(format!("JSON: {}", err))
    }
}

impl From<quick_xml::DeError> for SourceError {
    fn from(err: quick_xml::DeError) -> Self {
        SourceError::Parse(format!("XML: {}", err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_capabilities() {
        let caps = SourceCapabilities::SEARCH | SourceCapabilities::DOWNLOAD;

        assert!(caps.contains(SourceCapabilities::SEARCH));
        assert!(caps.contains(SourceCapabilities::DOWNLOAD));
        assert!(!caps.contains(SourceCapabilities::CITATIONS));
    }
}
