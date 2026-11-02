//! Search request and response models.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Sort order for search results
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// Sort field for search results
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortBy {
    Relevance,
    Date,
    CitationCount,
    Title,
    Author,
}

/// How search results are filtered by the work's reported license.
/// Unknown licenses never qualify as open. This does not waive API terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LicensePolicy {
    /// Keep every result.
    #[default]
    Any,
    /// Keep works with a recognized open license; exclude unknowns.
    Open,
    /// Skip filtering by work-license metadata, without waiving API terms.
    Ignore,
}

impl LicensePolicy {
    /// Parse a policy name. Accepts `any`, `open`, and `ignore`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "any" => Some(Self::Any),
            "open" => Some(Self::Open),
            "ignore" => Some(Self::Ignore),
            _ => None,
        }
    }

    /// Whether the reported license passes the policy.
    pub fn allows(self, license: Option<&str>) -> bool {
        match self {
            Self::Any | Self::Ignore => true,
            Self::Open => license
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some_and(is_open_license),
        }
    }
}

/// Parse the shared year filter into inclusive bounds.
///
/// Accepts `"2020"`, `"2018-2022"`, `"2010-"` (from onward), and `"-2015"`
/// (until). Open ranges get practical extremes rather than being silently
/// narrowed to a single year.
pub fn year_bounds(year: &str) -> Option<(u32, u32)> {
    const MIN_YEAR: u32 = 1000;
    const MAX_YEAR: u32 = 9999;

    let year = year.trim();
    let (from, to) = if let Some(until) = year.strip_prefix('-') {
        (MIN_YEAR, until.trim().parse().ok()?)
    } else if let Some(since) = year.strip_suffix('-') {
        (since.trim().parse().ok()?, MAX_YEAR)
    } else if let Some((a, b)) = year.split_once('-') {
        (a.trim().parse().ok()?, b.trim().parse().ok()?)
    } else {
        let y = year.parse().ok()?;
        (y, y)
    };
    (from <= to).then_some((from, to))
}

/// Whether a license string describes an open license.
///
/// Matched case-insensitively against the license names and SPDX identifiers
/// sources actually return. Unknown strings are not treated as open.
pub fn is_open_license(license: &str) -> bool {
    let normalized = license.to_ascii_lowercase();
    let name = normalized.trim().trim_end_matches('/');
    let name = name.split('?').next().unwrap_or(name);
    let name = name.split('#').next().unwrap_or(name);
    name == "cc0"
        || name
            .strip_prefix("cc0-")
            .is_some_and(|version| version.chars().next().is_some_and(|c| c.is_ascii_digit()))
        || (name.starts_with("cc-by-")
            && name[6..].chars().next().is_some_and(|c| c.is_ascii_digit()))
        || (name.starts_with("cc-by-sa-")
            && name[9..].chars().next().is_some_and(|c| c.is_ascii_digit()))
        || name == "public domain"
        || name == "creative commons attribution 4.0 international"
        || name == "creative commons zero 1.0 universal"
        || name
            .strip_prefix("https://creativecommons.org/licenses/by/")
            .is_some_and(|version| version.chars().next().is_some_and(|c| c.is_ascii_digit()))
        || name
            .strip_prefix("https://creativecommons.org/licenses/by-sa/")
            .is_some_and(|version| version.chars().next().is_some_and(|c| c.is_ascii_digit()))
        || name
            .strip_prefix("https://creativecommons.org/publicdomain/zero/")
            .is_some_and(|version| version.chars().next().is_some_and(|c| c.is_ascii_digit()))
        || matches!(
            name,
            "mit" | "apache-2.0" | "bsd-2-clause" | "bsd-3-clause" | "gpl-3.0" | "unlicense"
        )
}

/// Search query parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Main search query string
    pub query: String,

    /// Maximum number of results to return
    pub max_results: usize,

    /// Year filter (single year, range like "2018-2022", or "2010-" for from, "-2015" for until)
    pub year: Option<String>,

    /// Sort by field
    pub sort_by: Option<SortBy>,

    /// Sort order
    pub sort_order: Option<SortOrder>,

    /// Field-specific filters
    pub filters: HashMap<String, String>,

    /// Author name for author-specific search
    pub author: Option<String>,

    /// Category/subject filter
    pub category: Option<String>,

    /// Whether to fetch detailed information (slower but more complete)
    pub fetch_details: bool,

    /// Which results to keep based on the license a source reports
    #[serde(default)]
    pub license_policy: LicensePolicy,

    /// Publication-type tokens results must match (case-insensitive substring
    /// against `paper_type` and categories). Empty means no include filter.
    #[serde(default)]
    pub include_types: Vec<String>,

    /// Publication-type tokens that drop a result; wins over `include_types`
    #[serde(default)]
    pub exclude_types: Vec<String>,

    /// Whether papers with no reported type/categories fail `include_types`
    #[serde(default)]
    pub strict_types: bool,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            max_results: 10,
            year: None,
            sort_by: None,
            sort_order: None,
            filters: HashMap::new(),
            author: None,
            category: None,
            fetch_details: true,
            license_policy: LicensePolicy::Any,
            include_types: Vec::new(),
            exclude_types: Vec::new(),
            strict_types: false,
        }
    }
}

