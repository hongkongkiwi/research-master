//! Tool registry for MCP tools.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::sources::SourceRegistry;

pub use super::unified_tools::{
    AuthorProfileHandler, BatchGetPapersHandler, CitationGraphHandler, DeduplicatePapersHandler,
    DownloadPaperHandler, ExportPapersHandler, GetCitationsHandler, GetPaperHandler,
    GetReferencesHandler, GetRelatedPapersHandler, GithubSearchHandler, ListSourcesHandler,
    LookupByDoiHandler, ReadPaperHandler, RecommendPapersHandler, SearchByAuthorHandler,
    SearchPapersHandler, WebExtractHandler, WebSearchHandler, WritePaperHandler,
};

/// An MCP tool that can be called by the client
#[derive(Clone)]
pub struct Tool {
    /// Tool name (e.g., "search_papers")
    pub name: String,

    /// Human-readable description
    pub description: String,

    /// JSON Schema for input parameters
    pub input_schema: serde_json::Value,

    /// Handler function to execute the tool
    pub handler: Arc<dyn ToolHandler>,
}

impl std::fmt::Debug for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tool")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("input_schema", &self.input_schema)
            .finish()
    }
}

/// Handler for executing a tool
#[async_trait::async_trait]
pub trait ToolHandler: Send + Sync + std::fmt::Debug {
    /// Execute the tool with the given arguments
    async fn execute(&self, args: Value) -> Result<Value, String>;
}

/// Registry for all MCP tools
#[derive(Debug, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Tool>,
}

