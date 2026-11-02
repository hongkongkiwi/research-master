# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Research Master MCP is a Model Context Protocol (MCP) server that provides unified access to 36 academic research sources (32 enabled by default; arXiv, Semantic Scholar, PubMed, OpenAlex, Europe PMC, DataCite, INSPIRE-HEP, NASA ADS, etc.) for searching, downloading, and analyzing academic papers.

## Build & Test Commands

```bash
# Build the project
cargo build

# Build with specific features (e.g., only core sources)
cargo build --no-default-features --features core

# Build with specific sources
cargo build --no-default-features --features arxiv,semantic,openalex

# Build with all sources except one
cargo build --features -dblp

# Run all tests
cargo test

# Run a specific test
cargo test test_name

# Run doc tests
cargo test --doc

# Release build
cargo build --release

# Check for clippy warnings
cargo clippy

# Format code
cargo fmt
```

## Architecture

### Core Modules

- **`src/lib.rs`** - Library root, exports public API
- **`src/main.rs`** - CLI entry point using clap for argument parsing
- **`src/sources/`** - Plugin-based architecture with 36 research sources
- **`src/mcp/`** - MCP protocol server implementation (42 unified tools)
- **`src/models/`** - Core data structures (Paper, SearchQuery, etc.)
- **`src/panel.rs`** - Local web panel for browsing search and download history
- **`src/config/`** - Environment and file-based configuration
- **`src/utils/`** - HTTP client, deduplication, PDF extraction

### Source Plugin System

All sources implement the [`Source`](src/sources/mod.rs:138) trait from `src/sources/mod.rs`. Key aspects:

1. **Capability-based design**: Each source declares capabilities via `SourceCapabilities` bitflags (SEARCH, DOWNLOAD, READ, CITATIONS, DOI_LOOKUP, AUTHOR_SEARCH)

2. **Feature-gated modules**: Sources are conditionally compiled via `#[cfg(feature = "source-xxx")]` in `src/sources/mod.rs`

3. **Registration in registry**: Sources are registered in `src/sources/registry.rs` via `try_register!()` macro

4. **Unified tool handlers**: `src/mcp/unified_tools.rs` provides smart source auto-detection for paper IDs (e.g., arXiv IDs, DOIs, PMC IDs)

### Data Flow

1. **CLI/MCP Request** → `main.rs` parses args or MCP receives JSON-RPC
2. **Unified Tool Handler** → `src/mcp/unified_tools.rs` determines appropriate source(s)
3. **Source Implementation** → Specific source (e.g., `arxiv.rs`) makes HTTP API calls
4. **Response Parsing** → Deserialize JSON/XML to `Paper` models
5. **Deduplication** → Optional Jaro-Winkler similarity matching in `src/utils/dedup.rs`

### HTTP Client

`src/utils/http.rs` provides a rate-limited HTTP client using the `governor` crate for GCRA (Generic Cell Rate Limiting). Each source has configurable rate limits via environment variables.

## Key Patterns

### Source Implementation Pattern

```rust
#[derive(Debug)]
pub struct MySource;

#[async_trait]
impl Source for MySource {
    fn id(&self) -> &str { "mysource" }
    fn name(&self) -> &str { "My Source" }
    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::SEARCH | SourceCapabilities::DOWNLOAD
    }

    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SourceError> {
        // Implementation
    }
}
```

### Error Handling

Sources use `SourceError` enum with conversion implementations from `reqwest::Error`, `serde_json::Error`, and `quick_xml::DeError`.

## Important Configuration

