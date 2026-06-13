//! Integration tests for Research Master
//!
//! These tests verify the full functionality of the MCP server and research sources.

use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use research_master::mcp::{server::McpServer, ToolRegistry};
use research_master::models::{
    CitationRequest, Paper, PaperBuilder, SearchQuery, SearchResponse, SourceType,
};
use research_master::sources::{Source, SourceCapabilities, SourceError, SourceRegistry};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

const TEST_API_KEY_VARS: &[&str] = &[
    "ACM_API_KEY",
    "SPRINGER_API_KEY",
    "MDPI_API_KEY",
    "IEEE_XPLORE_API_KEY",
    "JSTOR_API_KEY",
    "RESEARCH_MASTER_DEFAULT_DISABLED_SOURCES",
];

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Set up environment variables for sources that require API keys
/// This allows tests to pass even when API keys are not set in the environment
fn setup_api_keys() {
    // Set dummy API keys for sources that require them
    std::env::set_var("ACM_API_KEY", "test_key_for_integration_tests");
    std::env::set_var("SPRINGER_API_KEY", "test_key_for_integration_tests");
    std::env::set_var("MDPI_API_KEY", "test_key_for_integration_tests");
    std::env::set_var("IEEE_XPLORE_API_KEY", "test_key_for_integration_tests");
    std::env::set_var("JSTOR_API_KEY", "test_key_for_integration_tests");
    // Disable default disabled sources so all sources are available for testing
    std::env::set_var("RESEARCH_MASTER_DEFAULT_DISABLED_SOURCES", "");
}

