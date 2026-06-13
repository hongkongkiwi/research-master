//! Tool registry for MCP tools.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::sources::SourceRegistry;

pub use super::unified_tools::{
    AuthorProfileHandler, BatchGetPapersHandler, CitationGraphHandler, DeduplicatePapersHandler,
    DownloadPaperHandler, ExportPapersHandler, GetCitationsHandler, GetPaperHandler,
    GetReferencesHandler, GetRelatedPapersHandler, ListSourcesHandler, LookupByDoiHandler,
    ReadPaperHandler, RecommendPapersHandler, SearchByAuthorHandler, SearchPapersHandler,
    WebSearchHandler, WritePaperHandler,
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
            description: "Get detailed metadata for a specific paper. Source is auto-detected from paper ID format.".to_string(),
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
            description: "Extract and return the full text content from a paper PDF. Source is auto-detected from paper ID format. Requires poppler to be installed.".to_string(),
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

        // 18. web_search - General web search
        self.register(Tool {
            name: "web_search".to_string(),
            description: "Search the general web for information. Used as a fallback when paper sources don't have enough results. Requires EXA_API_KEY environment variable.".to_string(),
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