- **Google Scholar**: Disabled by default, requires both `--features google_scholar` at compile time AND `GOOGLE_SCHOLAR_ENABLED=true` at runtime
- **Opt-in sources**: SciSpace, Connected Papers, and WorldWideScience are excluded from the default build (their public endpoints no longer resolve); build with `--features scispace,connected_papers,worldwidescience` to include them
- **API Keys**: Optional for most sources (rate limits apply). Keys via environment variables (e.g., `SEMANTIC_SCHOLAR_API_KEY`). IEEE Xplore, ACM, Springer, and NASA ADS require keys and fail with an error naming the key and signup URL when missing
- **Poppler**: Required for PDF text extraction (`brew install poppler` on macOS, `apt-get install libpoppler-cpp-dev` on Ubuntu)

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `RESEARCH_MASTER_USER_AGENT` | Custom User-Agent header for all HTTP requests | `research-master/{version}` |
| `SEMANTIC_SCHOLAR_API_KEY` | API key for higher rate limits | None |
| `SEMANTIC_SCHOLAR_RATE_LIMIT` | Rate limit (requests/second) | 1 |
| `SEMANTIC_SCHOLAR_CIRCUIT_BREAKER_THRESHOLD` | Failures before opening circuit | 10 |
| `DIMENSIONS_API_KEY` | API key for Dimensions API | None |
| `OPENALEX_API_KEY` | API key for OpenAlex (meters search endpoints) | None |
| `ADS_API_KEY` | Required for NASA ADS search and DOI lookup | None |
| `IEEE_XPLORE_API_KEY` | Required for IEEE Xplore | None |
| `ACM_API_KEY` | Required for ACM | None |
| `SPRINGER_API_KEY` | Required for Springer | None |
| `CORE_API_KEY` | API key for CORE | None |
| `EXA_API_KEY` | Enables the MCP `web_search` tool | None |
| `ZOTERO_API_KEY` | Required for the Zotero sync tools | None |
| `ZOTERO_USER_ID` / `ZOTERO_GROUP_ID` | Zotero library id (user wins if both set) | None |
| `RESEARCH_MASTER_ZOTERO_API_URL` | Zotero API base URL override | `https://api.zotero.org` |
| `RESEARCH_MASTER_SCREENINGS_PATH` | Directory for the screening store `screenings.jsonl` | User config dir |
| `RESEARCH_MASTER_PROXY` | Proxy for HTTP and HTTPS when `HTTP_PROXY`/`HTTPS_PROXY` are unset (supports socks5/socks5h) | None |
| `GOOGLE_SCHOLAR_ENABLED` | Enable Google Scholar feature | false |
| `RESEARCH_MASTER_RATE_LIMITS_DEFAULT_REQUESTS_PER_SECOND` | Global rate limit | 5 |

## Source Auto-Detection

Paper ID formats are auto-detected in `unified_tools.rs`:
- `arXiv:1234.5678` or numeric → arXiv
- `PMC12345678` → PMC
- `NCTxxxxxxxx` → clinical_trials
- `hal-xxxxxxx` → HAL
- `xxxx/xxxx` (single slash) → IACR
- `10.xxxx/xxxxxx` → DOI lookup (Semantic Scholar preferred)

## Research Sources (36 total; 32 in the default build)

| Source | Capabilities | Description |
|--------|-------------|-------------|
| arXiv | Search, Download | Preprint papers in physics, math, CS, and more |
| PubMed | Search, Citations | Biomedical literature from NIH/NLM |
| ClinicalTrials.gov | Search, DOI Lookup* | ClinicalTrials.gov study registry (*DOI query returns the citing study; excluded from `lookup_by_doi` fan-out) |
| bioRxiv | Search, Download | Biology preprints |
| medRxiv | Search, Download | Health sciences preprints |
| Semantic Scholar | Search, Citations, Download | AI-powered academic search |
| OpenAlex | Search, Citations | Open scholarly knowledge graph |
| ORCID | Search, Author Search | ORCID researcher profiles and works |
| CrossRef | Search, DOI Lookup | DOI registration metadata |
| IACR | Search, Download | Cryptology research papers |
| PMC | Search, Download | PubMed Central open access papers |
| HAL | Search, Download | French open archive |
| DBLP | Search | Computer science bibliography |
| SSRN | Search, Download | Social science research network |
| Dimensions | Search | Research discovery platform |
| IEEE Xplore | Search, Download | Engineering and technology papers |
| Europe PMC | Search, Read | European PMC mirror |
| CORE | Search | Open access research papers |
| CiteSeerX | Search, DOI Lookup | Computer science paper search |
| Zenodo | Search, Download | CERN-backed research repository |
| OpenAIRE | Search, Download, DOI Lookup | EU open access research aggregator |
| DataCite | Search, DOI Lookup | DOI metadata for datasets and research outputs |
| INSPIRE-HEP | Search, DOI Lookup | High-energy physics literature |
| NASA ADS | Search, DOI Lookup | Astronomy and astrophysics literature |
| Unpaywall | DOI Lookup | Find open access versions of papers |
| MDPI | Search, Download | Open access publisher |
| JSTOR | Search | Humanities and social sciences |
| SciSpace | Search, DOI Lookup | Scientific research platform (opt-in via `--features`; public endpoint retired) |
| ACM | Search, Download | Computing literature |
| Connected Papers | Citations, DOI Lookup | Find related papers (opt-in via `--features`; no public REST API) |
| DOAJ | Search | Directory of open access journals |
| WorldWideScience | Search, DOI Lookup | Global science portal (opt-in via `--features`; public endpoint retired) |
| OSF | Search, Download | Open Science Framework preprints |
| BASE | Search | Bielefeld Academic Search Engine |
| Springer | Search, Download | Springer/Nature publications |
| Google Scholar | Search | Web search for scholarly literature (opt-in; also needs `GOOGLE_SCHOLAR_ENABLED=true`) |

### Convenience Feature Groups

- `core` - arxiv, pubmed, semantic
- `preprints` - arxiv, biorxiv, medrxiv
- `full` - All sources except google_scholar (includes scispace, connected_papers, worldwidescience)
- default build - 32 sources: everything except google_scholar, scispace, connected_papers, worldwidescience
