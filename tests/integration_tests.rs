//! Integration tests for Research Master
//!
//! These tests verify the full functionality of the MCP server and research sources.

use research_master::mcp::server::McpServer;
use research_master::models::{SearchQuery, SourceType};
use research_master::sources::{SourceCapabilities, SourceRegistry};
use std::sync::Arc;

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

/// Clean up API key environment variables after tests
fn cleanup_api_keys() {
    std::env::remove_var("ACM_API_KEY");
    std::env::remove_var("SPRINGER_API_KEY");
    std::env::remove_var("MDPI_API_KEY");
    std::env::remove_var("IEEE_XPLORE_API_KEY");
    std::env::remove_var("JSTOR_API_KEY");
    std::env::remove_var("RESEARCH_MASTER_DEFAULT_DISABLED_SOURCES");
}

/// Wrapper to run tests with API keys set
fn with_api_keys<F>(test: F)
where
    F: FnOnce(),
{
    setup_api_keys();
    test();
    cleanup_api_keys();
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