fn restore_env(saved: Vec<(&'static str, Option<String>)>) {
    for (key, value) in saved {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

/// Wrapper to run tests with API keys set
fn with_api_keys<F, R>(test: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = env_lock().lock().expect("env lock poisoned");
    let saved = TEST_API_KEY_VARS
        .iter()
        .map(|&key| (key, std::env::var(key).ok()))
        .collect();

    setup_api_keys();
    let result = test();
    restore_env(saved);
    result
}

fn expected_source_count() -> usize {
    let mut count = 0;

    if cfg!(feature = "source-arxiv") {
        count += 1;
    }
    if cfg!(feature = "source-pubmed") {
        count += 1;
    }
    if cfg!(feature = "source-biorxiv") {
        count += 1;
    }
    if cfg!(feature = "source-semantic") {
        count += 1;
    }
    if cfg!(feature = "source-openalex") {
        count += 1;
    }
    if cfg!(feature = "source-crossref") {
        count += 1;
    }
    if cfg!(feature = "source-iacr") {
        count += 1;
    }
    if cfg!(feature = "source-pmc") {
        count += 1;
    }
    if cfg!(feature = "source-hal") {
        count += 1;
    }
    if cfg!(feature = "source-dblp") {
        count += 1;
    }
    if cfg!(feature = "source-ssrn") {
        count += 1;
    }
    if cfg!(feature = "source-dimensions") {
        count += 1;
    }
    if cfg!(feature = "source-ieee_xplore") {
        count += 1;
    }
    if cfg!(feature = "source-europe_pmc") {
        count += 1;
    }
    if cfg!(feature = "source-core-repo") {
        count += 1;
    }
    if cfg!(feature = "source-zenodo") {
        count += 1;
    }
    if cfg!(feature = "source-unpaywall") {
        count += 1;
    }
    if cfg!(feature = "source-mdpi") {
        count += 1;
    }
    if cfg!(feature = "source-jstor") {
        count += 1;
    }
    if cfg!(feature = "source-scispace") {
        count += 1;
    }
    if cfg!(feature = "source-acm") {
        count += 1;
    }
    if cfg!(feature = "source-connected_papers") {
        count += 1;
    }
    if cfg!(feature = "source-doaj") {
        count += 1;
    }
    if cfg!(feature = "source-worldwidescience") {
        count += 1;
    }
    if cfg!(feature = "source-osf") {
        count += 1;
    }
    if cfg!(feature = "source-base") {
        count += 1;
    }
    if cfg!(feature = "source-springer") {
        count += 1;
    }
    if cfg!(feature = "source-citeseerx") {
        count += 1;
    }
    if cfg!(feature = "source-medrxiv") {
        count += 1;
    }
    if cfg!(feature = "source-openaire") {
        count += 1;
    }
    if cfg!(feature = "source-orcid") {
        count += 1;
    }
    if cfg!(feature = "source-google_scholar") {
        count += 1;
    }

    count
}

/// Test that the server can be created successfully
#[tokio::test]
async fn test_server_initialization() {
    with_api_keys(|| {
        let registry = SourceRegistry::new();
        let server = McpServer::new(Arc::new(registry));
        assert!(server.is_ok());
    });
}

/// Test that all sources are registered
#[tokio::test]
async fn test_all_sources_registered() {
    with_api_keys(|| {
        let registry = SourceRegistry::new();
        let sources: Vec<_> = registry.all().collect();

        let expected = expected_source_count();
        assert_eq!(sources.len(), expected);

        // Check each source exists
        let source_ids: Vec<&str> = sources.iter().map(|s| s.id()).collect();
        assert!(source_ids.contains(&"arxiv"));
        assert!(source_ids.contains(&"pubmed"));
        assert!(source_ids.contains(&"biorxiv"));
        assert!(source_ids.contains(&"semantic"));
        assert!(source_ids.contains(&"openalex"));
        assert!(source_ids.contains(&"crossref"));
        assert!(source_ids.contains(&"iacr"));
        assert!(source_ids.contains(&"pmc"));
        assert!(source_ids.contains(&"hal"));
        assert!(source_ids.contains(&"dblp"));
        assert!(source_ids.contains(&"ssrn"));
        assert!(source_ids.contains(&"dimensions"));
        assert!(source_ids.contains(&"ieee_xplore"));
        if cfg!(feature = "source-europe_pmc") {
            assert!(source_ids.contains(&"europe_pmc"));
        } else {
            assert!(!source_ids.contains(&"europe_pmc"));
        }
        assert!(source_ids.contains(&"core"));
        assert!(source_ids.contains(&"zenodo"));
        assert!(source_ids.contains(&"unpaywall"));
        assert!(source_ids.contains(&"mdpi"));
        assert!(source_ids.contains(&"jstor"));
        assert!(source_ids.contains(&"scispace"));
        assert!(source_ids.contains(&"acm"));
        assert!(source_ids.contains(&"connected_papers"));
        assert!(source_ids.contains(&"doaj"));
        assert!(source_ids.contains(&"worldwidescience"));
        assert!(source_ids.contains(&"osf"));
        assert!(source_ids.contains(&"base"));
        assert!(source_ids.contains(&"springer"));
        if cfg!(feature = "source-google_scholar") {
            assert!(source_ids.contains(&"google_scholar"));
        } else {
            assert!(!source_ids.contains(&"google_scholar"));
        }
    });
}

/// Test source capabilities are properly reported
#[tokio::test]
async fn test_source_capabilities() {
    let registry = SourceRegistry::new();

    // arXiv should support search, download, and read
    let arxiv = registry.get("arxiv");
    assert!(arxiv.is_some());
    let caps = arxiv.unwrap().capabilities();
    assert!(caps.contains(SourceCapabilities::SEARCH));
    assert!(caps.contains(SourceCapabilities::DOWNLOAD));
    assert!(caps.contains(SourceCapabilities::READ));

    // CrossRef should support search and DOI lookup
    let crossref = registry.get("crossref");
    assert!(crossref.is_some());
    let caps = crossref.unwrap().capabilities();
    assert!(caps.contains(SourceCapabilities::SEARCH));
    assert!(caps.contains(SourceCapabilities::DOI_LOOKUP));
}

/// Test SearchQuery builder
#[test]
fn test_search_query_builder() {
    let query = SearchQuery::new("machine learning")
        .max_results(20)
        .year("2020-")
        .author("Hinton");

    assert_eq!(query.query, "machine learning");
    assert_eq!(query.max_results, 20);
    assert_eq!(query.year, Some("2020-".to_string()));
    assert_eq!(query.author, Some("Hinton".to_string()));
}

/// Test SourceType display and serialization
#[test]
fn test_source_type() {
    assert_eq!(SourceType::Arxiv.to_string(), "arXiv");
    assert_eq!(SourceType::PubMed.to_string(), "PubMed");
    assert_eq!(SourceType::SemanticScholar.to_string(), "Semantic Scholar");
}

/// Test error handling for invalid queries
#[test]
fn test_invalid_query_handling() {
    // Empty query should still be valid (some sources support listing)
    let query = SearchQuery::new("").max_results(10);
    assert_eq!(query.query, "");
    assert_eq!(query.max_results, 10);

    // Very large max_results should be accepted
    let query = SearchQuery::new("test").max_results(10000);
    assert_eq!(query.max_results, 10000);
}

/// Test source retrieval by name
#[tokio::test]
async fn test_get_source_by_name() {
    with_api_keys(|| {
        let registry = SourceRegistry::new();

        // Test getting existing sources
        assert!(registry.get("arxiv").is_some());
        assert!(registry.get("pubmed").is_some());
        assert!(registry.get("semantic").is_some());

        // Test getting non-existent source
        assert!(registry.get("nonexistent").is_none());
    });
}

/// Test getting sources by capability
#[tokio::test]
async fn test_get_sources_by_capability() {
    with_api_keys(|| {
        let registry = SourceRegistry::new();

        // Get all searchable sources
        let searchable = registry.with_capability(SourceCapabilities::SEARCH);

        assert!(!searchable.is_empty());
        assert!(searchable.len() >= 8); // At least 8 sources should support search

        // Get all DOI lookup sources
        let doi_lookup = registry.with_capability(SourceCapabilities::DOI_LOOKUP);

        assert!(!doi_lookup.is_empty());
    });
}

/// Test helper methods on registry
#[tokio::test]
async fn test_registry_helper_methods() {
    with_api_keys(|| {
        let registry = SourceRegistry::new();

        // Test has() method
        assert!(registry.has("arxiv"));
        assert!(!registry.has("nonexistent"));

        // Test len() method - should match enabled feature set
        assert_eq!(registry.len(), expected_source_count());

        // Test is_empty() method
        assert!(!registry.is_empty());

        // Test searchable() helper
        let searchable = registry.searchable();
        assert!(!searchable.is_empty());

        // Test downloadable() helper
        let downloadable = registry.downloadable();
        assert!(!downloadable.is_empty());
    });
}

/// Test source metadata
#[tokio::test]
async fn test_source_metadata() {
    let registry = SourceRegistry::new();
    let arxiv = registry.get("arxiv").unwrap();

    assert_eq!(arxiv.id(), "arxiv");
    assert_eq!(arxiv.name(), "arXiv");
}

/// Test Paper model
#[test]
fn test_paper_model() {
    use research_master::models::PaperBuilder;

    let paper = PaperBuilder::new(
        "1234.5678",
        "Test Paper",
        "https://example.com",
        SourceType::Arxiv,
    )
    .authors("John Doe; Jane Smith")
    .abstract_text("This is a test abstract.")
    .doi("10.1234/test")
    .published_date("2024")
    .citations(42)
    .build();

    assert_eq!(paper.paper_id, "1234.5678");
    assert_eq!(paper.title, "Test Paper");
    assert_eq!(paper.authors, "John Doe; Jane Smith");
    assert_eq!(paper.r#abstract, "This is a test abstract.");
    assert_eq!(paper.doi, Some("10.1234/test".to_string()));
    assert_eq!(paper.citations, Some(42));

    // Test helper methods
    assert_eq!(paper.primary_id(), "10.1234/test");
    assert_eq!(paper.author_list(), vec!["John Doe", "Jane Smith"]);
    assert!(!paper.has_pdf()); // No PDF URL set
}

/// Test Paper with PDF
#[test]
fn test_paper_with_pdf() {
    use research_master::models::PaperBuilder;

    let paper = PaperBuilder::new("1234", "Test", "https://example.com", SourceType::Arxiv)
        .pdf_url("https://example.com/paper.pdf")
        .build();

    assert!(paper.has_pdf());
    assert_eq!(
        paper.pdf_url,
        Some("https://example.com/paper.pdf".to_string())
    );
}

/// Test Paper categories and keywords
#[test]
fn test_paper_categories_keywords() {
    use research_master::models::PaperBuilder;

    let paper = PaperBuilder::new("1234", "Test", "https://example.com", SourceType::Arxiv)
        .categories("Machine Learning; AI")
        .keywords("deep learning; neural networks")
        .build();

    assert_eq!(paper.category_list(), vec!["Machine Learning", "AI"]);
    assert_eq!(
        paper.keyword_list(),
        vec!["deep learning", "neural networks"]
    );
}

/// Test registry ids() iterator
#[tokio::test]
async fn test_registry_ids() {
    let registry = SourceRegistry::new();
    let ids: Vec<&str> = registry.ids().collect();

    assert_eq!(ids.len(), expected_source_count());
    assert!(ids.contains(&"arxiv"));
    assert!(ids.contains(&"pubmed"));
    assert!(ids.contains(&"semantic"));
}

#[derive(Debug, Clone)]
struct IntegrationMockSource {
    id: String,
    name: String,
    capabilities: SourceCapabilities,
    search_responses: Arc<Mutex<VecDeque<SearchResponse>>>,
    citation_responses: Arc<Mutex<VecDeque<SearchResponse>>>,
    reference_responses: Arc<Mutex<VecDeque<SearchResponse>>>,
}

impl IntegrationMockSource {
    fn new(id: &str, capabilities: SourceCapabilities) -> Self {
        Self {
            id: id.to_string(),
            name: format!("Mock {id}"),
            capabilities,
            search_responses: Arc::new(Mutex::new(VecDeque::new())),
            citation_responses: Arc::new(Mutex::new(VecDeque::new())),
            reference_responses: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    fn push_search_response(&self, response: SearchResponse) {
        self.search_responses
            .lock()
            .expect("search response lock poisoned")
            .push_back(response);
    }

    fn push_citation_response(&self, response: SearchResponse) {
        self.citation_responses
            .lock()
            .expect("citation response lock poisoned")
            .push_back(response);
    }

    fn push_reference_response(&self, response: SearchResponse) {
        self.reference_responses
            .lock()
            .expect("reference response lock poisoned")
            .push_back(response);
    }

    fn empty_response(&self, query: &str) -> SearchResponse {
        SearchResponse::new(Vec::new(), self.name.clone(), query.to_string())
    }
}

#[async_trait]
impl Source for IntegrationMockSource {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> SourceCapabilities {
        self.capabilities
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        Ok(self
            .search_responses
            .lock()
            .expect("search response lock poisoned")
            .pop_front()
            .unwrap_or_else(|| self.empty_response(&query.query)))
    }

    async fn get_citations(
        &self,
        request: &CitationRequest,
    ) -> Result<SearchResponse, SourceError> {
        Ok(self
            .citation_responses
            .lock()
            .expect("citation response lock poisoned")
            .pop_front()
            .unwrap_or_else(|| self.empty_response(&request.paper_id)))
    }

    async fn get_references(
        &self,
        request: &CitationRequest,
    ) -> Result<SearchResponse, SourceError> {
        Ok(self
            .reference_responses
            .lock()
            .expect("reference response lock poisoned")
            .pop_front()
            .unwrap_or_else(|| self.empty_response(&request.paper_id)))
    }

    async fn get_related(&self, request: &CitationRequest) -> Result<SearchResponse, SourceError> {
        self.get_citations(request).await
    }

    async fn get_by_doi(&self, doi: &str) -> Result<Paper, SourceError> {
        Ok(make_mock_paper("doi-paper", "DOI Paper", "2024")
            .doi(doi)
            .build())
    }
}

fn make_mock_paper(paper_id: &str, title: &str, year: &str) -> PaperBuilder {
    PaperBuilder::new(
        paper_id,
        title,
        format!("https://example.com/{paper_id}"),
        SourceType::Arxiv,
    )
    .authors("Doe, Jane; Smith, John")
    .abstract_text("Mock abstract for integration testing.")
    .published_date(year)
}

fn expected_mcp_tool_names() -> Vec<&'static str> {
    vec![
        "search_papers",
        "search_by_author",
        "get_paper",
        "download_paper",
        "read_paper",
        "get_citations",
        "get_references",
        "lookup_by_doi",
        "deduplicate_papers",
        "get_related_papers",
        "list_sources",
        "author_profile",
        "batch_get_papers",
        "citation_graph",
        "export_papers",
        "recommend_papers",
        "write_paper",
        "web_search",
    ]
}

/// E2E: invoke an MCP tool through ToolRegistry using a local mock source.
#[tokio::test]
async fn test_mock_mcp_tool_invocation_search_papers() {
    let mock = IntegrationMockSource::new(
        "mock",
        SourceCapabilities::SEARCH | SourceCapabilities::CITATIONS,
    );
    mock.push_search_response(SearchResponse::new(
        vec![make_mock_paper("mock-1", "Mock MCP Paper", "2024").build()],
        "mock",
        "integration",
    ));
    mock.push_citation_response(SearchResponse::new(
        vec![make_mock_paper("cite-mock-1", "Mock Citing Paper", "2023").build()],
        "mock",
        "mock-1",
    ));
    mock.push_reference_response(SearchResponse::new(
        vec![make_mock_paper("ref-mock-1", "Mock Referenced Paper", "2022").build()],
        "mock",
        "mock-1",
    ));

    let tools = with_api_keys(|| {
        let mut registry = SourceRegistry::new();
        registry.register(Arc::new(mock));
        ToolRegistry::from_sources(&registry)
    });

    let result = tools
        .execute(
            "search_papers",
            serde_json::json!({
                "query": "integration",
                "source": "mock",
                "max_results": 5
            }),
        )
        .await
        .expect("mock search_papers tool should execute");

    let papers = result.as_array().expect("tool output should be an array");
    assert_eq!(papers.len(), 1);
    assert_eq!(papers[0]["paper_id"], "mock-1");
    assert_eq!(papers[0]["title"], "Mock MCP Paper");

    let citations = tools
        .execute(
            "get_citations",
            serde_json::json!({
                "paper_id": "mock-1",
                "source": "mock",
                "max_results": 1
            }),
        )
        .await
        .expect("mock get_citations tool should execute");
    assert_eq!(citations["papers"][0]["paper_id"], "cite-mock-1");

    let references = tools
        .execute(
            "get_references",
            serde_json::json!({
                "paper_id": "mock-1",
                "source": "mock",
                "max_results": 1
            }),
        )
        .await
        .expect("mock get_references tool should execute");
    assert_eq!(references["papers"][0]["paper_id"], "ref-mock-1");
}

/// E2E: every unified MCP tool is registered in ToolRegistry.
#[tokio::test]
async fn test_all_mcp_tools_registered() {
    let tools = with_api_keys(|| {
        let registry = SourceRegistry::new();
        ToolRegistry::from_sources(&registry)
    });

    let expected = expected_mcp_tool_names();
    let registered = tools.all();

    assert_eq!(
        registered.len(),
        expected.len(),
        "ToolRegistry should expose every unified MCP tool currently implemented"
    );

    for name in expected {
        assert!(tools.get(name).is_some(), "missing MCP tool: {name}");
    }
}

/// E2E: paper_stream yields papers from a cloneable mock source and stops on empty page.
#[tokio::test]
async fn test_paper_stream_with_mock_source() {
    let mock = IntegrationMockSource::new("mock-stream", SourceCapabilities::SEARCH);
    mock.push_search_response(SearchResponse::new(
        vec![
            make_mock_paper("stream-1", "Stream Paper 1", "2023").build(),
            make_mock_paper("stream-2", "Stream Paper 2", "2024").build(),
        ],
        "mock-stream",
        "streaming",
    ));

    let paper_stream = research_master::utils::paper_stream(mock, SearchQuery::new("streaming"), 2);
    let papers: Vec<Paper> = Box::pin(paper_stream).collect().await;

    assert_eq!(papers.len(), 2);
    assert_eq!(papers[0].paper_id, "stream-1");
    assert_eq!(papers[1].paper_id, "stream-2");
}

/// E2E: filter_by_year transforms an input stream and preserves only papers in range.
#[tokio::test]
async fn test_filter_by_year_stream_transformation() {
    let papers = vec![
        make_mock_paper("old", "Old Paper", "2019-01-01").build(),
        make_mock_paper("inside-a", "Inside Paper A", "2021").build(),
        make_mock_paper("inside-b", "Inside Paper B", "2023/05/01").build(),
        make_mock_paper("future", "Future Paper", "2025").build(),
        make_mock_paper("undated", "Undated Paper", "not-a-date").build(),
    ];

    let filtered =
        research_master::utils::filter_by_year(stream::iter(papers), Some(2020), Some(2023));
    let filtered = research_master::utils::collect_papers(filtered).await;
    let ids: Vec<&str> = filtered
        .iter()
        .map(|paper| paper.paper_id.as_str())
        .collect();

    assert_eq!(ids, vec!["inside-a", "inside-b", "undated"]);
}

/// E2E: citation formatting service produces structured and formatted citations from mock data.
#[test]
fn test_citation_service_with_mock_data() {
    let paper = make_mock_paper(
        "cite-1",
        "Citation Testing in Integration Tests",
        "2024-03-15",
    )
    .doi("10.1234/integration")
    .build();

    let formatted =
        research_master::utils::format_citation(&paper, research_master::utils::CitationStyle::Apa);
    let structured = research_master::utils::get_structured_citation(
        &paper,
        research_master::utils::CitationStyle::Mla,
    );

    assert!(formatted.contains("Citation Testing in Integration Tests"));
    assert!(formatted.contains("2024"));
    assert_eq!(structured.style, "Mla");
    assert_eq!(structured.title, "Citation Testing in Integration Tests");
    assert_eq!(structured.year, "2024");
    assert_eq!(structured.doi.as_deref(), Some("10.1234/integration"));
}

/// E2E: get_config returns a valid default/test configuration.
#[test]
fn test_get_config_returns_valid_config() {
    let _guard = env_lock().lock().expect("env lock poisoned");
    let saved = vec![
        (
            "RESEARCH_MASTER_TEST_MODE",
            std::env::var("RESEARCH_MASTER_TEST_MODE").ok(),
        ),
        (
            "RESEARCH_MASTER_CACHE_ENABLED",
            std::env::var("RESEARCH_MASTER_CACHE_ENABLED").ok(),
        ),
    ];

    std::env::set_var("RESEARCH_MASTER_TEST_MODE", "true");
    std::env::remove_var("RESEARCH_MASTER_CACHE_ENABLED");

    let config = research_master::config::get_config();
    assert!(config.downloads.organize_by_source);
    assert_eq!(config.downloads.max_file_size_mb, 100);
    assert_eq!(config.rate_limits.default_requests_per_second, 5.0);
    assert_eq!(config.rate_limits.max_concurrent_requests, 10);
    assert_eq!(config.cache.search_ttl_seconds, 1800);
    assert_eq!(config.cache.citation_ttl_seconds, 900);

    restore_env(saved);
}

/// E2E: cache service initializes directories and can round-trip cached search data.
#[test]
fn test_cache_service_initialization() {
    let temp_dir = tempfile::TempDir::new().expect("temp cache directory should be created");
    let cache =
        research_master::utils::CacheService::from_config(research_master::config::CacheConfig {
            enabled: true,
            directory: Some(temp_dir.path().to_path_buf()),
            search_ttl_seconds: 60,
            citation_ttl_seconds: 60,
            max_size_mb: 10,
        });

    cache
        .initialize()
        .expect("cache directories should initialize");
    assert!(cache.is_enabled());
    assert_eq!(cache.cache_dir(), temp_dir.path());
    assert!(temp_dir.path().join("searches").is_dir());
    assert!(temp_dir.path().join("citations").is_dir());

    let query = SearchQuery::new("cache integration");
    let response = SearchResponse::new(
        vec![make_mock_paper("cache-1", "Cached Paper", "2024").build()],
        "mock-cache",
        "cache integration",
    );
    cache.set_search("mock-cache", &query, &response);

    match cache.get_search(&query, "mock-cache") {
        research_master::utils::CacheResult::Hit(hit) => {
            assert_eq!(hit.papers.len(), 1);
            assert_eq!(hit.papers[0].paper_id, "cache-1");
        }
        _ => panic!("expected cache hit after set_search"),
    }

    let stats = cache.stats();
    assert!(stats.enabled);
    assert_eq!(stats.search_count, 1);
}

/// E2E: circuit breaker moves through closed/open/rejected/reset states.
#[tokio::test]
async fn test_circuit_breaker_state_machine_manual() {
    let breaker = research_master::utils::CircuitBreaker::new(
        "integration-source",
        2,
        std::time::Duration::from_secs(60),
    );

    assert_eq!(
        breaker.state(),
        research_master::utils::CircuitState::Closed
    );
    assert!(breaker.can_request());

    breaker.record_failure();
    assert_eq!(
        breaker.state(),
        research_master::utils::CircuitState::Closed
    );

    breaker.record_failure();
    assert_eq!(breaker.state(), research_master::utils::CircuitState::Open);
    assert!(!breaker.can_request());

    let result = breaker.execute(async { Ok::<_, &str>(()) }).await;
    assert!(result.is_rejected());

    breaker.reset();
    assert_eq!(
        breaker.state(),
        research_master::utils::CircuitState::Closed
    );
    assert!(breaker.can_request());

    let result = breaker.execute(async { Ok::<_, &str>("recovered") }).await;
    assert!(result.is_success());
    assert_eq!(result.unwrap(), "recovered");
}

/// E2E: progress reporter accepts dummy values without terminal output.
#[test]
fn test_progress_reporter_with_dummy_values() {
    let reporter = research_master::utils::ProgressReporter::quiet("integration-progress", 10);

    assert_eq!(reporter.current(), 0);
    assert!(!reporter.is_done());

    reporter.inc();
    reporter.inc_by(4);
    assert_eq!(reporter.current(), 5);

    reporter.set(10);
    assert_eq!(reporter.current(), 10);
    assert!(reporter.is_done());

    reporter.finish();
}

/// E2E: validation utilities accept safe values and reject dangerous ones.
#[test]
fn test_validate_utilities_work() {
    assert_eq!(
        research_master::utils::validate_doi("https://doi.org/10.1234/Test.DOI").unwrap(),
        "10.1234/test.doi"
    );
    assert!(research_master::utils::validate_doi("not-a-doi").is_err());

    assert_eq!(
        research_master::utils::validate_url("https://example.com/paper?id=123").unwrap(),
        "https://example.com/paper?id=123"
    );
    assert!(research_master::utils::validate_url("http://127.0.0.1/admin").is_err());

    assert_eq!(
        research_master::utils::sanitize_filename("A Safe Paper 2024.pdf").unwrap(),
        "A Safe Paper 2024.pdf"
    );
    assert!(research_master::utils::sanitize_filename("../secret.pdf").is_err());
}