impl SearchQuery {
    /// Create a new search query
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            ..Default::default()
        }
    }

    /// Set maximum results
    pub fn max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }

    /// Set year filter
    pub fn year(mut self, year: impl Into<String>) -> Self {
        self.year = Some(year.into());
        self
    }

    /// Set sort by
    pub fn sort_by(mut self, sort: SortBy) -> Self {
        self.sort_by = Some(sort);
        self
    }

    /// Set sort order
    pub fn sort_order(mut self, order: SortOrder) -> Self {
        self.sort_order = Some(order);
        self
    }

    /// Add a filter
    pub fn filter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.filters.insert(key.into(), value.into());
        self
    }

    /// Set author filter
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Set category filter
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Enable/disable detailed fetching
    pub fn fetch_details(mut self, fetch: bool) -> Self {
        self.fetch_details = fetch;
        self
    }

    /// Set the license policy applied to results
    pub fn license_policy(mut self, policy: LicensePolicy) -> Self {
        self.license_policy = policy;
        self
    }

    /// Set the publication-type tokens results must match
    pub fn include_types(mut self, types: Vec<String>) -> Self {
        self.include_types = types;
        self
    }

    /// Set the publication-type tokens results must not match
    pub fn exclude_types(mut self, types: Vec<String>) -> Self {
        self.exclude_types = types;
        self
    }

    /// Make papers without a reported type/categories fail `include_types`
    pub fn strict_types(mut self, strict: bool) -> Self {
        self.strict_types = strict;
        self
    }

    /// Whether a paper passes the include/exclude type filters.
    ///
    /// Tokens match case-insensitively by substring against the paper's
    /// reported type and category names. Exclusions always win. Papers with
    /// neither a type nor categories pass a non-empty include list (lenient
    /// default) unless `strict_types` is set.
    pub fn type_filter_allows(&self, paper: &crate::models::Paper) -> bool {
        let mut texts = Vec::with_capacity(1 + paper.category_list().len());
        if let Some(paper_type) = paper.paper_type.as_ref() {
            texts.push(paper_type.as_str());
        }
        texts.extend(paper.category_list());
        let lower: Vec<String> = texts.iter().map(|t| t.to_ascii_lowercase()).collect();

        let matches = |token: &str| {
            let token = token.to_ascii_lowercase();
            lower.iter().any(|text| text.contains(&token))
        };

        if self
            .exclude_types
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .any(matches)
        {
            return false;
        }
        let includes: Vec<&str> = self
            .include_types
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        if includes.is_empty() {
            return true;
        }
        if lower.is_empty() {
            return !self.strict_types;
        }
        includes.iter().any(|token| matches(token))
    }
}

/// Request for downloading a paper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    /// Paper ID (source-specific)
    pub paper_id: String,

    /// Where to save the PDF
    pub save_path: String,

    /// Optional DOI
    pub doi: Option<String>,
}

impl DownloadRequest {
    /// Create a new download request
    pub fn new(paper_id: impl Into<String>, save_path: impl Into<String>) -> Self {
        Self {
            paper_id: paper_id.into(),
            save_path: save_path.into(),
            doi: None,
        }
    }

    /// Set the DOI
    pub fn doi(mut self, doi: impl Into<String>) -> Self {
        self.doi = Some(doi.into());
        self
    }
}

/// Request for reading/parsing a paper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadRequest {
    /// Paper ID (source-specific)
    pub paper_id: String,

    /// Path where the PDF is saved (or will be saved)
    pub save_path: String,

    /// Whether to download if not found
    pub download_if_missing: bool,
}

impl ReadRequest {
    /// Create a new read request
    pub fn new(paper_id: impl Into<String>, save_path: impl Into<String>) -> Self {
        Self {
            paper_id: paper_id.into(),
            save_path: save_path.into(),
            download_if_missing: true,
        }
    }