impl ToolRegistry {
    /// Create a new tool registry and register all unified tools from the source registry
    pub fn from_sources(sources: &SourceRegistry) -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
        };

        // Convert sources to a shared Arc<Vec>
        let sources_vec: Vec<Arc<dyn crate::sources::Source>> = sources.all().cloned().collect();
        let sources_arc = Arc::new(sources_vec);

        // Register unified tools
        registry.register_unified_tools(&sources_arc);

        registry
    }

    /// Register unified tools (9 tools total instead of per-source tools)
    fn register_unified_tools(&mut self, sources: &Arc<Vec<Arc<dyn crate::sources::Source>>>) {
        let sources_count = sources.len();

        // 1. search_papers - Search across all or specific sources
        self.register(Tool {
            name: "search_papers".to_string(),
            description: format!(
                "Search for papers across {} available research sources",
                sources_count
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query string"
                    },
                    "source": {
                        "type": "string",
                        "description": "Specific source to search (e.g., 'arxiv', 'semantic', 'pubmed'). If not specified, searches all sources."
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of results per source",
                        "default": 10
                    },
                    "year": {
                        "type": "string",
                        "description": "Year filter (e.g., '2020', '2018-2022', '2010-', '-2015')"
                    },
                    "category": {
                        "type": "string",
                        "description": "Category/subject filter"
                    },
                    "fulltext": {
                        "type": "boolean",
                        "description": "Restrict to works whose full text is indexed and search inside it (Europe PMC, OpenAlex). Default false."
                    },
                    "license": {
                        "type": "string",
                        "enum": ["any", "open", "ignore"],
                        "description": "Work-license filter. 'any' (default) and 'ignore' keep all results. 'open' keeps only works with recognized open licenses and excludes unknowns. This does not override any provider API terms."
                    }
                },
                "required": ["query"]
            }),
            handler: Arc::new(SearchPapersHandler {
                sources: sources.clone(),
            }),
        });

        // 2. search_by_author - Author search across sources
        self.register(Tool {
            name: "search_by_author".to_string(),
            description: format!(
                "Search for papers by author across {} research sources",
                sources_count
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "author": {
                        "type": "string",
                        "description": "Author name"
                    },
                    "source": {
                        "type": "string",
                        "description": "Specific source to search. If not specified, searches all sources with author search capability."
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results per source",
                        "default": 10
                    }
                },
                "required": ["author"]
            }),
            handler: Arc::new(SearchByAuthorHandler {
                sources: sources.clone(),
            }),
        });

        // 3. get_paper - Get paper metadata with auto-detection
        self.register(Tool {
            name: "get_paper".to_string(),
            description: "Get detailed metadata for a specific paper. Enriches with a Semantic Scholar TLDR and reports retraction status when a DOI is present. Source is auto-detected from paper ID format.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_id": {
                        "type": "string",
                        "description": "Paper identifier (e.g., '2301.12345', 'arXiv:2301.12345', 'PMC12345678')"
                    },
                    "source": {
                        "type": "string",
                        "description": "Override auto-detection and use specific source"
                    },
                    "enrich": {
                        "type": "boolean",
                        "description": "Fetch the Semantic Scholar TLDR for the DOI (default true). Disable for faster bulk lookups.",
                        "default": true
                    }
                },
                "required": ["paper_id"]
            }),
            handler: Arc::new(GetPaperHandler {
                sources: sources.clone(),
            }),
        });

        // 4. download_paper - Download with auto-detection
        self.register(Tool {
            name: "download_paper".to_string(),
            description: "Download a paper PDF to your local filesystem. Source is auto-detected from paper ID format.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_id": {
                        "type": "string",
                        "description": "Paper identifier"
                    },
                    "source": {
                        "type": "string",
                        "description": "Override auto-detection and use specific source"
                    },
                    "output_path": {
                        "type": "string",
                        "description": "Save path for the PDF",
                        "default": "./downloads"
                    },
                    "auto_filename": {
                        "type": "boolean",
                        "description": "Auto-generate filename from paper title",
                        "default": true
                    }
                },
                "required": ["paper_id"]
            }),
            handler: Arc::new(DownloadPaperHandler {
                sources: sources.clone(),
            }),
        });

        // 5. read_paper - PDF text extraction with auto-detection
        self.register(Tool {
            name: "read_paper".to_string(),
            description: "Extract text from a paper PDF, bounded to 12000 characters per call. Paginate with the offset from next_start, or pass return_full_text for the whole document. Source is auto-detected from paper ID format. Requires poppler.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_id": {
                        "type": "string",
                        "description": "Paper identifier"
                    },
                    "source": {
                        "type": "string",
                        "description": "Override auto-detection and use specific source"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Maximum characters returned per call (default 12000)",
                        "default": 12000
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Character offset to start from (use next_start from the previous call)",
                        "default": 0
                    },
                    "return_full_text": {
                        "type": "boolean",
                        "description": "Return the entire text unbounded. Only when you really need it: large PDFs overflow LLM contexts.",
                        "default": false
                    }
                },
                "required": ["paper_id"]
            }),
            handler: Arc::new(ReadPaperHandler {
                sources: sources.clone(),
            }),
        });

        // 6. get_citations - Get papers that cite a given paper
        self.register(Tool {
            name: "get_citations".to_string(),
            description:
                "Get papers that cite a specific paper. Prefers Semantic Scholar for best results."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_id": {
                        "type": "string",
                        "description": "Paper identifier"
                    },
                    "source": {
                        "type": "string",
                        "description": "Specific source (default: 'semantic')",
                        "default": "semantic"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results",
                        "default": 20
                    }
                },
                "required": ["paper_id"]
            }),
            handler: Arc::new(GetCitationsHandler {
                sources: sources.clone(),
            }),
        });

        // 7. get_references - Get papers referenced by a given paper
        self.register(Tool {
            name: "get_references".to_string(),
            description: "Get papers referenced by a specific paper. Prefers Semantic Scholar for best results.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_id": {
                        "type": "string",
                        "description": "Paper identifier"
                    },
                    "source": {
                        "type": "string",
                        "description": "Specific source (default: 'semantic')",
                        "default": "semantic"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results",
                        "default": 20
                    }
                },
                "required": ["paper_id"]
            }),
            handler: Arc::new(GetReferencesHandler {
                sources: sources.clone(),
            }),
        });

        // 8. lookup_by_doi - DOI lookup across all sources
        self.register(Tool {
            name: "lookup_by_doi".to_string(),
            description: "Look up a paper by its DOI across all sources that support DOI lookup.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "doi": {
                        "type": "string",
                        "description": "Digital Object Identifier (e.g., '10.48550/arXiv.2301.12345')"
                    },
                    "source": {
                        "type": "string",
                        "description": "Specific source to query. If not specified, queries all sources with DOI lookup capability."
                    }
                },
                "required": ["doi"]
            }),
            handler: Arc::new(LookupByDoiHandler {
                sources: sources.clone(),
            }),
        });

        // 9. deduplicate_papers - Remove duplicates
        self.register(Tool {
            name: "deduplicate_papers".to_string(),
            description: "Remove duplicate papers from a list using DOI matching and title similarity.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "papers": {
                        "type": "array",
                        "description": "Array of paper objects",
                        "items": {
                            "type": "object"
                        }
                    },
                    "strategy": {
                        "type": "string",
                        "description": "Deduplication strategy: 'first' (keep first), 'last' (keep last), or 'mark' (add is_duplicate flag)",
                        "enum": ["first", "last", "mark"],
                        "default": "first"
                    }
                },
                "required": ["papers"]
            }),
            handler: Arc::new(DeduplicatePapersHandler),
        });

        // 10. get_related_papers - Get related/similar papers
        self.register(Tool {
            name: "get_related_papers".to_string(),
            description: "Get papers related or similar to a specific paper. Source auto-detected from paper ID format.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_id": {
                        "type": "string",
                        "description": "Paper identifier"
                    },
                    "source": {
                        "type": "string",
                        "description": "Override auto-detection and use specific source"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results",
                        "default": 20
                    }
                },
                "required": ["paper_id"]
            }),
            handler: Arc::new(GetRelatedPapersHandler {
                sources: sources.clone(),
            }),
        });

        // 11. list_sources - List available sources with capabilities
        self.register(Tool {
            name: "list_sources".to_string(),
            description: format!(
                "List all {} available research sources with their capabilities (search, download, citations, etc.)",
                sources_count
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            handler: Arc::new(ListSourcesHandler {
                sources: sources.clone(),
            }),
        });

        // 12. author_profile - Get author profile with publication stats
        self.register(Tool {
            name: "author_profile".to_string(),
            description: "Get an author's publication profile including paper counts, most cited works, and publication history across sources.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "author": {
                        "type": "string",
                        "description": "Author name"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum papers to analyze per source",
                        "default": 20
                    }
                },
                "required": ["author"]
            }),
            handler: Arc::new(AuthorProfileHandler {
                sources: sources.clone(),
            }),
        });

        // 13. batch_get_papers - Batch lookup multiple papers
        self.register(Tool {
            name: "batch_get_papers".to_string(),
            description: "Look up multiple papers by their IDs/DOIs in one call. Returns metadata for each with auto-detected sources.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_ids": {
                        "type": "array",
                        "description": "Array of paper identifiers (arXiv IDs, DOIs, PMC IDs, etc.)",
                        "items": {
                            "type": "string"
                        }
                    },
                    "max_per_source": {
                        "type": "integer",
                        "description": "Maximum results per paper lookup",
                        "default": 3
                    }
                },
                "required": ["paper_ids"]
            }),
            handler: Arc::new(BatchGetPapersHandler {
                sources: sources.clone(),
            }),
        });

        // 14. citation_graph - Get combined citation graph
        self.register(Tool {
            name: "citation_graph".to_string(),
            description: "Get both forward citations and backward references for a paper in a single call. Prefers Semantic Scholar.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_id": {
                        "type": "string",
                        "description": "Paper identifier"
                    },
                    "source": {
                        "type": "string",
                        "description": "Specific source (default: 'semantic')"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results for both citations and references",
                        "default": 20
                    }
                },
                "required": ["paper_id"]
            }),
            handler: Arc::new(CitationGraphHandler {
                sources: sources.clone(),
            }),
        });

        // 15. export_papers - Export papers in citation format
        self.register(Tool {
            name: "export_papers".to_string(),
            description: "Export a list of papers in a citation format (bibtex, apa, mla, chicago, csv, json).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "papers": {
                        "type": "array",
                        "description": "Array of paper objects to export",
                        "items": {
                            "type": "object"
                        }
                    },
                    "format": {
                        "type": "string",
                        "description": "Export format: 'bibtex', 'apa', 'mla', 'chicago', 'csv', or 'json'",
                        "default": "bibtex"
                    }
                },
                "required": ["papers"]
            }),
            handler: Arc::new(ExportPapersHandler {
                sources: sources.clone(),
            }),
        });

        // 16. recommend_papers - AI paper recommendations
        self.register(Tool {
            name: "recommend_papers".to_string(),
            description: "Get AI-powered paper recommendations similar to a given paper. Tries Semantic Scholar, Connected Papers, then OpenAlex.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_id": {
                        "type": "string",
                        "description": "Paper identifier to find similar papers for"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum recommendations",
                        "default": 20
                    }
                },
                "required": ["paper_id"]
            }),
            handler: Arc::new(RecommendPapersHandler {
                sources: sources.clone(),
            }),
        });

        // 17. write_paper - Save papers to local library
        self.register(Tool {
            name: "write_paper".to_string(),
            description: "Save paper metadata to a local JSON library file for later reference. Supports adding notes.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "papers": {
                        "type": "array",
                        "description": "Array of paper objects to save",
                        "items": {"type": "object"}
                    },
                    "notes": {
                        "type": "string",
                        "description": "Optional notes to attach to saved papers",
                        "default": ""
                    }
                },
                "required": ["papers"]
            }),
            handler: Arc::new(WritePaperHandler),
        });

        // 18. web_search - General web search (Exa, Tavily, or Brave)
        self.register(Tool {
            name: "web_search".to_string(),
            description: "Search the general web for information. Used as a fallback when paper sources don't have enough results. Uses the first configured of EXA_API_KEY, TAVILY_API_KEY, or BRAVE_API_KEY; if that provider fails, falls back to the next.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results",
                        "default": 5
                    }
                },
                "required": ["query"]
            }),
            handler: Arc::new(WebSearchHandler),
        });

        // 19. web_extract - Read a web page or PDF URL as text
        self.register(Tool {
            name: "web_extract".to_string(),
            description: "Fetch a URL and return its readable text: HTML pages are reduced to title plus block text (scripts, styles and navigation dropped), PDFs are run through the same extraction as read_paper. Use for paper landing pages, supplementary material, blog posts, or any page referenced in search results.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "http(s) URL to fetch and extract"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Maximum characters of text to return (default 20000, cap 100000)",
                        "default": 20000
                    }
                },
                "required": ["url"]
            }),
            handler: Arc::new(WebExtractHandler),
        });

        // 20. github_search - Find repositories (paper code, datasets, tools)
        self.register(Tool {
            name: "github_search".to_string(),
            description: "Search GitHub repositories by query. Useful for finding the code implementation accompanying a paper, plus datasets and research tools. Optional GITHUB_API_KEY raises the rate limit from 10 to 30 searches per minute.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Repository search query (GitHub search syntax, e.g. 'transformer attention language:python')"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results (1-20)",
                        "default": 5
                    }
                },
                "required": ["query"]
            }),
            handler: Arc::new(GithubSearchHandler {
                api_base: None,
                api_key: None,
            }),
        });

        // Evidence: smart-citation tallies plus retraction lookup.
        self.register(super::handlers_evidence::get_citation_tallies_tool());

        // CSL citation formatting for any archived style.
        self.register(Tool {
            name: "format_citation_style".to_string(),
            description: "Format a paper as an in-text citation in any CSL style from the Zotero style archive (thousands of styles: nature, ieee, vancouver, chicago-author-date, ...).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper": {
                        "type": "object",
                        "description": "A paper object as returned by get_paper or search_papers"
                    },
                    "style": {
                        "type": "string",
                        "description": "CSL style name, e.g. 'nature', 'ieee', 'vancouver', 'chicago-author-date'. See https://www.zotero.org/styles"
                    }
                },
                "required": ["paper", "style"]
            }),
            handler: Arc::new(super::unified_tools::CslCitationHandler),
        });

        // arXiv topic watches: saved queries with new-paper alerts.
        self.register(Tool {
            name: "watch_topic".to_string(),
            description: "Save a topic watch: later check_alerts calls surface new arXiv papers matching the query since the last check.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Topic query (arXiv 'all:' field syntax)" }
                },
                "required": ["query"]
            }),
            handler: Arc::new(super::handlers_watch::WatchTopicHandler),
        });
        self.register(Tool {
            name: "list_watches".to_string(),
            description: "List saved topic watches with their last-checked timestamps.".to_string(),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            handler: Arc::new(super::handlers_watch::ListWatchesHandler),
        });
        self.register(Tool {
            name: "check_alerts".to_string(),
            description: "Check one watch or all watches for new arXiv papers since the last check, returning any new papers and persisting the check.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Check only this saved query (default: check all)" },
                    "max_results": { "type": "integer", "description": "Max papers fetched per watch (default 20)", "default": 20 }
                }
            }),
            handler: Arc::new(super::handlers_watch::CheckAlertsHandler::default()),
        });
        self.register(Tool {
            name: "unwatch_topic".to_string(),
            description: "Remove a saved topic watch.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The saved query to remove" }
                },
                "required": ["query"]
            }),
            handler: Arc::new(super::handlers_watch::UnwatchTopicHandler),
        });

        // GROBID: PDF structure extraction against a self-hosted service.
        self.register(Tool {
            name: "extract_pdf_references".to_string(),
            description: "Extract the references of a local PDF as structured papers using a self-hosted GROBID service. Set GROBID_URL (default http://localhost:8070), e.g. docker run --rm -p 8070:8070 lfoppiano/grobid:0.8.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pdf_path": { "type": "string", "description": "Path to a local PDF file" }
                },
                "required": ["pdf_path"]
            }),
            handler: Arc::new(super::handlers_grobid::ExtractReferencesHandler),
        });
        self.register(Tool {
            name: "extract_pdf_header".to_string(),
            description: "Extract header metadata (title, authors, year, DOI, abstract) from a local PDF using a self-hosted GROBID service. Set GROBID_URL (default http://localhost:8070).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pdf_path": { "type": "string", "description": "Path to a local PDF file" }
                },
                "required": ["pdf_path"]
            }),
            handler: Arc::new(super::handlers_grobid::ExtractPaperHeaderHandler),
        });

        // Section-aware reading over extracted PDF text, and arXiv LaTeX.
        self.register(Tool {
            name: "get_paper_outline".to_string(),
            description: "Extract a section outline (title, offsets, preview) from a paper's PDF text. Combine with read_paper_section to read only the sections that matter.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_id": { "type": "string", "description": "Paper identifier" },
                    "source": { "type": "string", "description": "Override auto-detection and use specific source" }
                },
                "required": ["paper_id"]
            }),
            handler: Arc::new(super::handlers_reading::GetPaperOutlineHandler {
                sources: sources.clone(),
            }),
        });
        self.register(Tool {
            name: "read_paper_section".to_string(),
            description: "Read one section of a paper's PDF text by index (from get_paper_outline) or exact title. Bounded to max_chars with pagination.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper_id": { "type": "string", "description": "Paper identifier" },
                    "source": { "type": "string", "description": "Override auto-detection and use specific source" },
                    "section": { "description": "0-based section index or exact section title", "type": ["integer", "string"] },
                    "max_chars": { "type": "integer", "description": "Maximum characters (default 12000)", "default": 12000 }
                },
                "required": ["paper_id", "section"]
            }),
            handler: Arc::new(super::handlers_reading::ReadPaperSectionHandler {
                sources: sources.clone(),
            }),
        });
        self.register(Tool {
            name: "get_paper_latex".to_string(),
            description: "Fetch the LaTeX source of an arXiv paper (reliable for equations, unlike PDF extraction). Returns the file list and the main .tex file.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "arxiv_id": { "type": "string", "description": "arXiv identifier (e.g. '2310.12345', 'arXiv:2310.12345v2')" }
                },
                "required": ["arxiv_id"]
            }),
            handler: Arc::new(super::handlers_reading::GetPaperLatexHandler),
        });

        // Local library: persistent saved papers with collections, tags,
        // notes and retrieval scoring.
        self.register_library_tools();
    }

    /// Register the saved-papers library tools.
    fn register_library_tools(&mut self) {
        self.register(Tool {
            name: "library_add".to_string(),
            description: "Save a paper to the local library (JSONL at the user config dir). Re-saving the same DOI or paper_id updates its collection and tags; notes are preserved.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "paper": {
                        "type": "object",
                        "description": "A paper object as returned by get_paper or search_papers"
                    },
                    "paper_id": {
                        "type": "string",
                        "description": "Alternative to a full paper object: the paper id"
                    },
                    "doi": {
                        "type": "string",
                        "description": "Alternative to a full paper object: the DOI"
                    },
                    "title": {
                        "type": "string",
                        "description": "Title when constructing a paper from paper_id/doi"
                    },
                    "collection": {
                        "type": "string",
                        "description": "Optional collection name"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional tags"
                    }
                }
            }),
            handler: Arc::new(super::handlers_library::LibraryAddHandler),
        });
        self.register(Tool {
            name: "library_list".to_string(),
            description: "List saved papers, newest first. Optionally filter by collection.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "collection": { "type": "string", "description": "Only papers in this collection" }
                }
            }),
            handler: Arc::new(super::handlers_library::LibraryListHandler),
        });
        self.register(Tool {
            name: "library_remove".to_string(),
            description: "Remove a saved paper by id or DOI.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Paper id or DOI" },
                    "collection": { "type": "string", "description": "Only remove from this collection" }
                },
                "required": ["id"]
            }),
            handler: Arc::new(super::handlers_library::LibraryRemoveHandler),
        });
        self.register(Tool {
            name: "library_search".to_string(),
            description:
                "Search saved papers by keywords (TF-IDF style scoring with fuzzy term matching)."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Keyword query" },
                    "collection": { "type": "string", "description": "Restrict to a collection" },
                    "limit": { "type": "integer", "description": "Maximum matches (default 10)" }
                },
                "required": ["query"]
            }),
            handler: Arc::new(super::handlers_library::LibrarySearchHandler),
        });
        self.register(Tool {
            name: "library_ask".to_string(),
            description: "Retrieval over saved papers: returns the most relevant saved papers for a question with their abstracts and the terms that matched. This is keyword retrieval, not generated synthesis.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "Question to match against saved papers" },
                    "collection": { "type": "string", "description": "Restrict to a collection" },
                    "top_k": { "type": "integer", "description": "How many papers to return (default 5)" }
                },
                "required": ["question"]
            }),
            handler: Arc::new(super::handlers_library::LibraryAskHandler),
        });
    }

    /// Register a tool
    pub fn register(&mut self, tool: Tool) {
        self.tools.insert(tool.name.clone(), tool);
    }

    /// Get all tools
    pub fn all(&self) -> Vec<&Tool> {
        self.tools.values().collect()
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name)
    }

    /// Execute a tool by name
    pub async fn execute(&self, name: &str, args: Value) -> Result<Value, String> {
        let tool = self
            .get(name)
            .ok_or_else(|| format!("Tool '{}' not found", name))?;

        tool.handler.execute(args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{MockSource, SourceRegistry};
    use serde_json::json;

    #[derive(Debug)]
    struct NoopHandler;

    #[async_trait::async_trait]
    impl ToolHandler for NoopHandler {
        async fn execute(&self, _args: Value) -> Result<Value, String> {
            Ok(json!({"ok": true}))
        }
    }

    fn source_registry_with_mock() -> SourceRegistry {
        let mut registry = SourceRegistry::empty_for_tests();
        registry.register(Arc::new(MockSource::new()));
        registry
    }

    fn tool_registry() -> ToolRegistry {
        ToolRegistry::from_sources(&source_registry_with_mock())
    }

    fn required_fields(tool: &Tool) -> Vec<&str> {
        tool.input_schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|values| values.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn test_tool_struct_creation() {
        let input_schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"]
        });

        let tool = Tool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            input_schema: input_schema.clone(),
            handler: Arc::new(NoopHandler),
        };

        assert_eq!(tool.name, "test_tool");
        assert_eq!(tool.description, "A test tool");
        assert_eq!(tool.input_schema, input_schema);
    }

    #[test]
    fn test_tool_registry_creation_from_sources() {
        let registry = tool_registry();

        assert!(!registry.all().is_empty());
        assert!(registry.get("search_papers").is_some());
        assert!(registry.get("list_sources").is_some());
    }

    #[test]
    fn test_tool_registry_all_returns_all_registered_tools() {
        let registry = tool_registry();
        let tools = registry.all();
        let names: std::collections::HashSet<&str> =
            tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(tools.len(), 36);
        for expected in [
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
            "web_extract",
            "github_search",
        ] {
            assert!(names.contains(expected), "missing tool {expected}");
        }
    }

    #[test]
    fn test_tool_registry_get_by_name() {
        let registry = tool_registry();

        let search = registry.get("search_papers").expect("search_papers tool");
        assert_eq!(search.name, "search_papers");
        assert!(registry.get("not_a_tool").is_none());
    }

    #[test]
    fn test_each_tool_schema_has_valid_required_parameters() {
        let registry = tool_registry();

        for tool in registry.all() {
            let schema = &tool.input_schema;
            assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
            let properties = schema
                .get("properties")
                .and_then(|v| v.as_object())
                .expect("tool schema should define properties");

            for required in required_fields(tool) {
                assert!(
                    properties.contains_key(required),
                    "required field {required} for {} must be defined in properties",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn test_search_papers_schema_requires_query() {
        let registry = tool_registry();
        let search = registry.get("search_papers").expect("search_papers tool");

        assert_eq!(required_fields(search), vec!["query"]);
        assert!(search.input_schema["properties"].get("query").is_some());
    }

    #[test]
    fn test_list_sources_schema_has_no_required_params() {
        let registry = tool_registry();
        let list_sources = registry.get("list_sources").expect("list_sources tool");

        assert!(required_fields(list_sources).is_empty());
        assert!(list_sources.input_schema.get("required").is_none());
    }

    #[test]
    fn test_download_paper_schema_requires_paper_id() {
        let registry = tool_registry();
        let download = registry.get("download_paper").expect("download_paper tool");

        assert_eq!(required_fields(download), vec!["paper_id"]);
        assert!(download.input_schema["properties"]
            .get("paper_id")
            .is_some());
    }
}