    /// Set whether to download if missing
    pub fn download_if_missing(mut self, download: bool) -> Self {
        self.download_if_missing = download;
        self
    }
}

/// Request for getting citations/references
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationRequest {
    /// Paper ID (source-specific)
    pub paper_id: String,

    /// Maximum results
    pub max_results: usize,
}

impl CitationRequest {
    /// Create a new citation request
    pub fn new(paper_id: impl Into<String>) -> Self {
        Self {
            paper_id: paper_id.into(),
            max_results: 20,
        }
    }

    /// Set max results
    pub fn max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }
}

/// Search response containing papers and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    /// Papers found
    pub papers: Vec<crate::models::Paper>,

    /// Total number of results (may be more than returned)
    pub total_results: Option<usize>,

    /// Source of the results
    pub source: String,

    /// Query that was executed
    pub query: String,

    /// Whether more results are available
    pub has_more: bool,
}

impl SearchResponse {
    /// Create a new search response
    pub fn new(
        papers: Vec<crate::models::Paper>,
        source: impl Into<String>,
        query: impl Into<String>,
    ) -> Self {
        Self {
            papers,
            total_results: None,
            source: source.into(),
            query: query.into(),
            has_more: false,
        }
    }

    /// Set total results
    pub fn total_results(mut self, total: usize) -> Self {
        self.total_results = Some(total);
        self
    }

    /// Set has_more flag
    pub fn has_more(mut self, has_more: bool) -> Self {
        self.has_more = has_more;
        self
    }
}

/// Result of a download operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadResult {
    /// Path where the file was saved
    pub path: String,

    /// Number of bytes downloaded
    pub bytes: u64,

    /// Whether the download was successful
    pub success: bool,

    /// Error message if failed
    pub error: Option<String>,
}

impl DownloadResult {
    /// Create a successful download result
    pub fn success(path: impl Into<String>, bytes: u64) -> Self {
        Self {
            path: path.into(),
            bytes,
            success: true,
            error: None,
        }
    }

    /// Create a failed download result
    pub fn error(error: impl Into<String>) -> Self {
        Self {
            path: String::new(),
            bytes: 0,
            success: false,
            error: Some(error.into()),
        }
    }
}

/// Batch download request containing multiple individual download requests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDownloadRequest {
    /// List of individual download requests
    pub requests: Vec<DownloadRequest>,
}

impl BatchDownloadRequest {
    /// Create a new batch download request from a list of requests
    pub fn new(requests: Vec<DownloadRequest>) -> Self {
        Self { requests }
    }

    /// Add a download request to the batch
    pub fn add_request(&mut self, request: DownloadRequest) {
        self.requests.push(request);
    }

    /// Get the number of requests in the batch
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    /// Check if the batch is empty
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}

/// Result of a batch download operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchDownloadResult {
    /// Individual download results
    pub results: Vec<DownloadResult>,

    /// Total number of successful downloads
    pub successful: usize,

    /// Total number of failed downloads
    pub failed: usize,

    /// Total bytes downloaded
    pub total_bytes: u64,
}

impl BatchDownloadResult {
    /// Create a new batch download result from individual results
    pub fn new(results: Vec<DownloadResult>) -> Self {
        let successful = results.iter().filter(|r| r.success).count();
        let failed = results.len() - successful;
        let total_bytes = results.iter().map(|r| r.bytes).sum();

        Self {
            results,
            successful,
            failed,
            total_bytes,
        }
    }

    /// Get the success rate as a percentage (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        if self.results.is_empty() {
            0.0
        } else {
            self.successful as f64 / self.results.len() as f64
        }
    }

    /// Check if all downloads succeeded (and there was at least one)
    pub fn is_all_success(&self) -> bool {
        !self.results.is_empty() && self.failed == 0
    }
}

/// Result of a paper read operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResult {
    /// Extracted text content
    pub text: String,

    /// Number of pages
    pub pages: Option<usize>,

    /// Whether the read was successful
    pub success: bool,

    /// Error message if failed
    pub error: Option<String>,
}

impl ReadResult {
    /// Create a successful read result
    pub fn success(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            pages: None,
            success: true,
            error: None,
        }
    }

    /// Set page count
    pub fn pages(mut self, pages: usize) -> Self {
        self.pages = Some(pages);
        self
    }

    /// Create a failed read result
    pub fn error(error: impl Into<String>) -> Self {
        Self {
            text: String::new(),
            pages: None,
            success: false,
            error: Some(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Paper, PaperBuilder, SourceType};

    #[test]
    fn test_batch_download_request_new() {
        let requests = vec![
            DownloadRequest::new("paper1", "/downloads"),
            DownloadRequest::new("paper2", "/downloads"),
        ];
        let batch = BatchDownloadRequest::new(requests);
        assert_eq!(batch.len(), 2);
        assert!(!batch.is_empty());
    }

    #[test]
    fn test_batch_download_request_add() {
        let mut batch = BatchDownloadRequest::new(vec![]);
        assert!(batch.is_empty());

        batch.add_request(DownloadRequest::new("paper1", "/downloads"));
        assert_eq!(batch.len(), 1);
    }

    #[test]
    fn test_batch_download_result_new() {
        let results = vec![
            DownloadResult::success("/path/to/paper1.pdf", 1024),
            DownloadResult::success("/path/to/paper2.pdf", 2048),
            DownloadResult::error("Failed to download"),
        ];

        let batch = BatchDownloadResult::new(results);

        assert_eq!(batch.successful, 2);
        assert_eq!(batch.failed, 1);
        assert_eq!(batch.total_bytes, 3072);
        assert!((batch.success_rate() - 0.666).abs() < 0.001);
        assert!(!batch.is_all_success());
    }

    #[test]
    fn test_batch_download_result_all_success() {
        let results = vec![
            DownloadResult::success("/path/to/paper1.pdf", 1024),
            DownloadResult::success("/path/to/paper2.pdf", 2048),
        ];

        let batch = BatchDownloadResult::new(results);

        assert_eq!(batch.successful, 2);
        assert_eq!(batch.failed, 0);
        assert_eq!(batch.success_rate(), 1.0);
        assert!(batch.is_all_success());
    }

    #[test]
    fn test_batch_download_result_empty() {
        let batch = BatchDownloadResult::new(vec![]);

        assert_eq!(batch.successful, 0);
        assert_eq!(batch.failed, 0);
        assert_eq!(batch.total_bytes, 0);
        assert_eq!(batch.success_rate(), 0.0);
        assert!(!batch.is_all_success());
    }

    #[test]
    fn license_policy_excludes_unknown_and_restricted_licenses() {
        assert!(LicensePolicy::Open.allows(Some("cc-by-4.0")));
        assert!(LicensePolicy::Open.allows(Some("https://creativecommons.org/licenses/by/4.0/")));
        for value in [
            None,
            Some("all rights reserved"),
            Some("cc-by-nc-4.0"),
            Some("arXiv nonexclusive-distrib 1.0"),
            Some("open access"),
        ] {
            assert!(!LicensePolicy::Open.allows(value));
            assert!(LicensePolicy::Ignore.allows(value));
        }
    }

    #[test]
    fn type_filter_exclusion_wins_over_inclusion() {
        let query = SearchQuery::new("crispr")
            .include_types(vec!["review".to_string()])
            .exclude_types(vec!["meta-analysis".to_string()]);

        let paper = PaperBuilder::new("1", "Review", "https://example.com", SourceType::OpenAlex)
            .paper_type("Meta-Analysis of CRISPR Reviews")
            .build();
        assert!(!query.type_filter_allows(&paper));

        let clean = PaperBuilder::new("2", "Review", "https://example.com", SourceType::OpenAlex)
            .paper_type("review")
            .build();
        assert!(query.type_filter_allows(&clean));

        // Exclusion applies even with no include list.
        let exclude_only = SearchQuery::new("crispr").exclude_types(vec!["editorial".to_string()]);
        let editorial = PaperBuilder::new("3", "Edit", "https://example.com", SourceType::OpenAlex)
            .paper_type("editorial")
            .build();
        assert!(!exclude_only.type_filter_allows(&editorial));
        assert!(exclude_only.type_filter_allows(&clean));
    }

    #[test]
    fn type_filter_is_lenient_for_papers_without_types_by_default() {
        let query = SearchQuery::new("crispr").include_types(vec!["review".to_string()]);
        let unclassified =
            PaperBuilder::new("1", "Paper", "https://example.com", SourceType::OpenAlex).build();
        assert!(query.type_filter_allows(&unclassified));
    }

    #[test]
    fn type_filter_strict_mode_rejects_unclassified_papers() {
        let query = SearchQuery::new("crispr")
            .include_types(vec!["review".to_string()])
            .strict_types(true);
        let unclassified =
            PaperBuilder::new("1", "Paper", "https://example.com", SourceType::OpenAlex).build();
        assert!(!query.type_filter_allows(&unclassified));

        // Strict mode only tightens a non-empty include list.
        let strict_no_include = SearchQuery::new("crispr").strict_types(true);
        assert!(strict_no_include.type_filter_allows(&unclassified));
    }

    #[test]
    fn type_filter_matches_case_insensitively() {
        let query = SearchQuery::new("crispr").include_types(vec!["ReVieW".to_string()]);
        let paper = PaperBuilder::new("1", "Paper", "https://example.com", SourceType::OpenAlex)
            .paper_type("review article")
            .build();
        assert!(query.type_filter_allows(&paper));

        let excluded = SearchQuery::new("crispr").exclude_types(vec!["CLINICAL".to_string()]);
        let trial = PaperBuilder::new("2", "Paper", "https://example.com", SourceType::OpenAlex)
            .paper_type("clinical trial")
            .build();
        assert!(!excluded.type_filter_allows(&trial));
    }

    #[test]
    fn type_filter_ignores_blank_and_trims_tokens() {
        let paper = PaperBuilder::new("1", "Paper", "https://example.com", SourceType::OpenAlex)
            .paper_type("review article")
            .build();
        let unclassified =
            PaperBuilder::new("2", "Paper", "https://example.com", SourceType::OpenAlex).build();
        assert!(SearchQuery::new("q")
            .exclude_types(vec!["".into()])
            .type_filter_allows(&paper));
        let blank = SearchQuery::new("q")
            .include_types(vec!["".into(), "  ".into()])
            .strict_types(true);
        assert!(blank.type_filter_allows(&paper));
        assert!(blank.type_filter_allows(&unclassified));
        assert!(SearchQuery::new("q")
            .include_types(vec![" ".into(), " review ".into()])
            .type_filter_allows(&paper));
        assert!(!SearchQuery::new("q")
            .exclude_types(vec![" ".into(), " review ".into()])
            .type_filter_allows(&paper));
    }

    #[test]
    fn type_filter_matches_category_text() {
        let query = SearchQuery::new("genomics").include_types(vec!["bioinformatics".to_string()]);
        let paper = PaperBuilder::new("1", "Paper", "https://example.com", SourceType::OpenAlex)
            .categories("Bioinformatics; Genomics")
            .build();
        assert!(query.type_filter_allows(&paper));

        let other = PaperBuilder::new("2", "Paper", "https://example.com", SourceType::OpenAlex)
            .categories("Ecology")
            .build();
        assert!(!query.type_filter_allows(&other));
    }

    #[test]
    fn test_search_query_new() {
        let query = SearchQuery::new("machine learning");
        assert_eq!(query.query, "machine learning");
        assert_eq!(query.max_results, 10); // default
        assert!(query.year.is_none());
        assert!(query.sort_by.is_none());
        assert!(query.sort_order.is_none());
    }

    #[test]
    fn test_search_query_with_options() {
        let query = SearchQuery::new("neural networks")
            .max_results(50)
            .year("2020-2023")
            .sort_by(SortBy::Relevance)
            .sort_order(SortOrder::Descending);

        assert_eq!(query.query, "neural networks");
        assert_eq!(query.max_results, 50);
        assert_eq!(query.year, Some("2020-2023".to_string()));
        assert_eq!(query.sort_by, Some(SortBy::Relevance));
        assert_eq!(query.sort_order, Some(SortOrder::Descending));
    }

    #[test]
    fn test_search_query_builder_pattern() {
        let query = SearchQuery::new("deep learning")
            .max_results(100)
            .author("John Doe")
            .category("cs.AI")
            .year("2022");

        assert_eq!(query.query, "deep learning");
        assert_eq!(query.max_results, 100);
        assert_eq!(query.author, Some("John Doe".to_string()));
        assert_eq!(query.category, Some("cs.AI".to_string()));
        assert_eq!(query.year, Some("2022".to_string()));
    }

    #[test]
    fn test_search_query_year_formats() {
        let single_year = SearchQuery::new("test").year("2020");
        assert_eq!(single_year.year, Some("2020".to_string()));

        let year_range = SearchQuery::new("test").year("2019-2023");
        assert_eq!(year_range.year, Some("2019-2023".to_string()));

        let from_year = SearchQuery::new("test").year("2020-");
        assert_eq!(from_year.year, Some("2020-".to_string()));
    }

    #[test]
    fn test_year_bounds_parses_all_documented_forms() {
        assert_eq!(year_bounds("2020"), Some((2020, 2020)));
        assert_eq!(year_bounds("2018-2022"), Some((2018, 2022)));
        assert_eq!(year_bounds("2010-"), Some((2010, 9999)));
        assert_eq!(year_bounds("-2015"), Some((1000, 2015)));
        assert_eq!(year_bounds("2023 "), Some((2023, 2023)));
        assert_eq!(year_bounds("2024-2022"), None);
        assert_eq!(year_bounds("abc"), None);
        assert_eq!(year_bounds(""), None);
    }

    #[test]
    fn test_search_response_new() {
        let papers = vec![
            Paper::new(
                "1".to_string(),
                "Paper 1".to_string(),
                "url1".to_string(),
                SourceType::Arxiv,
            ),
            Paper::new(
                "2".to_string(),
                "Paper 2".to_string(),
                "url2".to_string(),
                SourceType::Arxiv,
            ),
        ];
        let response = SearchResponse::new(papers, "test source", "search term");

        assert_eq!(response.papers.len(), 2);
        assert_eq!(response.source, "test source");
        assert_eq!(response.query, "search term");
        // total_results is None by default, set via builder method
        assert!(response.total_results.is_none());
    }

    #[test]
    fn test_search_response_with_total() {
        let papers = vec![Paper::new(
            "1".to_string(),
            "Paper 1".to_string(),
            "url1".to_string(),
            SourceType::Arxiv,
        )];
        let response = SearchResponse::new(papers, "test source", "search term").total_results(100);

        assert_eq!(response.total_results, Some(100));
    }

    #[test]
    fn test_search_response_empty() {
        let response = SearchResponse::new(vec![], "test source", "search term");
        assert!(response.papers.is_empty());
        // total_results is None by default
        assert!(response.total_results.is_none());
    }

    #[test]
    fn test_citation_request_new() {
        let request = CitationRequest::new("paper123");
        assert_eq!(request.paper_id, "paper123");
        assert_eq!(request.max_results, 20); // default
    }

    #[test]
    fn test_citation_request_with_options() {
        let request = CitationRequest::new("paper456").max_results(50);

        assert_eq!(request.paper_id, "paper456");
        assert_eq!(request.max_results, 50);
    }

    #[test]
    fn test_download_request_new() {
        let request = DownloadRequest::new("paper123", "/downloads");
        assert_eq!(request.paper_id, "paper123");
        assert_eq!(request.save_path, "/downloads");
    }

    #[test]
    fn test_download_result_success() {
        let result = DownloadResult::success("/path/to/file.pdf", 1024);
        assert!(result.success);
        assert_eq!(result.path, "/path/to/file.pdf");
        assert_eq!(result.bytes, 1024);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_download_result_error() {
        let result = DownloadResult::error("Network timeout");
        assert!(!result.success);
        assert!(result.path.is_empty());
        assert_eq!(result.bytes, 0);
        assert_eq!(result.error, Some("Network timeout".to_string()));
    }

    #[test]
    fn test_read_request_new() {
        let request = ReadRequest::new("123", "/papers");

        assert_eq!(request.paper_id, "123");
        assert_eq!(request.save_path, "/papers");
        assert!(request.download_if_missing);
    }

    #[test]
    fn test_read_request_with_download_option() {
        let request = ReadRequest::new("123", "/papers").download_if_missing(false);

        assert!(!request.download_if_missing);
    }

    #[test]
    fn test_read_result_new() {
        let result = ReadResult::success("Extracted text content");
        assert_eq!(result.text, "Extracted text content");
        assert!(result.success);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_read_result_with_pages() {
        let result = ReadResult::success("Text".to_string()).pages(5);
        assert_eq!(result.pages, Some(5));
    }

    #[test]
    fn test_sort_by_variants() {
        // SortBy uses Debug formatting via derive
        assert_eq!(format!("{:?}", SortBy::Relevance), "Relevance");
        assert_eq!(format!("{:?}", SortBy::Date), "Date");
        assert_eq!(format!("{:?}", SortBy::CitationCount), "CitationCount");
        assert_eq!(format!("{:?}", SortBy::Title), "Title");
        assert_eq!(format!("{:?}", SortBy::Author), "Author");
    }

    #[test]
    fn test_sort_order_variants() {
        // SortOrder uses Debug formatting via derive
        assert_eq!(format!("{:?}", SortOrder::Descending), "Descending");
        assert_eq!(format!("{:?}", SortOrder::Ascending), "Ascending");
    }
}
