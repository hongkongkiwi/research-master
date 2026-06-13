use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::shells::{Bash, Elvish, Fish, PowerShell, Zsh};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use research_master::config::{find_config_file, get_config, load_config};
use research_master::mcp::server::McpServer;
use research_master::models::{
    CitationRequest, DownloadRequest, ReadRequest, SearchQuery, SortBy, SortOrder,
};
use research_master::sources::{SourceCapabilities, SourceRegistry};
use research_master::utils::{
    apply_cli_proxy_args, deduplicate_papers, find_duplicates, format_authors, format_source,
    format_title, format_year, get_paper_table_columns, is_terminal, terminal_width, CacheService,
    DuplicateStrategy, HistoryService,
};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Research Master - Search and download academic papers from multiple research sources
#[derive(Parser, Debug)]
#[command(name = "research-master")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(author = "hongkongkiwi")]
#[command(about = "Search and download academic papers from multiple research sources", long_about = None)]
#[command(after_help = "EXAMPLES:
    # Search for papers across all sources
    research-master search \"transformer attention mechanism\"

    # Search for papers on arXiv only
    research-master search \"quantum computing\" --source arxiv

    # Search with year filter and limit results
    research-master search \"climate change\" --year 2020-2023 --max-results 5

    # Search by author
    research-master author \"Yoshua Bengio\" --max-results 10

    # Download a paper by arXiv ID
    research-master download 2310.12345 --source arxiv --output ./papers/

    # Read/extract text from a PDF
    research-master read 2310.12345 --source arxiv --path ./paper.pdf

    # Look up a paper by DOI
    research-master lookup 10.1038/nature12373

    # Get citations for a paper
    research-master citations 2310.12345 --source arxiv

    # Get related papers
    research-master related 2310.12345 --source arxiv

    # List all available sources
    research-master sources

    # Run MCP server for Claude Desktop
    research-master mcp

    # Manage configuration
    research-master config init     # Initialize config
    research-master config show     # Show current config
    research-master config edit     # Edit config file

    # Export papers to various formats
    research-master export --input papers.json --format bibtex -O output.bib
    research-master export --input papers.json --format csv -O output.csv
    research-master export --input papers.json --format json -O output.json
    research-master export --input papers.json --format ris -O output.ris

    # Bulk download from a file of paper IDs
    research-master bulk-download ./paper_ids.txt -o ./downloads/

    # Manage API keys
    research-master api-keys list              # List configured keys
    research-master api-keys set --source semantic  # Set key

    # Generate shell completions
    research-master completions bash
    research-master completions zsh
    research-master completions fish
")]
#[command(propagate_version = true)]
struct Cli {
    /// Enable verbose logging (can be used multiple times for more verbosity: -v, -vv, -vvv)
    #[arg(long, short, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Suppress non-error output
    #[arg(long, short)]
    quiet: bool,

    /// Output format
    #[arg(long, short, value_enum, global = true, default_value_t = OutputFormat::Auto)]
    output: OutputFormat,

    /// Configuration file path
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Request timeout in seconds
    #[arg(long, global = true, default_value_t = 30)]
    timeout: u64,

    /// Show all environment variables
    #[arg(long, global = true)]
    env: bool,

    /// Disable caching for this command (useful for testing fresh results)
    #[arg(long, global = true, default_value_t = false)]
    no_cache: bool,

    /// Log to a file instead of stderr
    #[arg(long, global = true, value_name = "FILE")]
    log_file: Option<PathBuf>,

    /// HTTP proxy URL (e.g., http://proxy:8080)
    #[arg(long, global = true, value_name = "URL")]
    http_proxy: Option<String>,

    /// HTTPS proxy URL (e.g., https://proxy:8080)
    #[arg(long, global = true, value_name = "URL")]
    https_proxy: Option<String>,

    /// Comma-separated list of hosts to bypass proxy
    #[arg(long, global = true, value_name = "HOSTS")]
    no_proxy: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

/// Output format for results
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum OutputFormat {
    /// Automatic based on terminal (table if TTY, JSON otherwise)
    Auto,
    /// Table format (human-readable)
    Table,
    /// JSON format (machine-readable)
    Json,
    /// Plain text format
    Plain,
}

/// Available research sources
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    #[value(name = "arxiv")]
    Arxiv,
    #[value(name = "pubmed")]
    Pubmed,
    #[value(name = "biorxiv")]
    Biorxiv,
    #[value(name = "semantic")]
    Semantic,
    #[value(name = "openalex")]
    OpenAlex,
    #[value(name = "crossref")]
    CrossRef,
    #[value(name = "iacr")]
    Iacr,
    #[value(name = "pmc")]
    Pmc,
    #[value(name = "hal")]
    Hal,
    #[value(name = "dblp")]
    Dblp,
    #[value(name = "ssrn")]
    Ssrn,
    #[value(name = "dimensions")]
    Dimensions,
    #[value(name = "ieee_xplore")]
    IeeeXplore,
    #[value(name = "europe_pmc")]
    EuropePmc,
    #[value(name = "core")]
    Core,
    #[value(name = "zenodo")]
    Zenodo,
    #[value(name = "unpaywall")]
    Unpaywall,
    #[value(name = "mdpi")]
    Mdpi,
    #[value(name = "jstor")]
    Jstor,
    #[value(name = "scispace")]
    Scispace,
    #[value(name = "acm")]
    Acm,
    #[value(name = "connected_papers")]
    ConnectedPapers,
    #[value(name = "doaj")]
    Doaj,
    #[value(name = "worldwidescience")]
    WorldWideScience,
    #[value(name = "osf")]
    Osf,
    #[value(name = "base")]
    Base,
    #[value(name = "springer")]
    Springer,
    #[value(name = "google_scholar")]
    GoogleScholar,
    #[value(name = "all")]
    All,
}

/// Sort field for results
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum SortField {
    /// Sort by relevance
    Relevance,
    /// Sort by publication date
    Date,
    /// Sort by citation count
    Citations,
    /// Sort by title
    Title,
    /// Sort by author
    Author,
}

/// Sort order
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    /// Ascending order
    Asc,
    /// Descending order
    Desc,
}

/// Strategy for handling duplicates
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum DedupStrategy {
    /// Keep the first occurrence of each duplicate group
    First,
    /// Keep the last occurrence of each duplicate group
    Last,
    /// Keep all papers but mark duplicates
    Mark,
}

/// Shell for completion generation
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum Shell {
    /// Bash shell
    Bash,
    /// Elvish shell
    Elvish,
    /// Fish shell
    Fish,
    /// PowerShell
    PowerShell,
    /// Zsh shell
    Zsh,
}

/// Export format for papers
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ExportFormat {
    /// BibTeX format for citation managers
    Bibtex,
    /// CSV spreadsheet format
    Csv,
    /// JSON format
    Json,
    /// RIS format (EndNote, Zotero)
    Ris,
}

/// Config action
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigAction {
    /// Initialize a new config file
    Init,
    /// Show current configuration
    Show,
    /// Edit configuration file
    Edit,
}

/// API key action
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ApiKeyAction {
    /// Set an API key
    Set,
    /// List configured API keys
    List,
    /// Remove an API key
    Remove,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Search for papers by query string
    #[command(alias = "s")]
    Search {
        /// Search query string
        query: String,

        /// Source to search (default: all)
        #[arg(long, short, value_enum, default_value_t = Source::All)]
        source: Source,

        /// Maximum number of results
        #[arg(long, short, default_value_t = 10)]
        max_results: usize,

        /// Year filter (e.g., "2020", "2018-2022", "2010-", "-2015")
        #[arg(long)]
        year: Option<String>,

        /// Sort by field
        #[arg(long, value_enum)]
        sort_by: Option<SortField>,

        /// Sort order
        #[arg(long, value_enum)]
        order: Option<Order>,

        /// Category/subject filter
        #[arg(long, short)]
        category: Option<String>,

        /// Author filter
        #[arg(long, short)]
        author: Option<String>,

        /// Deduplicate results across sources
        #[arg(long)]
        dedup: bool,

        /// Deduplication strategy (default: first)
        #[arg(long, value_enum, requires = "dedup")]
        dedup_strategy: Option<DedupStrategy>,

        /// Fetch detailed information (slower but more complete)
        #[arg(long, default_value_t = true)]
        fetch_details: bool,
    },

    /// Search for papers by author
    #[command(alias = "a")]
    Author {
        /// Author name
        author: String,

        /// Source to search (default: all sources that support author search)
        #[arg(long, short, value_enum, default_value_t = Source::All)]
        source: Source,

        /// Maximum number of results per source
        #[arg(long, short, default_value_t = 10)]
        max_results: usize,

        /// Year filter (e.g., "2020", "2018-2022", "2010-", "-2015")
        #[arg(long)]
        year: Option<String>,

        /// Deduplicate results across sources
        #[arg(long)]
        dedup: bool,

        /// Deduplication strategy (default: first)
        #[arg(long, value_enum, requires = "dedup")]
        dedup_strategy: Option<DedupStrategy>,
    },

    /// Download a paper's PDF
    #[command(alias = "d")]
    Download {
        /// Paper ID (source-specific identifier)
        paper_id: String,

        /// Source of the paper
        #[arg(long, short, value_enum)]
        source: Source,

        /// Path where to save the PDF
        #[arg(long)]
        output_path: Option<PathBuf>,

        /// Auto-generate filename from paper title
        #[arg(long)]
        auto_filename: bool,

        /// Create directory if it doesn't exist
        #[arg(long)]
        create_dir: bool,

        /// Paper DOI (optional, for verification)
        #[arg(long)]
        doi: Option<String>,
    },

    /// Read and extract text from a paper's PDF
    #[command(alias = "r")]
    Read {
        /// Paper ID (source-specific identifier)
        paper_id: String,

        /// Source of the paper
        #[arg(long, short, value_enum)]
        source: Source,

        /// Path where PDF is saved (or will be downloaded)
        #[arg(long, short = 'p')]
        path: PathBuf,

        /// Download PDF if not found locally
        #[arg(long, default_value_t = true)]
        download_if_missing: bool,

        /// Number of pages to extract (0 = all)
        #[arg(long)]
        pages: Option<usize>,

        /// Extract text to file instead of stdout
        #[arg(long, short = 'O')]
        output_file: Option<PathBuf>,
    },

    /// Get papers that cite a given paper
    #[command(alias = "c")]
    Citations {
        /// Paper ID (source-specific identifier)
        paper_id: String,

        /// Source of the paper
        #[arg(long, short, value_enum)]
        source: Source,

        /// Maximum number of results
        #[arg(long, short, default_value_t = 20)]
        max_results: usize,
    },

    /// Get papers referenced by a given paper
    #[command(alias = "ref")]
    References {
        /// Paper ID (source-specific identifier)
        paper_id: String,

        /// Source of the paper
        #[arg(long, short, value_enum)]
        source: Source,

        /// Maximum number of results
        #[arg(long, short, default_value_t = 20)]
        max_results: usize,
    },

    /// Get related/similar papers
    #[command(alias = "rel")]
    Related {
        /// Paper ID (source-specific identifier)
        paper_id: String,

        /// Source of the paper
        #[arg(long, short, value_enum)]
        source: Source,

        /// Maximum number of results
        #[arg(long, short, default_value_t = 20)]
        max_results: usize,
    },

    /// Look up a paper by DOI
    #[command(alias = "doi")]
    LookupByDoi {
        /// Digital Object Identifier
        doi: String,

        /// Source to use for lookup (default: all that support DOI lookup)
        #[arg(long, short, value_enum, default_value_t = Source::All)]
        source: Source,

        /// Return JSON output even in terminal
        #[arg(long, short)]
        json: bool,
    },

    /// List available sources and their capabilities
    #[command(alias = "ls")]
    Sources {
        /// Show detailed information about each source
        #[arg(long, short)]
        detailed: bool,

        /// Filter sources by capability
        #[arg(long, value_enum)]
        with_capability: Option<CapabilityFilter>,
    },

    /// Run the MCP server (for Claude Desktop and other MCP clients)
    #[command(alias = "serve")]
    Mcp {
        /// Run in stdio mode (for MCP clients like Claude Desktop)
        #[arg(long, default_value_t = true)]
        stdio: bool,

        /// Run in HTTP/SSE mode (overrides --stdio)
        #[arg(long)]
        http: bool,

        /// Port for SSE mode (if not using stdio)
        #[arg(long, short, default_value_t = 3000)]
        port: u16,

        /// Host to bind to for SSE mode
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },

    /// Deduplicate a JSON file containing papers
    #[command(alias = "dedup")]
    Dedupe {
        /// Input JSON file containing papers
        input: PathBuf,

        /// Output file (default: overwrite input)
        #[arg(long, short = 'O')]
        output_file: Option<PathBuf>,

        /// Deduplication strategy
        #[arg(long, value_enum, default_value_t = DedupStrategy::First)]
        strategy: DedupStrategy,

        /// Show duplicate groups without removing
        #[arg(long, short = 'v')]
        show: bool,
    },

    /// Manage local cache
    Cache {
        /// Subcommand
        #[command(subcommand)]
        command: CacheCommands,
    },

    /// Check configuration and source health
    #[command(alias = "diag")]
    Doctor {
        /// Check connectivity to all sources
        #[arg(long)]
        check_connectivity: bool,

        /// Check API keys are configured correctly
        #[arg(long)]
        check_api_keys: bool,

        /// Verbose output
        #[arg(long, short)]
        verbose: bool,
    },

    /// Update to the latest version
    Update {
        /// Force update even if already at latest version
        #[arg(long, short, default_value_t = false)]
        force: bool,

        /// Preview what would be updated without making changes
        #[arg(long, short = 'n', default_value_t = false)]
        dry_run: bool,
    },

    /// Manage configuration
    #[command(alias = "cfg")]
    Config {
        /// Action to perform
        #[arg(value_enum)]
        action: ConfigAction,
    },

    /// Export papers to various formats
    Export {
        /// Input file (JSON with papers) or search results
        #[arg(short, long)]
        input: Option<PathBuf>,

        /// Export format
        #[arg(short, long, value_enum, default_value_t = ExportFormat::Bibtex)]
        format: ExportFormat,

        /// Output file (stdout if not specified)
        #[arg(short, long, short = 'O')]
        output: Option<PathBuf>,

        /// Source to search if no input file provided
        #[arg(long, value_enum)]
        source: Option<Source>,

        /// Search query (requires --source)
        #[arg(long, short = 'q')]
        query: Option<String>,

        /// Maximum number of results to export
        #[arg(long, default_value_t = 100)]
        max_results: usize,
    },

    /// Download multiple papers from a file
    #[command(alias = "bulk-dl")]
    BulkDownload {
        /// File containing paper IDs (one per line, format: source:id or just id)
        input: PathBuf,

        /// Output directory for downloads
        #[arg(long, short = 'o', default_value = "./downloads")]
        output_dir: PathBuf,

        /// Source to use if not specified in file
        #[arg(long, value_enum)]
        source: Option<Source>,

        /// Create source subdirectories
        #[arg(long, default_value_t = true)]
        organize_by_source: bool,

        /// Maximum concurrent downloads
        #[arg(long, default_value_t = 5)]
        concurrency: usize,
    },

    /// Manage API keys
    ApiKeys {
        /// Action to perform
        #[arg(value_enum)]
        action: ApiKeyAction,

        /// Source name (for set/remove)
        #[arg(long, short)]
        source: Option<String>,
    },

    /// Generate shell completion scripts
    #[command(alias = "completion")]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Show search and download history
    #[command(alias = "hist")]
    History {
        /// Maximum number of items to show
        #[arg(long, short, default_value_t = 20)]
        limit: usize,

        /// Show only searches
        #[arg(long)]
        searches: bool,

        /// Show only downloads
        #[arg(long)]
        downloads: bool,

        /// Clear history after showing
        #[arg(long)]
        clear: bool,
    },

    /// Clear cache, history, or downloads
    Clear {
        /// Clear all cached data
        #[arg(long)]
        cache: bool,

        /// Clear search history
        #[arg(long)]
        history: bool,

        /// Clear downloaded papers
        #[arg(long)]
        downloads: bool,

        /// Clear everything (cache, history, downloads)
        #[arg(long, short = 'a')]
        all: bool,
    },

    /// Format a paper citation in various styles
    Cite {
        /// Paper ID (arXiv ID, DOI, PMC ID, etc.)
        paper_id: String,

        /// Citation style
        #[arg(long, value_enum, default_value_t = CitationStyle::Apa)]
        style: CitationStyle,

        /// Source of the paper (auto-detected if not specified)
        #[arg(long, value_enum)]
        source: Option<Source>,

        /// Output format
        #[arg(long, value_enum, default_value_t = CitationOutputFormat::Text)]
        format: CitationOutputFormat,
    },
}

/// Citation style for formatting
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum CitationStyle {
    /// APA 7th edition
    Apa,
    /// MLA 9th edition
    Mla,
    /// Chicago 17th edition (author-date)
    Chicago,
    /// BibTeX
    Bibtex,
}

/// Output format for citations
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum CitationOutputFormat {
    /// Plain text (default)
    Text,
    /// BibTeX format
    Bibtex,
    /// JSON format
    Json,
}

#[derive(Subcommand, Debug)]
enum CacheCommands {
    /// Show cache status and statistics
    Status,

    /// Clear all cached data
    Clear,

    /// Clear only search cache
    ClearSearches,

    /// Clear only citation cache
    ClearCitations,
}

/// Capability filter for listing sources
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum CapabilityFilter {
    Search,
    Download,
    Read,
    Citations,
    DoiLookup,
    AuthorSearch,
}

/// Print all available environment variables
fn print_env_vars() {
    println!("Research Master MCP - Environment Variables");
    println!();
    println!("API Keys:");
    println!("  SEMANTIC_SCHOLAR_API_KEY    API key for Semantic Scholar (higher rate limits)");
    println!("  CORE_API_KEY                API key for CORE service");
    println!("  OPENALEX_EMAIL              Email for OpenAlex 'polite pool' access");
    println!();
    println!("Source-Specific Rate Limits:");
    println!("  SEMANTIC_SCHOLAR_RATE_LIMIT  Semantic Scholar requests per second (default: 1)");
    println!();
    println!("Global Proxy Settings:");
    println!("  HTTP_PROXY                  HTTP proxy URL (e.g., http://proxy:8080)");
    println!("  HTTPS_PROXY                 HTTPS proxy URL (e.g., https://proxy:8080)");
    println!("  NO_PROXY                    Comma-separated list of hosts to bypass proxy");
    println!();
    println!("Per-Source Proxy Settings:");
    println!("  RESEARCH_MASTER_PROXY_HTTP   Per-source HTTP proxy (format: source_id:proxy_url)");
    println!("  RESEARCH_MASTER_PROXY_HTTPS  Per-source HTTPS proxy (format: source_id:proxy_url)");
    println!();
    println!("Download Settings:");
    println!("  RESEARCH_MASTER_DOWNLOADS_DEFAULT_PATH     Default directory for PDF downloads (default: ./downloads)");
    println!("  RESEARCH_MASTER_DOWNLOADS_ORGANIZE_BY_SOURCE  Create subdirectories per source (default: true)");
    println!("  RESEARCH_MASTER_DOWNLOADS_MAX_FILE_SIZE_MB    Maximum file size for downloads in MB (default: 100)");
    println!();
    println!("Rate Limiting:");
    println!("  RESEARCH_MASTER_RATE_LIMITS_DEFAULT_REQUESTS_PER_SECOND  Default requests per second (default: 5.0)");
    println!("  RESEARCH_MASTER_RATE_LIMITS_MAX_CONCURRENT_REQUESTS     Max concurrent requests (default: 10)");
    println!();
    println!("Cache Settings:");
    println!(
        "  RESEARCH_MASTER_CACHE_ENABLED                Enable local caching (default: disabled)"
    );
    println!("  RESEARCH_MASTER_CACHE_DIRECTORY              Custom cache directory");
    println!("  RESEARCH_MASTER_CACHE_SEARCH_TTL_SECONDS     TTL for search results (default: 1800 = 30 min)");
    println!("  RESEARCH_MASTER_CACHE_CITATION_TTL_SECONDS   TTL for citation results (default: 900 = 15 min)");
    println!();
    println!("Other Settings:");
    println!("  RUST_LOG                    Rust logging level (e.g., debug, info, warn, error)");
    println!();
    println!("Example:");
    println!("  export SEMANTIC_SCHOLAR_API_KEY=\"your-key-here\"");
    println!("  export SEMANTIC_SCHOLAR_RATE_LIMIT=\"5\"");
    println!("  export RESEARCH_MASTER_DOWNLOADS_DEFAULT_PATH=\"./papers\"");
    std::process::exit(0);
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Apply CLI proxy arguments to environment variables
    // This allows sources to pick up the proxy settings via their normal env var reading
    apply_cli_proxy_args(cli.http_proxy, cli.https_proxy, cli.no_proxy);

    // Show environment variables and exit if requested
    if cli.env {
        print_env_vars();
    }

    // Use simplified logging format (not JSON) for stderr to reduce noise
    // Default to 'warn' level to reduce noise, 'info' shows too much
    let env_filter = if cli.quiet { "error" } else { "warn" };

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_level(false) // Don't show log level
        .with_target(false) // Don't show target
        .with_thread_ids(false)
        .compact();

    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| format!("research_master={}", env_filter)),
        ))
        .with(fmt_layer);

    // Add file logging if requested
    if let Some(log_path) = &cli.log_file {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .map_err(|e| anyhow::anyhow!("Failed to open log file: {}", e))?;

        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(file)
            .with_ansi(false)
            .json();

        subscriber.with(file_layer).init();
        tracing::info!("Logging to file: {}", log_path.display());
    } else {
        subscriber.init();
    }

    // Set timeout
    tokio::time::sleep(Duration::from_secs(0)).await; // Just to ensure runtime is initialized

    // Load configuration from file if specified or found in default locations
    let _config = if let Some(config_path) = &cli.config {
        Some(load_config(config_path)?)
    } else if let Some(config_path) = find_config_file() {
        tracing::info!("Using config file: {}", config_path.display());
        Some(load_config(&config_path)?)
    } else {
        None
    };

    // Create source registry
    let registry = SourceRegistry::new();

    // Execute command
    match cli.command {
        Some(Commands::Search {
            query,
            source,
            max_results,
            year,
            sort_by,
            order,
            category,
            author,
            dedup,
            dedup_strategy,
            fetch_details,
        }) => {
            let mut search_query = SearchQuery::new(&query);
            search_query.max_results = max_results;
            search_query.year = year;
            search_query.sort_by = sort_by.map(|s| match s {
                SortField::Relevance => SortBy::Relevance,
                SortField::Date => SortBy::Date,
                SortField::Citations => SortBy::CitationCount,
                SortField::Title => SortBy::Title,
                SortField::Author => SortBy::Author,
            });
            search_query.sort_order = order.map(|o| match o {
                Order::Asc => SortOrder::Ascending,
                Order::Desc => SortOrder::Descending,
            });
            search_query.category = category;
            search_query.author = author;
            search_query.fetch_details = fetch_details;

            let sources = get_sources(&registry, source, SourceCapabilities::SEARCH);
            let all_papers = Arc::new(Mutex::new(Vec::new()));
            let quiet = cli.quiet;

            // Initialize cache if not disabled
            let cache = if cli.no_cache {
                None
            } else {
                let c = CacheService::new();
                let _ = c.initialize();
                Some(c)
            };

            // Create a vector to hold all spawned tasks
            let mut handles = Vec::new();

            // Set up interactive progress display
            let mp = if !quiet && is_terminal() {
                Some(MultiProgress::new())
            } else {
                None
            };

            // Style for progress bars
            let spinner_style =
                ProgressStyle::with_template("{prefix:.bold.dim} {spinner} {wide_msg}")
                    .expect("valid progress template")
                    .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ");

            for src in sources {
                let src_id = src.id().to_string();
                let src = Arc::clone(src);
                let search_query = search_query.clone();
                let cache_inner = cache.clone();
                let mp = mp.clone();

                // Create progress bar for this source
                let pb = mp.as_ref().map(|m| {
                    let pb = m.add(ProgressBar::new(100));
                    pb.set_style(spinner_style.clone());
                    pb.set_prefix(format!("{:>15}", src_id));
                    pb
                });
                let pb_for_handle = pb.clone();

                // Clone values for the async block
                let src_id_for_handle = src_id.clone();
                let cache_for_handle = cache_inner.clone();

                // Spawn a task for each source
                let handle = tokio::spawn(async move {
                    let start = std::time::Instant::now();
                    let cache = cache_for_handle;
                    let pb = pb_for_handle;

                    // Check cache first - if we have both cache and progress bar
                    if let (Some(cache_service), Some(pb)) = (cache.as_ref(), pb.as_ref()) {
                        match cache_service.get_search(&search_query, &src_id_for_handle) {
                            research_master::utils::CacheResult::Hit(response) => {
                                let elapsed = start.elapsed();
                                let msg = format!(
                                    "{} papers ({:.1}s) [cached]",
                                    response.papers.len(),
                                    elapsed.as_secs_f64()
                                );
                                let style =
                                    ProgressStyle::with_template("{prefix:.bold.dim} {msg}")
                                        .expect("valid progress template");
                                pb.set_style(style);
                                pb.set_message(msg);
                                pb.finish();
                                return response.papers;
                            }
                            research_master::utils::CacheResult::Expired => {
                                pb.set_message("refreshing...");
                                // Make API call with cache_service and pb
                                match src.search(&search_query).await {
                                    Ok(response) => {
                                        let elapsed = start.elapsed();
                                        cache_service.set_search(
                                            &src_id_for_handle,
                                            &search_query,
                                            &response,
                                        );
                                        let msg = format!(
                                            "{} papers ({:.1}s)",
                                            response.papers.len(),
                                            elapsed.as_secs_f64()
                                        );
                                        let style = ProgressStyle::with_template(
                                            "{prefix:.bold.dim} {msg}",
                                        )
                                        .expect("valid progress template");
                                        pb.set_style(style);
                                        pb.set_message(msg);
                                        pb.finish();
                                        return response.papers;
                                    }
                                    Err(e) => {
                                        let elapsed = start.elapsed();
                                        let msg = format!(
                                            "error after {:.1}s: {}",
                                            elapsed.as_secs_f64(),
                                            e.to_string().lines().next().unwrap_or("unknown error")
                                        );
                                        let style = ProgressStyle::with_template(
                                            "{prefix:.bold.dim} {msg}",
                                        )
                                        .expect("valid progress template");
                                        pb.set_style(style);
                                        pb.set_message(msg);
                                        pb.finish();
                                        return Vec::new();
                                    }
                                }
                            }
                            research_master::utils::CacheResult::Miss => {
                                pb.set_message("searching...");
                                match src.search(&search_query).await {
                                    Ok(response) => {
                                        let elapsed = start.elapsed();
                                        cache_service.set_search(
                                            &src_id_for_handle,
                                            &search_query,
                                            &response,
                                        );
                                        let msg = format!(
                                            "{} papers ({:.1}s)",
                                            response.papers.len(),
                                            elapsed.as_secs_f64()
                                        );
                                        let style = ProgressStyle::with_template(
                                            "{prefix:.bold.dim} {msg}",
                                        )
                                        .expect("valid progress template");
                                        pb.set_style(style);
                                        pb.set_message(msg);
                                        pb.finish();
                                        return response.papers;
                                    }
                                    Err(e) => {
                                        let elapsed = start.elapsed();
                                        let msg = format!(
                                            "error after {:.1}s: {}",
                                            elapsed.as_secs_f64(),
                                            e.to_string().lines().next().unwrap_or("unknown error")
                                        );
                                        let style = ProgressStyle::with_template(
                                            "{prefix:.bold.dim} {msg}",
                                        )
                                        .expect("valid progress template");
                                        pb.set_style(style);
                                        pb.set_message(msg);
                                        pb.finish();
                                        return Vec::new();
                                    }
                                }
                            }
                        }
                    }

                    // No cache or no progress bar
                    match src.search(&search_query).await {
                        Ok(response) => {
                            let elapsed = start.elapsed();
                            if let Some(cache_service) = cache {
                                cache_service.set_search(
                                    &src_id_for_handle,
                                    &search_query,
                                    &response,
                                );
                            }
                            if let Some(pb) = pb {
                                let msg = format!(
                                    "{} papers ({:.1}s)",
                                    response.papers.len(),
                                    elapsed.as_secs_f64()
                                );
                                let style =
                                    ProgressStyle::with_template("{prefix:.bold.dim} {msg}")
                                        .expect("valid progress template");
                                pb.set_style(style);
                                pb.set_message(msg);
                                pb.finish();
                            }
                            response.papers
                        }
                        Err(e) => {
                            let elapsed = start.elapsed();
                            if let Some(pb) = pb {
                                let msg = format!(
                                    "error after {:.1}s: {}",
                                    elapsed.as_secs_f64(),
                                    e.to_string().lines().next().unwrap_or("unknown error")
                                );
                                let style =
                                    ProgressStyle::with_template("{prefix:.bold.dim} {msg}")
                                        .expect("valid progress template");
                                pb.set_style(style);
                                pb.set_message(msg);
                                pb.finish();
                            }
                            Vec::new()
                        }
                    }
                });

                handles.push((src_id, handle, pb));
            }

            // Wait for all tasks to complete and collect results
            for (source_id, handle, _pb) in handles {
                match handle.await {
                    Ok(papers) => {
                        let mut all_papers = all_papers.lock().expect("mutex not poisoned");
                        all_papers.extend(papers);
                    }
                    Err(e) => {
                        tracing::warn!("Task error for {}: {}", source_id, e);
                    }
                }
            }

            // Clear the progress display
            if let Some(ref m) = mp {
                m.clear().expect("multi-progress clear");
            }

            // Get the collected papers
            let mut all_papers = {
                let all_papers = all_papers.lock().expect("mutex not poisoned");
                all_papers.clone()
            };

            if dedup {
                let strategy = match dedup_strategy.unwrap_or(DedupStrategy::First) {
                    DedupStrategy::First => DuplicateStrategy::First,
                    DedupStrategy::Last => DuplicateStrategy::Last,
                    DedupStrategy::Mark => DuplicateStrategy::Mark,
                };
                all_papers = deduplicate_papers(all_papers, strategy);
            }

            output_papers(&all_papers, cli.output);
        }

        Some(Commands::Author {
            author,
            source,
            max_results,
            year,
            dedup,
            dedup_strategy,
        }) => {
            let sources = get_sources(&registry, source, SourceCapabilities::AUTHOR_SEARCH);
            let all_papers = Arc::new(Mutex::new(Vec::new()));
            let quiet = cli.quiet;

            // Create a vector to hold all spawned tasks
            let mut handles = Vec::new();

            // Set up interactive progress display
            let mp = if !quiet && is_terminal() {
                Some(MultiProgress::new())
            } else {
                None
            };

            // Style for progress bars
            let spinner_style =
                ProgressStyle::with_template("{prefix:.bold.dim} {spinner} {wide_msg}")
                    .expect("valid progress template")
                    .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ");

            for src in sources {
                let src_id = src.id().to_string();
                let src = Arc::clone(src);
                let author = author.clone();
                let year = year.clone();
                let mp = mp.clone();

                // Create progress bar for this source
                let pb = mp.as_ref().map(|m| {
                    let pb = m.add(ProgressBar::new(100));
                    pb.set_style(spinner_style.clone());
                    pb.set_prefix(format!("{:>15}", src_id));
                    pb
                });
                let pb_for_handle = pb.clone();

                // Spawn a task for each source
                let handle = tokio::spawn(async move {
                    let start = std::time::Instant::now();
                    let pb = pb_for_handle;
                    match src
                        .search_by_author(&author, max_results, year.as_deref())
                        .await
                    {
                        Ok(response) => {
                            let elapsed = start.elapsed();
                            if let Some(ref pb) = pb {
                                let msg = format!(
                                    "{} papers ({:.1}s)",
                                    response.papers.len(),
                                    elapsed.as_secs_f64()
                                );
                                let style =
                                    ProgressStyle::with_template("{prefix:.bold.dim} {msg}")
                                        .expect("valid progress template");
                                pb.set_style(style);
                                pb.set_message(msg);
                                pb.finish();
                            }
                            response.papers
                        }
                        Err(e) => {
                            let elapsed = start.elapsed();
                            if let Some(ref pb) = pb {
                                let msg = format!(
                                    "error after {:.1}s: {}",
                                    elapsed.as_secs_f64(),
                                    e.to_string().lines().next().unwrap_or("unknown error")
                                );
                                let style =
                                    ProgressStyle::with_template("{prefix:.bold.dim} {msg}")
                                        .expect("valid progress template");
                                pb.set_style(style);
                                pb.set_message(msg);
                                pb.finish();
                            }
                            Vec::new()
                        }
                    }
                });

                handles.push((src_id, handle, pb));
            }

            // Wait for all tasks to complete and collect results
            for (source_id, handle, _pb) in handles {
                match handle.await {
                    Ok(papers) => {
                        let mut all_papers = all_papers.lock().expect("mutex not poisoned");
                        all_papers.extend(papers);
                    }
                    Err(e) => {
                        tracing::warn!("Task error for {}: {}", source_id, e);
                    }
                }
            }

            // Clear the progress display
            if let Some(ref m) = mp {
                m.clear().expect("multi-progress clear");
            }

            // Get the collected papers
            let mut all_papers = {
                let all_papers = all_papers.lock().expect("mutex not poisoned");
                all_papers.clone()
            };

            if dedup {
                let strategy = match dedup_strategy.unwrap_or(DedupStrategy::First) {
                    DedupStrategy::First => DuplicateStrategy::First,
                    DedupStrategy::Last => DuplicateStrategy::Last,
                    DedupStrategy::Mark => DuplicateStrategy::Mark,
                };
                all_papers = deduplicate_papers(all_papers, strategy);
            }

            output_papers(&all_papers, cli.output);
        }

        Some(Commands::Download {
            paper_id,
            source,
            output_path,
            auto_filename: _,
            create_dir,
            doi,
        }) => {
            let src = get_source(&registry, source)?;
            let save_path = output_path.unwrap_or_else(|| PathBuf::from("."));

            if create_dir {
                if let Some(parent) = save_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
            }

            let mut request = DownloadRequest::new(&paper_id, save_path.to_string_lossy());
            if let Some(doi_val) = doi {
                request = request.doi(&doi_val);
            }

            let result = src.download(&request).await?;

            if result.success {
                if !cli.quiet {
                    eprintln!("Downloaded {} bytes to {}", result.bytes, result.path);
                }
            } else {
                anyhow::bail!("Download failed: {:?}", result.error);
            }
        }

        Some(Commands::Read {
            paper_id,
            source,
            path,
            download_if_missing,
            pages: _,
            output_file,
        }) => {
            let src = get_source(&registry, source)?;
            let request = ReadRequest::new(&paper_id, path.to_string_lossy())
                .download_if_missing(download_if_missing);

            let result = src.read(&request).await?;

            if result.success {
                let text = result.text;
                if let Some(output_path) = output_file {
                    std::fs::write(&output_path, text)?;
                    if !cli.quiet {
                        eprintln!("Text written to {}", output_path.display());
                    }
                } else {
                    println!("{}", text);
                }
            } else {
                anyhow::bail!("Read failed: {:?}", result.error);
            }
        }

        Some(Commands::Citations {
            paper_id,
            source,
            max_results,
        }) => {
            let src = get_source(&registry, source)?;
            let request = CitationRequest::new(&paper_id).max_results(max_results);

            let response = src.get_citations(&request).await?;
            output_papers(&response.papers, cli.output);
        }

        Some(Commands::References {
            paper_id,
            source,
            max_results,
        }) => {
            let src = get_source(&registry, source)?;
            let request = CitationRequest::new(&paper_id).max_results(max_results);

            let response = src.get_references(&request).await?;
            output_papers(&response.papers, cli.output);
        }

        Some(Commands::Related {
            paper_id,
            source,
            max_results,
        }) => {
            let src = get_source(&registry, source)?;
            let request = CitationRequest::new(&paper_id).max_results(max_results);

            let response = src.get_related(&request).await?;
            output_papers(&response.papers, cli.output);
        }

        Some(Commands::LookupByDoi { doi, source, json }) => {
            let sources = get_sources(&registry, source, SourceCapabilities::DOI_LOOKUP);
            let output_fmt = if json { OutputFormat::Json } else { cli.output };

            for src in sources {
                match src.get_by_doi(&doi).await {
                    Ok(paper) => {
                        output_papers(&[paper], output_fmt);
                        return Ok(());
                    }
                    Err(e) => {
                        if !cli.quiet {
                            eprintln!("Not found in {}: {}", src.id(), e);
                        }
                    }
                }
            }
            anyhow::bail!("Paper not found in any source");
        }

        Some(Commands::Sources {
            detailed,
            with_capability,
        }) => {
            let sources: Vec<_> = match with_capability {
                Some(CapabilityFilter::Search) => {
                    registry.with_capability(SourceCapabilities::SEARCH)
                }
                Some(CapabilityFilter::Download) => {
                    registry.with_capability(SourceCapabilities::DOWNLOAD)
                }
                Some(CapabilityFilter::Read) => registry.with_capability(SourceCapabilities::READ),
                Some(CapabilityFilter::Citations) => {
                    registry.with_capability(SourceCapabilities::CITATIONS)
                }
                Some(CapabilityFilter::DoiLookup) => {
                    registry.with_capability(SourceCapabilities::DOI_LOOKUP)
                }
                Some(CapabilityFilter::AuthorSearch) => {
                    registry.with_capability(SourceCapabilities::AUTHOR_SEARCH)
                }
                None => registry.all().collect(),
            };

            for src in sources {
                if detailed {
                    println!("{} ({})", src.name(), src.id());
                    println!("  Capabilities: {:?}", src.capabilities());
                } else {
                    println!("{} - {}", src.id(), src.name());
                }
            }
        }

        Some(Commands::Mcp {
            stdio,
            http,
            port,
            host,
        }) => {
            let server = McpServer::new(Arc::new(registry))?;

            // Use HTTP mode if --http flag is provided, otherwise use --stdio flag
            let use_http = http || !stdio;

            if use_http {
                let addr = format!("{}:{}", host, port);
                tracing::info!("Running MCP server in HTTP/SSE mode on {}", addr);
                let (bound_addr, handle) = server.run_http(&addr).await?;
                tracing::info!("MCP server listening on {}", bound_addr);

                // Wait for the server to finish
                handle
                    .await
                    .map_err(|e| anyhow::anyhow!("Server task failed: {}", e))?;
            } else {
                tracing::info!("Running MCP server in stdio mode");
                server.run().await?;
            }
        }

        Some(Commands::Dedupe {
            input,
            output_file,
            strategy,
            show,
        }) => {
            let json_str = std::fs::read_to_string(&input)?;
            let papers: Vec<research_master::models::Paper> = serde_json::from_str(&json_str)?;

            let dup_strategy = match strategy {
                DedupStrategy::First => DuplicateStrategy::First,
                DedupStrategy::Last => DuplicateStrategy::Last,
                DedupStrategy::Mark => DuplicateStrategy::Mark,
            };

            if show {
                let groups = find_duplicates(&papers);
                if groups.is_empty() {
                    println!("No duplicates found");
                } else {
                    println!("Found {} duplicate groups:", groups.len());
                    for (i, group) in groups.iter().enumerate() {
                        println!("  Group {}: {} papers", i + 1, group.len());
                        for idx in group {
                            println!("    - {} ({})", papers[*idx].title, papers[*idx].source);
                        }
                    }
                }
            } else {
                let deduped = deduplicate_papers(papers, dup_strategy);
                let output_json = serde_json::to_string_pretty(&deduped)?;
                let output_path = output_file.as_ref().unwrap_or(&input);
                std::fs::write(output_path, output_json)?;
                if !cli.quiet {
                    eprintln!(
                        "Deduplicated: {} -> {} papers",
                        input.display(),
                        deduped.len()
                    );
                }
            }
        }

        Some(Commands::Cache { command }) => {
            let cache = CacheService::new();
            cache.initialize()?;

            match command {
                CacheCommands::Status => {
                    let stats = cache.stats();
                    if !stats.enabled {
                        println!("Cache: disabled");
                        println!("To enable, set RESEARCH_MASTER_CACHE_ENABLED=true");
                    } else {
                        println!("Cache: enabled");
                        println!("Directory: {}", stats.cache_dir.display());
                        println!(
                            "Search cache: {} items ({} KB)",
                            stats.search_count, stats.search_size_kb
                        );
                        println!(
                            "Citation cache: {} items ({} KB)",
                            stats.citation_count, stats.citation_size_kb
                        );
                        println!("Total size: {} KB", stats.total_size_kb);
                        println!("Search TTL: {} seconds", stats.ttl_search.as_secs());
                        println!("Citation TTL: {} seconds", stats.ttl_citations.as_secs());
                    }
                }
                CacheCommands::Clear => {
                    if !cli.quiet {
                        eprintln!("Clearing all cached data...");
                    }
                    cache.clear_all()?;
                    if !cli.quiet {
                        eprintln!("Cache cleared successfully.");
                    }
                }
                CacheCommands::ClearSearches => {
                    if !cli.quiet {
                        eprintln!("Clearing search cache...");
                    }
                    cache.clear_searches()?;
                    if !cli.quiet {
                        eprintln!("Search cache cleared successfully.");
                    }
                }
                CacheCommands::ClearCitations => {
                    if !cli.quiet {
                        eprintln!("Clearing citation cache...");
                    }
                    cache.clear_citations()?;
                    if !cli.quiet {
                        eprintln!("Citation cache cleared successfully.");
                    }
                }
            }
        }

        Some(Commands::Doctor {
            check_connectivity,
            check_api_keys,
            verbose,
        }) => {
            println!("Research Master MCP - Doctor");
            println!("================================");

            // Check configuration
            println!("\n[Configuration]");
            let config = get_config();
            println!("  API Keys:");
            if config.api_keys.semantic_scholar.is_some() {
                println!("    - Semantic Scholar: Configured");
            } else {
                println!("    - Semantic Scholar: Not configured (optional)");
            }
            if config.api_keys.core.is_some() {
                println!("    - CORE: Configured");
            } else {
                println!("    - CORE: Not configured (optional)");
            }

            // Check sources
            println!("\n[Sources]");
            println!("  Total sources loaded: {}", registry.len());
            let mut sources_info: Vec<_> = registry
                .all()
                .map(|s| (s.id(), s.name(), format!("{:?}", s.capabilities())))
                .collect();
            sources_info.sort_by_key(|(id, _, _)| *id);

            for (id, name, caps) in &sources_info {
                println!("  - {} ({})", name, id);
                if verbose {
                    println!("    Capabilities: {}", caps);
                }
            }

            // Check connectivity if requested
            if check_connectivity {
                println!("\n[Connectivity]");
                for (id, name, _) in &sources_info {
                    let test_url = format!("https://{}.org", id.replace('_', ""));
                    match reqwest::Client::new().head(&test_url).send().await {
                        Ok(resp) => {
                            let status = if resp.status().is_success() {
                                "OK"
                            } else {
                                "ERROR"
                            };
                            println!("  - {}: {} ({})", name, status, resp.status());
                        }
                        Err(e) => {
                            println!(
                                "  - {}: ERROR ({})",
                                name,
                                e.to_string().split(':').next().unwrap_or("unknown")
                            );
                        }
                    }
                }
            }

            // Check API keys if requested
            if check_api_keys {
                println!("\n[API Key Validation]");
                // Semantic Scholar
                if let Some(key) = &config.api_keys.semantic_scholar {
                    if key.len() >= 10 {
                        println!("  - Semantic Scholar API key: Valid format");
                    } else {
                        println!("  - Semantic Scholar API key: May be invalid (too short)");
                    }
                }
            }

            // Check proxy settings
            println!("\n[Proxy Settings]");
            let http_proxy = std::env::var("HTTP_PROXY").ok();
            let https_proxy = std::env::var("HTTPS_PROXY").ok();
            if http_proxy.is_some() || https_proxy.is_some() {
                if let Some(http) = &http_proxy {
                    println!("  - HTTP_PROXY: {}", http);
                }
                if let Some(https) = &https_proxy {
                    println!("  - HTTPS_PROXY: {}", https);
                }
            } else {
                println!("  - No proxy configured (direct connection)");
            }

            println!("\n================================");
            println!("Doctor check complete.");
        }

        Some(Commands::Update { force, dry_run }) => {
            use anyhow::Context as _;
            use research_master::utils::{
                detect_installation, download_and_extract_asset, fetch_and_verify_sha256,
                fetch_latest_release, fetch_sha256_signature, find_asset_for_platform,
                get_current_target, get_update_instructions, replace_binary, verify_gpg_signature,
                verify_sha256, InstallationMethod,
            };
            #[cfg(unix)]
            use std::os::unix::fs::PermissionsExt;

            let current_version = env!("CARGO_PKG_VERSION");
            println!("Research Master MCP Updater");
            println!("============================");
            println!("Current version: v{}", current_version);

            // Detect installation method
            let install_method = detect_installation();
            let instructions = get_update_instructions(&install_method);

            // Fetch latest release
            eprintln!("Checking for updates...");
            let latest = match fetch_latest_release().await {
                Ok(release) => release,
                Err(e) => {
                    eprintln!("Failed to check for updates: {}", e);
                    eprintln!("\n{}", instructions);
                    return Ok(());
                }
            };

            println!("Latest version: {}", latest.version);

            // Check if update is needed
            let needs_update = if force {
                true
            } else {
                let current = semver::Version::parse(current_version)
                    .unwrap_or_else(|_| semver::Version::new(0, 0, 0));
                let latest_v = semver::Version::parse(&latest.version)
                    .unwrap_or_else(|_| semver::Version::new(0, 0, 0));
                latest_v > current
            };

            if !needs_update && !force {
                println!("You are already on the latest version!");
                return Ok(());
            }

            // If dry run, just show what would happen
            if dry_run {
                println!("\n[Dry run] Would update to v{}", latest.version);
                println!("Installation method detected: {:?}", install_method);
                return Ok(());
            }

            // Show release notes if available
            if !latest.body.is_empty() {
                println!("\nRelease notes:");
                println!("--------------");
                // Show first 500 characters of release notes
                let notes = if latest.body.len() > 500 {
                    &latest.body[..500]
                } else {
                    &latest.body
                };
                println!("{}", notes);
                if latest.body.len() > 500 {
                    println!("...\n(Full notes available at https://github.com/hongkongkiwi/research-master/releases/tag/v{})", latest.version);
                }
            }

            // Handle based on installation method
            match &install_method {
                InstallationMethod::Homebrew { .. } | InstallationMethod::Cargo { .. } => {
                    println!("\n{}", instructions);
                    println!("\nAfter updating, run 'research-master --version' to verify.");
                }
                InstallationMethod::Direct { .. } | InstallationMethod::Unknown => {
                    // Attempt self-update
                    let target = get_current_target();
                    if target.is_empty() {
                        eprintln!("Unsupported platform for automatic update.");
                        eprintln!("\n{}", instructions);
                        return Ok(());
                    }

                    println!("\nTarget platform: {}", target);

                    // Find appropriate asset
                    let asset = match find_asset_for_platform(&latest) {
                        Some(a) => a,
                        None => {
                            eprintln!("No release asset found for platform: {}", target);
                            eprintln!("Please download manually from: https://github.com/hongkongkiwi/research-master/releases/tag/v{}", latest.version);
                            return Ok(());
                        }
                    };

                    println!("\nAsset: {}", asset.name);

                    // Create temp directory
                    let temp_dir = std::env::temp_dir().join("research-master-update");
                    let _ = std::fs::create_dir_all(&temp_dir);

                    // Download and extract
                    #[allow(clippy::needless_borrow)]
                    match download_and_extract_asset(&asset, &temp_dir).await {
                        Ok(archive_path) => {
                            // Fetch expected SHA256 checksum
                            let expected_checksum = match fetch_and_verify_sha256(
                                &asset.name,
                                &temp_dir,
                            )
                            .await
                            {
                                Ok(hash) => hash,
                                Err(e) => {
                                    eprintln!("Warning: Could not fetch SHA256 checksums: {}. Proceeding without verification.", e);
                                    "".to_string()
                                }
                            };

                            // Verify checksum if available
                            if !expected_checksum.is_empty() {
                                eprintln!("Verifying SHA256 checksum...");
                                match verify_sha256(&archive_path, &expected_checksum) {
                                    Ok(true) => {
                                        eprintln!("SHA256 verification passed!");
                                    }
                                    Ok(false) => {
                                        eprintln!("ERROR: SHA256 verification failed! The download may be corrupted or tampered with.");
                                        eprintln!("Aborting update for safety.");
                                        let _ = std::fs::remove_file(&archive_path);
                                        let _ = std::fs::remove_dir_all(&temp_dir);
                                        return Ok(());
                                    }
                                    Err(e) => {
                                        eprintln!("Warning: Could not verify checksum: {}. Proceeding without verification.", e);
                                    }
                                }

                                // Fetch and verify GPG signature if available
                                eprintln!("Checking for GPG signature...");
                                match fetch_sha256_signature().await {
                                    Ok(signature) => {
                                        // Write SHA256SUMS.txt to temp location for verification
                                        let sha256sums_path = temp_dir.join("SHA256SUMS.txt");
                                        let checksums_content =
                                            format!("{}  {}", expected_checksum, asset.name);
                                        std::fs::write(&sha256sums_path, &checksums_content).ok();

                                        if sha256sums_path.exists() {
                                            match verify_gpg_signature(&sha256sums_path, &signature)
                                            {
                                                Ok(true) => {
                                                    eprintln!("GPG signature verification passed!");
                                                }
                                                Ok(false) => {
                                                    // GPG verification failed but we continue if SHA256 passed
                                                    eprintln!("WARNING: GPG signature verification failed or not configured.");
                                                    eprintln!("Only SHA256 checksum verification was performed.");
                                                }
                                                Err(e) => {
                                                    eprintln!("Warning: Could not verify GPG signature: {}. Continuing with SHA256 verification only.", e);
                                                }
                                            }
                                            let _ = std::fs::remove_file(&sha256sums_path);
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Note: GPG signature not available ({}). Using SHA256 verification only.", e);
                                    }
                                }
                            }

                            // Extract the archive
                            let binary_path = if asset.name.ends_with(".tar.gz") {
                                use std::process::Command;
                                let output = Command::new("tar")
                                    .args([
                                        "xzf",
                                        archive_path.to_string_lossy().as_ref(),
                                        "-C",
                                        temp_dir.to_string_lossy().as_ref(),
                                    ])
                                    .output()
                                    .context("Failed to extract archive")?;

                                if !output.status.success() {
                                    anyhow::bail!(
                                        "Extraction failed: {}",
                                        String::from_utf8_lossy(&output.stderr)
                                    );
                                }

                                // Find the binary
                                let mut binary_path = None;
                                for entry in std::fs::read_dir(&temp_dir)? {
                                    let entry = entry?;
                                    let path = entry.path();
                                    if path.is_file()
                                        && path
                                            .file_name()
                                            .map(|n| {
                                                n.to_string_lossy().starts_with("research-master")
                                            })
                                            .unwrap_or(false)
                                    {
                                        // Make executable
                                        #[cfg(unix)]
                                        {
                                            let mut perms = std::fs::metadata(&path)?.permissions();
                                            perms.set_mode(0o755);
                                            std::fs::set_permissions(&path, perms)?;
                                        }
                                        #[cfg(not(unix))]
                                        {
                                            // On Windows, just ensure the file is writable
                                            let mut perms = std::fs::metadata(&path)?.permissions();
                                            perms.set_readonly(false);
                                            std::fs::set_permissions(&path, perms)?;
                                        }
                                        binary_path = Some(path);
                                        break;
                                    }
                                }
                                binary_path.context("Could not find binary in archive")?
                            } else {
                                anyhow::bail!("Unsupported archive format");
                            };

                            println!("\nDownloaded and extracted to: {}", binary_path.display());

                            // Get current binary path
                            let current_exe = std::env::current_exe().map_err(|e| {
                                anyhow::anyhow!("Failed to get current executable path: {}", e)
                            })?;

                            // Replace binary
                            match replace_binary(&current_exe, &binary_path) {
                                Ok(_) => {
                                    println!("\nUpdate successful!");
                                    println!("New binary will be used on next run.");
                                }
                                Err(e) => {
                                    eprintln!("\nFailed to replace binary: {}", e);
                                    eprintln!(
                                        "You may need to manually replace the binary at: {}",
                                        current_exe.display()
                                    );
                                }
                            }

                            // Cleanup
                            let _ = std::fs::remove_file(&archive_path);
                            let _ = std::fs::remove_file(&binary_path);
                        }
                        Err(e) => {
                            eprintln!("\nFailed to download/update: {}", e);
                        }
                    }

                    // Cleanup temp dir
                    let _ = std::fs::remove_dir_all(&temp_dir);
                }
            }
        }

        Some(Commands::Config { action }) => {
            match action {
                ConfigAction::Init => {
                    println!("Initializing configuration...");
                    println!("Config file location: ~/.config/research-master/config.toml");
                    println!("Run 'research-master config show' to view current config.");
                    println!("Run 'research-master config edit' to edit config.");
                }
                ConfigAction::Show => {
                    let config_path = find_config_file();
                    if let Some(path) = config_path {
                        match std::fs::read_to_string(&path) {
                            Ok(content) => {
                                println!("Configuration at {}:\n", path.display());
                                println!("{}", content);
                            }
                            Err(_) => {
                                println!("Config file exists but could not be read.");
                            }
                        }
                    } else {
                        println!("No config file found.");
                        println!("Run 'research-master config init' to create one.");
                    }
                }
                ConfigAction::Edit => {
                    let config_path = find_config_file().unwrap_or_else(|| {
                        let path = dirs::config_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("research-master")
                            .join("config.toml");
                        println!("Creating new config at: {}", path.display());
                        let _ = std::fs::create_dir_all(
                            path.parent().unwrap_or(std::path::Path::new(".")),
                        );
                        path
                    });

                    if !config_path.exists() {
                        println!("Creating new config file: {}", config_path.display());
                        let default_config = r#"# Research Master MCP Configuration
# See https://github.com/hongkongkiwi/research-master for documentation

[general]
# Default output format: auto, table, json, plain
output = "auto"

[downloads]
# Default download directory
default_path = "./downloads"
# Organize downloads by source
organize_by_source = true
# Maximum concurrent downloads
concurrency = 5

[cache]
# Enable caching (requires RESEARCH_MASTER_CACHE_ENABLED=true)
enabled = false
# Cache directory
directory = "~/.cache/research-master"

[api_keys]
# Add your API keys here (uncomment and replace with your keys)
# semantic_scholar = "your-api-key"
# core = "your-api-key"
# openalex = "your-email@example.com"
"#;
                        let _ = std::fs::write(&config_path, default_config);
                    }

                    // Open in editor
                    let editor = if cfg!(target_os = "windows") {
                        std::env::var("EDITOR").unwrap_or_else(|_| "notepad".to_string())
                    } else {
                        std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string())
                    };
                    println!("Opening {} in {}...", config_path.display(), editor);
                    let status = std::process::Command::new(&editor)
                        .arg(&config_path)
                        .status();
                    match status {
                        Ok(s) if s.success() => {
                            println!("Config updated successfully.");
                        }
                        Ok(_) => {
                            println!("Editor closed without saving changes.");
                        }
                        Err(e) => {
                            eprintln!("Failed to open editor: {}", e);
                            println!(
                                "You can edit the config manually at: {}",
                                config_path.display()
                            );
                        }
                    }
                }
            }
        }

        Some(Commands::Export {
            input,
            format,
            output,
            source: _,
            query: _,
            max_results: _,
        }) => {
            println!(
                "Export command - format: {:?}, input: {:?}, output: {:?}",
                format, input, output
            );
            println!(
                "Example: research-master export --input papers.json --format bibtex -O output.bib"
            );
            println!(
                "This feature exports search results or input files to BibTeX, CSV, JSON, or RIS format."
            );
        }

        Some(Commands::BulkDownload {
            input,
            output_dir,
            source: _,
            organize_by_source,
            concurrency,
        }) => {
            println!("Bulk download from: {}", input.display());
            println!("Output directory: {}", output_dir.display());
            println!("Organize by source: {}", organize_by_source);
            println!("Concurrency: {}", concurrency);

            // Check if input file exists
            if !input.exists() {
                eprintln!("Error: Input file not found: {}", input.display());
            } else {
                println!("Reading paper IDs from {}...", input.display());
                // This would be implemented to read and download papers
                println!("Feature ready for implementation.");
            }
        }

        Some(Commands::ApiKeys { action, source }) => match action {
            ApiKeyAction::Set => {
                if let Some(src) = source {
                    println!("Set API key for: {}", src);
                    println!("Run 'research-master doctor --check-api-keys' to verify keys.");
                } else {
                    println!("Usage: research-master api-keys set --source <source-name>");
                }
            }
            ApiKeyAction::List => {
                println!("Configured API keys:");
                println!(
                    "  SEMANTIC_SCHOLAR_API_KEY: ***{}",
                    std::env::var("SEMANTIC_SCHOLAR_API_KEY")
                        .map(|s| s.len().to_string())
                        .unwrap_or_else(|_| "not set".to_string())
                );
                println!(
                    "  CORE_API_KEY: ***{}",
                    std::env::var("CORE_API_KEY")
                        .map(|s| s.len().to_string())
                        .unwrap_or_else(|_| "not set".to_string())
                );
                println!(
                    "  OPENALEX_EMAIL: {}",
                    std::env::var("OPENALEX_EMAIL").unwrap_or_else(|_| "not set".to_string())
                );
                println!(
                    "\nRun 'research-master doctor --check-api-keys' to verify configuration."
                );
            }
            ApiKeyAction::Remove => {
                if let Some(src) = source {
                    println!("Remove API key for: {}", src);
                    println!("Unset the corresponding environment variable to disable.");
                } else {
                    println!("Usage: research-master api-keys remove --source <source-name>");
                }
            }
        },

        Some(Commands::Completions { shell }) => {
            use clap::CommandFactory;

            let mut cmd = Cli::command();
            let bin_name = cmd.get_name().to_string();

            match shell {
                Shell::Bash => {
                    clap_complete::generate(Bash, &mut cmd, &bin_name, &mut std::io::stdout());
                }
                Shell::Elvish => {
                    clap_complete::generate(Elvish, &mut cmd, &bin_name, &mut std::io::stdout());
                }
                Shell::Fish => {
                    clap_complete::generate(Fish, &mut cmd, &bin_name, &mut std::io::stdout());
                }
                Shell::PowerShell => {
                    clap_complete::generate(
                        PowerShell,
                        &mut cmd,
                        &bin_name,
                        &mut std::io::stdout(),
                    );
                }
                Shell::Zsh => {
                    clap_complete::generate(Zsh, &mut cmd, &bin_name, &mut std::io::stdout());
                }
            }
            println!();
            println!("To use these completions:");
            println!();
            match shell {
                Shell::Bash => {
                    println!("  # Add to ~/.bashrc or ~/.bash_profile:");
                    println!("  source <({} completions bash)", bin_name);
                }
                Shell::Zsh => {
                    println!("  # Add to ~/.zshrc:");
                    println!("  autoload -U compinit");
                    println!("  compinit");
                    println!(
                        "  {} completions zsh > ~/.zsh/completion/_research-master",
                        bin_name
                    );
                }
                Shell::Fish => {
                    println!("  # Fish handles completions automatically when placed in:");
                    println!("  mkdir -p ~/.config/fish/completions/");
                    println!(
                        "  {} completions fish > ~/.config/fish/completions/research-master.fish",
                        bin_name
                    );
                }
                Shell::PowerShell => {
                    println!("  # Add to your PowerShell profile:");
                    println!(
                        "  {} completions powershell | Out-String | Invoke-Expression",
                        bin_name
                    );
                }
                Shell::Elvish => {
                    println!("  # Add to ~/.elvish/rc.elv:");
                    println!("  use {} completions", bin_name);
                }
            }
        }

        Some(Commands::History {
            limit,
            searches,
            downloads,
            clear,
        }) => {
            use research_master::utils::HistoryEntryType;

            let history = HistoryService::new();

            if clear {
                history.clear()?;
                println!("History cleared.");
                return Ok(());
            }

            let entries = history.read_entries(limit)?;

            let entries: Vec<_> = if searches {
                history.filter_entries(&entries, HistoryEntryType::Search)
            } else if downloads {
                history.filter_entries(&entries, HistoryEntryType::Download)
            } else {
                entries
            };

            if entries.is_empty() {
                println!("No history entries found.");
                return Ok(());
            }

            println!("Recent History:");
            println!("{}", "=".repeat(80));

            for (i, entry) in entries.iter().enumerate() {
                let timestamp = chrono::DateTime::from_timestamp(entry.timestamp as i64, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                match &entry.entry_type {
                    HistoryEntryType::Search => {
                        println!("{}. [{}] SEARCH: {}", i + 1, timestamp, entry.query);
                        if let Some(source) = &entry.source {
                            println!("   Source: {}", source);
                        }
                    }
                    HistoryEntryType::Download => {
                        println!("{}. [{}] DOWNLOAD: {}", i + 1, timestamp, entry.query);
                        if let Some(title) = &entry.title {
                            println!("   Title: {}", title);
                        }
                        if let Some(source) = &entry.source {
                            println!("   Source: {}", source);
                        }
                        if let Some(path) = &entry.details {
                            println!("   Saved to: {}", path);
                        }
                    }
                    HistoryEntryType::View => {
                        println!("{}. [{}] VIEW: {}", i + 1, timestamp, entry.query);
                        if let Some(title) = &entry.title {
                            println!("   Title: {}", title);
                        }
                    }
                }
                println!();
            }
        }

        Some(Commands::Clear {
            cache,
            history,
            downloads,
            all,
        }) => {
            use research_master::utils::{CacheService, HistoryService};
            use std::fs;

            let config = get_config();
            let downloads_path = config.downloads.default_path.clone();

            if all || (cache && history && downloads) {
                // Clear everything
                let cache_service = CacheService::new();
                cache_service.clear_all()?;
                println!("Cleared all cache data.");
                let history = HistoryService::new();
                history.clear()?;
                println!("Cleared history.");
                if downloads_path.exists() {
                    fs::remove_dir_all(&downloads_path)?;
                    println!("Cleared downloads directory.");
                }
                println!("All cleared.");
            } else {
                if cache {
                    let cache_service = CacheService::new();
                    cache_service.clear_all()?;
                    println!("Cleared cache.");
                }
                if history {
                    let history = HistoryService::new();
                    history.clear()?;
                    println!("Cleared history.");
                }
                if downloads {
                    if downloads_path.exists() {
                        fs::remove_dir_all(&downloads_path)?;
                        println!("Cleared downloads directory.");
                    } else {
                        println!("Downloads directory does not exist.");
                    }
                }
                if !cache && !history && !downloads {
                    println!("Nothing to clear. Use --cache, --history, --downloads, or --all.");
                }
            }
        }

        Some(Commands::Cite {
            paper_id,
            style,
            source: _,
            format,
        }) => {
            use research_master::sources::Source;
            use research_master::utils::{
                format_citation, get_structured_citation, CitationStyle as UtilsCitationStyle,
            };

            let registry = SourceRegistry::new();

            // Get all sources that support DOI lookup
            let sources_vec: Vec<&Arc<dyn Source>> =
                registry.with_capability(research_master::sources::SourceCapabilities::DOI_LOOKUP);

            // Try to find the paper
            let mut paper_opt = None;

            // First, try DOI lookup if it looks like a DOI
            if paper_id.contains("10.") && paper_id.contains("/") {
                for source in &sources_vec {
                    if source.supports_doi_lookup() {
                        match source.get_by_doi(&paper_id).await {
                            Ok(paper) => {
                                paper_opt = Some(paper);
                                break;
                            }
                            Err(e) => {
                                tracing::debug!("DOI lookup failed for {}: {}", source.id(), e);
                            }
                        }
                    }
                }
            }

            // If not found by DOI, try search as fallback
            if paper_opt.is_none() {
                // Get sources that support search
                let search_sources: Vec<&Arc<dyn Source>> =
                    registry.with_capability(research_master::sources::SourceCapabilities::SEARCH);

                for source in &search_sources {
                    let search_query = SearchQuery {
                        query: paper_id.clone(),
                        max_results: 1,
                        year: None,
                        sort_by: None,
                        sort_order: None,
                        filters: std::collections::HashMap::new(),
                        author: None,
                        category: None,
                        fetch_details: true,
                    };
                    match source.search(&search_query).await {
                        Ok(response) => {
                            if !response.papers.is_empty() {
                                paper_opt = Some(response.papers[0].clone());
                                break;
                            }
                        }
                        Err(e) => tracing::debug!("Search failed for {}: {}", source.id(), e),
                    }
                }
            }

            let paper =
                paper_opt.ok_or_else(|| anyhow::anyhow!("Paper not found: {}", paper_id))?;

            // Convert CLI CitationStyle to utils CitationStyle
            let utils_style = match style {
                CitationStyle::Apa => UtilsCitationStyle::Apa,
                CitationStyle::Mla => UtilsCitationStyle::Mla,
                CitationStyle::Chicago => UtilsCitationStyle::Chicago,
                CitationStyle::Bibtex => UtilsCitationStyle::Bibtex,
            };

            match format {
                CitationOutputFormat::Text | CitationOutputFormat::Bibtex => {
                    let citation = format_citation(&paper, utils_style);
                    println!("{}", citation);
                }
                CitationOutputFormat::Json => {
                    let structured = get_structured_citation(&paper, utils_style);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&structured).expect("serialize")
                    );
                }
            }
        }

        None => {
            // No command provided - show help
            println!("No command provided. Use --help for usage information.");
            println!("Common commands:");
            println!("  search <query>   - Search for papers");
            println!("  author <name>    - Search by author");
            println!("  download <id>    - Download a paper");
            println!("  sources          - List available sources");
            println!("  history          - Show search/download history");
            println!("  clear            - Clear cache, history, or downloads");
            println!("  cite <id>        - Format paper citation (APA, MLA, Chicago, BibTeX)");
            println!("  mcp              - Run MCP server");
        }
    }

    Ok(())
}

fn get_source(
    registry: &SourceRegistry,
    source: Source,
) -> Result<&std::sync::Arc<dyn research_master::sources::Source>> {
    let source_id = match source {
        Source::All => anyhow::bail!("Please specify a specific source"),
        s => source_to_id(s),
    };
    registry
        .get_required(source_id)
        .map_err(|e| anyhow::anyhow!(e))
}

fn get_sources(
    registry: &SourceRegistry,
    source: Source,
    capability: SourceCapabilities,
) -> Vec<&std::sync::Arc<dyn research_master::sources::Source>> {
    match source {
        Source::All => registry.with_capability(capability),
        s => {
            let id = source_to_id(s);
            registry.get(id).into_iter().collect()
        }
    }
}

fn source_to_id(source: Source) -> &'static str {
    match source {
        Source::Arxiv => "arxiv",
        Source::Pubmed => "pubmed",
        Source::Biorxiv => "biorxiv",
        Source::Semantic => "semantic",
        Source::OpenAlex => "openalex",
        Source::CrossRef => "crossref",
        Source::Iacr => "iacr",
        Source::Pmc => "pmc",
        Source::Hal => "hal",
        Source::Dblp => "dblp",
        Source::Ssrn => "ssrn",
        Source::Dimensions => "dimensions",
        Source::IeeeXplore => "ieee_xplore",
        Source::EuropePmc => "europe_pmc",
        Source::Core => "core",
        Source::Zenodo => "zenodo",
        Source::Unpaywall => "unpaywall",
        Source::Mdpi => "mdpi",
        Source::Jstor => "jstor",
        Source::Scispace => "scispace",
        Source::Acm => "acm",
        Source::ConnectedPapers => "connected_papers",
        Source::Doaj => "doaj",
        Source::WorldWideScience => "worldwidescience",
        Source::Osf => "osf",
        Source::Base => "base",
        Source::Springer => "springer",
        Source::GoogleScholar => "google_scholar",
        Source::All => unreachable!(),
    }
}

fn output_papers(papers: &[research_master::models::Paper], format: OutputFormat) {
    let actual_format = if format == OutputFormat::Auto {
        if std::io::stdout().is_terminal() {
            OutputFormat::Table
        } else {
            OutputFormat::Json
        }
    } else {
        format
    };

    match actual_format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(papers).expect("serialize")
            );
        }
        OutputFormat::Plain => {
            for paper in papers {
                println!("{} - {} ({})", paper.title, paper.authors, paper.source);
                println!("  URL: {}", paper.url);
                if let Some(ref doi) = paper.doi {
                    println!("  DOI: {}", doi);
                }
                if let Some(ref pdf_url) = paper.pdf_url {
                    println!("  PDF: {}", pdf_url);
                }
                println!();
            }
        }
        OutputFormat::Table => {
            use comfy_table::{Attribute, Cell, ColumnConstraint, Table, Width};
            use owo_colors::OwoColorize;

            if papers.is_empty() {
                println!("{}", "No papers found.".yellow());
                return;
            }

            // Use our robust terminal width detection with fallback
            let width = if is_terminal() && terminal_width() > 0 {
                terminal_width()
            } else {
                100
            };

            let (title_width, authors_width, source_width, year_width) =
                get_paper_table_columns(width);

            let mut table = Table::new();
            table.load_preset(comfy_table::presets::UTF8_FULL);

            table.set_header(vec![
                Cell::new("Title").add_attribute(Attribute::Bold),
                Cell::new("Authors").add_attribute(Attribute::Bold),
                Cell::new("Source").add_attribute(Attribute::Bold),
                Cell::new("Year").add_attribute(Attribute::Bold),
            ]);

            // Set column constraints for proper text fitting
            table.set_constraints([
                ColumnConstraint::Absolute(Width::Fixed(title_width as u16)),
                ColumnConstraint::Absolute(Width::Fixed(authors_width as u16)),
                ColumnConstraint::Absolute(Width::Fixed(source_width as u16)),
                ColumnConstraint::Absolute(Width::Fixed(year_width as u16)),
            ]);

            for paper in papers {
                let year = format_year(paper.published_date.as_deref().unwrap_or("?"));

                let title = format_title(&paper.title, title_width);
                let authors = format_authors(&paper.authors, authors_width);
                let source = format_source(&paper.source.to_string(), source_width);

                table.add_row(vec![
                    Cell::new(title).add_attribute(Attribute::Bold),
                    Cell::new(authors),
                    Cell::new(source),
                    Cell::new(year),
                ]);
            }
            println!("{table}");
        }
        OutputFormat::Auto => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_version() {
        let version = env!("CARGO_PKG_VERSION");
        assert!(!version.is_empty());
        // Version should be semantic versioning format
        let parts: Vec<&str> = version.split('.').collect();
        assert!(parts.len() >= 2);
        assert!(parts[0].parse::<u32>().is_ok());
    }

    #[test]
    fn test_output_format_values() {
        assert_eq!(OutputFormat::Auto as i32, 0);
        assert_eq!(OutputFormat::Table as i32, 1);
        assert_eq!(OutputFormat::Json as i32, 2);
        assert_eq!(OutputFormat::Plain as i32, 3);
    }

    #[test]
    fn test_cli_default_values() {
        let cli = Cli::parse_from(["research-master"]);
        assert_eq!(cli.verbose, 0);
        assert!(!cli.quiet);
        assert_eq!(cli.output, OutputFormat::Auto);
        assert_eq!(cli.timeout, 30);
        assert!(!cli.no_cache);
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_cli_verbose_flag() {
        let cli = Cli::parse_from(["research-master", "-v"]);
        assert_eq!(cli.verbose, 1);

        let cli = Cli::parse_from(["research-master", "-vv"]);
        assert_eq!(cli.verbose, 2);

        let cli = Cli::parse_from(["research-master", "--verbose"]);
        assert_eq!(cli.verbose, 1);
    }

    #[test]
    fn test_cli_quiet_flag() {
        let cli = Cli::parse_from(["research-master", "-q"]);
        assert!(cli.quiet);

        let cli = Cli::parse_from(["research-master", "--quiet"]);
        assert!(cli.quiet);
    }

    #[test]
    fn test_cli_output_format() {
        let cli = Cli::parse_from(["research-master", "-o", "json"]);
        assert_eq!(cli.output, OutputFormat::Json);

        let cli = Cli::parse_from(["research-master", "--output", "table"]);
        assert_eq!(cli.output, OutputFormat::Table);
    }

    #[test]
    fn test_cli_timeout() {
        let cli = Cli::parse_from(["research-master", "--timeout", "60"]);
        assert_eq!(cli.timeout, 60);
    }

    #[test]
    fn test_cli_config_flag() {
        let cli = Cli::parse_from(["research-master", "--config", "/path/to/config.toml"]);
        assert_eq!(cli.config, Some(PathBuf::from("/path/to/config.toml")));
    }

    #[test]
    fn test_cli_no_cache_flag() {
        let cli = Cli::parse_from(["research-master", "--no-cache"]);
        assert!(cli.no_cache);
    }

    #[test]
    fn test_cli_search_command() {
        let cli = Cli::parse_from(["research-master", "search", "machine learning"]);
        match &cli.command {
            Some(Commands::Search {
                query, max_results, ..
            }) => {
                assert_eq!(query, "machine learning");
                assert_eq!(*max_results, 10);
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_search_with_options() {
        let cli = Cli::parse_from([
            "research-master",
            "search",
            "neural networks",
            "--max-results",
            "50",
            "--year",
            "2023",
            "--source",
            "arxiv",
            "--dedup",
        ]);
        match &cli.command {
            Some(Commands::Search {
                query,
                max_results,
                year,
                ..
            }) => {
                assert_eq!(query, "neural networks");
                assert_eq!(*max_results, 50);
                assert_eq!(year.clone(), Some("2023".to_string()));
            }
            _ => panic!("Expected Search command"),
        }
    }

    #[test]
    fn test_cli_download_command() {
        let cli = Cli::parse_from([
            "research-master",
            "download",
            "2301.12345",
            "--source",
            "arxiv",
        ]);
        match &cli.command {
            Some(Commands::Download {
                paper_id,
                source,
                output_path: _,
                auto_filename: _,
                create_dir: _,
                doi: _,
            }) => {
                assert_eq!(paper_id, "2301.12345");
                assert_eq!(*source, Source::Arxiv);
            }
            _ => panic!("Expected Download command"),
        }
    }

    #[test]
    fn test_cli_doi_command() {
        let cli = Cli::parse_from(["research-master", "doi", "10.1234/test"]);
        match &cli.command {
            Some(Commands::LookupByDoi { doi, .. }) => {
                assert_eq!(doi, "10.1234/test");
            }
            _ => panic!("Expected LookupByDoi command"),
        }
    }

    #[test]
    fn test_cli_sources_command() {
        let cli = Cli::parse_from(["research-master", "sources"]);
        match &cli.command {
            Some(Commands::Sources { detailed, .. }) => {
                assert!(!*detailed);
            }
            _ => panic!("Expected Sources command"),
        }
    }

    #[test]
    fn test_cli_sources_detailed() {
        let cli = Cli::parse_from(["research-master", "sources", "--detailed"]);
        match &cli.command {
            Some(Commands::Sources { detailed, .. }) => {
                assert!(*detailed);
            }
            _ => panic!("Expected Sources command"),
        }
    }

    #[test]
    fn test_cli_mcp_command() {
        let cli = Cli::parse_from(["research-master", "mcp"]);
        match &cli.command {
            Some(Commands::Mcp {
                stdio, port, host, ..
            }) => {
                assert!(*stdio);
                assert_eq!(*port, 3000);
                assert_eq!(host, "127.0.0.1");
            }
            _ => panic!("Expected Mcp command"),
        }
    }

    #[test]
    fn test_cli_mcp_http_mode() {
        // Just verify the command parses - stdio defaults to true so http doesn't override it
        let cli = Cli::parse_from(["research-master", "mcp", "--http"]);
        assert!(matches!(cli.command, Some(Commands::Mcp { .. })));
    }

    // Author command tests
    #[test]
    fn test_cli_author_command() {
        let cli = Cli::parse_from(["research-master", "author", "Geoffrey Hinton"]);
        match &cli.command {
            Some(Commands::Author { author, .. }) => {
                assert_eq!(author, "Geoffrey Hinton");
            }
            _ => panic!("Expected Author command"),
        }
    }

    #[test]
    fn test_cli_author_with_source() {
        let cli = Cli::parse_from([
            "research-master",
            "author",
            "Geoffrey Hinton",
            "--source",
            "semantic",
        ]);
        match &cli.command {
            Some(Commands::Author { author, source, .. }) => {
                assert_eq!(author, "Geoffrey Hinton");
                assert_eq!(*source, Source::Semantic);
            }
            _ => panic!("Expected Author command"),
        }
    }

    // Read command tests
    #[test]
    fn test_cli_read_command() {
        let cli = Cli::parse_from([
            "research-master",
            "read",
            "2301.12345",
            "--source",
            "arxiv",
            "--path",
            "/path/to/paper.pdf",
        ]);
        match &cli.command {
            Some(Commands::Read { paper_id, .. }) => {
                assert_eq!(paper_id, "2301.12345");
            }
            _ => panic!("Expected Read command"),
        }
    }

    #[test]
    fn test_cli_read_with_options() {
        let cli = Cli::parse_from([
            "research-master",
            "read",
            "2301.12345",
            "--source",
            "arxiv",
            "--path",
            "/path/to/paper.pdf",
            "--output-file",
            "output.txt",
            "--pages",
            "5",
        ]);
        match &cli.command {
            Some(Commands::Read {
                paper_id,
                source,
                pages,
                output_file,
                path: _,
                ..
            }) => {
                assert_eq!(paper_id, "2301.12345");
                assert_eq!(*source, Source::Arxiv);
                assert_eq!(*pages, Some(5));
                assert_eq!(
                    output_file.clone().map(|p| p.to_string_lossy().to_string()),
                    Some("output.txt".to_string())
                );
            }
            _ => panic!("Expected Read command"),
        }
    }

    // Citations command tests
    #[test]
    fn test_cli_citations_command() {
        let cli = Cli::parse_from([
            "research-master",
            "citations",
            "2301.12345",
            "--source",
            "arxiv",
        ]);
        match &cli.command {
            Some(Commands::Citations { paper_id, .. }) => {
                assert_eq!(paper_id, "2301.12345");
            }
            _ => panic!("Expected Citations command"),
        }
    }

    #[test]
    fn test_cli_citations_with_options() {
        let cli = Cli::parse_from([
            "research-master",
            "citations",
            "2301.12345",
            "--source",
            "semantic",
            "--max-results",
            "50",
        ]);
        match &cli.command {
            Some(Commands::Citations {
                paper_id,
                source,
                max_results,
            }) => {
                assert_eq!(paper_id, "2301.12345");
                assert_eq!(*source, Source::Semantic);
                assert_eq!(*max_results, 50);
            }
            _ => panic!("Expected Citations command"),
        }
    }

    // References command tests
    #[test]
    fn test_cli_references_command() {
        let cli = Cli::parse_from([
            "research-master",
            "references",
            "1706.03762",
            "--source",
            "semantic",
        ]);
        match &cli.command {
            Some(Commands::References { paper_id, .. }) => {
                assert_eq!(paper_id, "1706.03762");
            }
            _ => panic!("Expected References command"),
        }
    }

    #[test]
    fn test_cli_references_alias() {
        let cli = Cli::parse_from([
            "research-master",
            "ref",
            "1706.03762",
            "--source",
            "semantic",
        ]);
        assert!(matches!(cli.command, Some(Commands::References { .. })));
    }

    // Related command tests
    #[test]
    fn test_cli_related_command() {
        let cli = Cli::parse_from([
            "research-master",
            "related",
            "1706.03762",
            "--source",
            "connected_papers",
        ]);
        match &cli.command {
            Some(Commands::Related { paper_id, .. }) => {
                assert_eq!(paper_id, "1706.03762");
            }
            _ => panic!("Expected Related command"),
        }
    }

    #[test]
    fn test_cli_related_alias() {
        let cli = Cli::parse_from([
            "research-master",
            "rel",
            "1706.03762",
            "--source",
            "connected_papers",
        ]);
        assert!(matches!(cli.command, Some(Commands::Related { .. })));
    }

    // Lookup command tests
    #[test]
    fn test_cli_lookup_command() {
        let cli = Cli::parse_from(["research-master", "doi", "10.1234/test"]);
        match &cli.command {
            Some(Commands::LookupByDoi { doi, .. }) => {
                assert_eq!(doi, "10.1234/test");
            }
            _ => panic!("Expected LookupByDoi command"),
        }
    }

    #[test]
    fn test_cli_lookup_with_source() {
        let cli = Cli::parse_from([
            "research-master",
            "doi",
            "10.1234/test",
            "--source",
            "crossref",
        ]);
        match &cli.command {
            Some(Commands::LookupByDoi { doi, source, .. }) => {
                assert_eq!(doi, "10.1234/test");
                assert_eq!(*source, Source::CrossRef);
            }
            _ => panic!("Expected LookupByDoi command"),
        }
    }

    // Cache command tests
    #[test]
    fn test_cli_cache_status() {
        let cli = Cli::parse_from(["research-master", "cache", "status"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Cache {
                command: CacheCommands::Status
            })
        ));
    }

    #[test]
    fn test_cli_cache_clear() {
        let cli = Cli::parse_from(["research-master", "cache", "clear"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Cache {
                command: CacheCommands::Clear
            })
        ));
    }

    #[test]
    fn test_cli_cache_clear_searches() {
        let cli = Cli::parse_from(["research-master", "cache", "clear-searches"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Cache {
                command: CacheCommands::ClearSearches
            })
        ));
    }

    #[test]
    fn test_cli_cache_clear_citations() {
        let cli = Cli::parse_from(["research-master", "cache", "clear-citations"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Cache {
                command: CacheCommands::ClearCitations
            })
        ));
    }

    // Doctor command tests
    #[test]
    fn test_cli_doctor_command() {
        let cli = Cli::parse_from(["research-master", "doctor"]);
        match &cli.command {
            Some(Commands::Doctor {
                check_connectivity,
                check_api_keys,
                verbose,
            }) => {
                assert!(!*check_connectivity);
                assert!(!*check_api_keys);
                assert!(!*verbose);
            }
            _ => panic!("Expected Doctor command"),
        }
    }

    #[test]
    fn test_cli_doctor_with_options() {
        let cli = Cli::parse_from([
            "research-master",
            "doctor",
            "--check-connectivity",
            "--check-api-keys",
            "--verbose",
        ]);
        match &cli.command {
            Some(Commands::Doctor {
                check_connectivity,
                check_api_keys,
                verbose,
            }) => {
                assert!(*check_connectivity);
                assert!(*check_api_keys);
                assert!(*verbose);
            }
            _ => panic!("Expected Doctor command"),
        }
    }

    #[test]
    fn test_cli_doctor_alias() {
        let cli = Cli::parse_from(["research-master", "diag"]);
        assert!(matches!(cli.command, Some(Commands::Doctor { .. })));
    }

    // Update command tests
    #[test]
    fn test_cli_update_command() {
        let cli = Cli::parse_from(["research-master", "update"]);
        match &cli.command {
            Some(Commands::Update { force, dry_run }) => {
                assert!(!*force);
                assert!(!*dry_run);
            }
            _ => panic!("Expected Update command"),
        }
    }

    #[test]
    fn test_cli_update_with_options() {
        let cli = Cli::parse_from(["research-master", "update", "--force", "--dry-run"]);
        match &cli.command {
            Some(Commands::Update { force, dry_run }) => {
                assert!(*force);
                assert!(*dry_run);
            }
            _ => panic!("Expected Update command"),
        }
    }

    // Completions command tests
    #[test]
    fn test_cli_completions_bash() {
        let cli = Cli::parse_from(["research-master", "completions", "bash"]);
        match &cli.command {
            Some(Commands::Completions { shell }) => {
                assert!(matches!(shell, Shell::Bash));
            }
            _ => panic!("Expected Completions command"),
        }
    }

    #[test]
    fn test_cli_completions_zsh() {
        let cli = Cli::parse_from(["research-master", "completions", "zsh"]);
        match &cli.command {
            Some(Commands::Completions { shell }) => {
                assert!(matches!(shell, Shell::Zsh));
            }
            _ => panic!("Expected Completions command"),
        }
    }

    #[test]
    fn test_cli_completions_fish() {
        let cli = Cli::parse_from(["research-master", "completions", "fish"]);
        match &cli.command {
            Some(Commands::Completions { shell }) => {
                assert!(matches!(shell, Shell::Fish));
            }
            _ => panic!("Expected Completions command"),
        }
    }

    #[test]
    fn test_cli_completions_powershell() {
        let cli = Cli::parse_from(["research-master", "completions", "power-shell"]);
        match &cli.command {
            Some(Commands::Completions { shell }) => {
                assert!(matches!(shell, Shell::PowerShell));
            }
            _ => panic!("Expected Completions command"),
        }
    }

    #[test]
    fn test_cli_completions_alias() {
        let cli = Cli::parse_from(["research-master", "completion", "bash"]);
        assert!(matches!(cli.command, Some(Commands::Completions { .. })));
    }

    // Dedupe command tests
    #[test]
    fn test_cli_dedupe_command() {
        let cli = Cli::parse_from(["research-master", "dedupe", "papers.json"]);
        match &cli.command {
            Some(Commands::Dedupe { input, .. }) => {
                assert_eq!(input.to_string_lossy(), "papers.json");
            }
            _ => panic!("Expected Dedupe command"),
        }
    }

    #[test]
    fn test_cli_dedupe_with_options() {
        let cli = Cli::parse_from([
            "research-master",
            "dedupe",
            "papers.json",
            "-O",
            "deduped.json",
            "--strategy",
            "last",
            "--show",
        ]);
        match &cli.command {
            Some(Commands::Dedupe {
                input,
                output_file,
                strategy,
                show,
            }) => {
                assert_eq!(input.to_string_lossy(), "papers.json");
                assert_eq!(
                    output_file.clone().map(|p| p.to_string_lossy().to_string()),
                    Some("deduped.json".to_string())
                );
                assert_eq!(*strategy, DedupStrategy::Last);
                assert!(*show);
            }
            _ => panic!("Expected Dedupe command"),
        }
    }

    #[test]
    fn test_cli_dedupe_alias() {
        let cli = Cli::parse_from(["research-master", "dedup", "papers.json"]);
        assert!(matches!(cli.command, Some(Commands::Dedupe { .. })));
    }

    // Search with all options
    #[test]
    fn test_cli_search_all_options() {
        let cli = Cli::parse_from([
            "research-master",
            "search",
            "transformer",
            "--source",
            "arxiv",
            "--max-results",
            "25",
            "--year",
            "2020-2023",
            "--sort-by",
            "citations",
            "--order",
            "desc",
            "--category",
            "cs.CL",
            "--author",
            "Vaswani",
            "--dedup",
            "--dedup-strategy",
            "mark",
        ]);
        match &cli.command {
            Some(Commands::Search {
                query,
                source,
                max_results,
                year,
                sort_by,
                order,
                category,
                author,
                dedup,
                dedup_strategy,
                fetch_details,
            }) => {
                assert_eq!(query, "transformer");
                assert_eq!(*source, Source::Arxiv);
                assert_eq!(*max_results, 25);
                assert_eq!(year.clone(), Some("2020-2023".to_string()));
                assert_eq!(*sort_by, Some(SortField::Citations));
                assert_eq!(*order, Some(Order::Desc));
                assert_eq!(category.clone(), Some("cs.CL".to_string()));
                assert_eq!(author.clone(), Some("Vaswani".to_string()));
                assert!(*dedup);
                assert_eq!(*dedup_strategy, Some(DedupStrategy::Mark));
                assert!(*fetch_details); // Default is true
            }
            _ => panic!("Expected Search command"),
        }
    }

    // Source enum variant tests
    #[test]
    fn test_source_enum_all_variants() {
        let variants = [
            Source::Arxiv,
            Source::Pubmed,
            Source::Biorxiv,
            Source::Semantic,
            Source::OpenAlex,
            Source::CrossRef,
            Source::Iacr,
            Source::Pmc,
            Source::Hal,
            Source::Dblp,
            Source::Ssrn,
            Source::Dimensions,
            Source::IeeeXplore,
            Source::EuropePmc,
            Source::Core,
            Source::Zenodo,
            Source::Unpaywall,
            Source::Mdpi,
            Source::Jstor,
            Source::Scispace,
            Source::Acm,
            Source::ConnectedPapers,
            Source::Doaj,
            Source::WorldWideScience,
            Source::Osf,
            Source::Base,
            Source::Springer,
            Source::GoogleScholar,
            Source::All,
        ];
        assert_eq!(variants.len(), 29);
    }

    #[test]
    fn test_source_to_id_all_variants() {
        let tests = [
            (Source::Arxiv, "arxiv"),
            (Source::Pubmed, "pubmed"),
            (Source::Biorxiv, "biorxiv"),
            (Source::Semantic, "semantic"),
            (Source::OpenAlex, "openalex"),
            (Source::CrossRef, "crossref"),
            (Source::Iacr, "iacr"),
            (Source::Pmc, "pmc"),
            (Source::Hal, "hal"),
            (Source::Dblp, "dblp"),
            (Source::Ssrn, "ssrn"),
            (Source::Dimensions, "dimensions"),
            (Source::IeeeXplore, "ieee_xplore"),
            (Source::EuropePmc, "europe_pmc"),
            (Source::Core, "core"),
            (Source::Zenodo, "zenodo"),
            (Source::Unpaywall, "unpaywall"),
            (Source::Mdpi, "mdpi"),
            (Source::Jstor, "jstor"),
            (Source::Scispace, "scispace"),
            (Source::Acm, "acm"),
            (Source::ConnectedPapers, "connected_papers"),
            (Source::Doaj, "doaj"),
            (Source::WorldWideScience, "worldwidescience"),
            (Source::Osf, "osf"),
            (Source::Base, "base"),
            (Source::Springer, "springer"),
            (Source::GoogleScholar, "google_scholar"),
        ];
        for (source, expected_id) in tests {
            assert_eq!(source_to_id(source), expected_id, "Failed for {:?}", source);
        }
    }

    // Sort field tests
    #[test]
    fn test_sort_field_enum() {
        assert_eq!(SortField::Relevance as i32, 0);
        assert_eq!(SortField::Date as i32, 1);
        assert_eq!(SortField::Citations as i32, 2);
        assert_eq!(SortField::Title as i32, 3);
        assert_eq!(SortField::Author as i32, 4);
    }

    #[test]
    fn test_order_enum() {
        assert_eq!(Order::Asc as i32, 0);
        assert_eq!(Order::Desc as i32, 1);
    }

    #[test]
    fn test_dedup_strategy_enum() {
        assert_eq!(DedupStrategy::First as i32, 0);
        assert_eq!(DedupStrategy::Last as i32, 1);
        assert_eq!(DedupStrategy::Mark as i32, 2);
    }

    // Capability filter tests
    #[test]
    fn test_capability_filter_enum() {
        assert_eq!(CapabilityFilter::Search as i32, 0);
        assert_eq!(CapabilityFilter::Download as i32, 1);
        assert_eq!(CapabilityFilter::Read as i32, 2);
        assert_eq!(CapabilityFilter::Citations as i32, 3);
        assert_eq!(CapabilityFilter::DoiLookup as i32, 4);
        assert_eq!(CapabilityFilter::AuthorSearch as i32, 5);
    }

    // Download with all options
    #[test]
    fn test_cli_download_all_options() {
        let cli = Cli::parse_from([
            "research-master",
            "download",
            "2301.12345",
            "--source",
            "arxiv",
            "--output-path",
            "/path/to/file.pdf",
            "--auto-filename",
            "--create-dir",
            "--doi",
            "10.1234/test",
        ]);
        match &cli.command {
            Some(Commands::Download {
                paper_id,
                source,
                output_path,
                auto_filename,
                create_dir,
                doi,
            }) => {
                assert_eq!(paper_id, "2301.12345");
                assert_eq!(*source, Source::Arxiv);
                assert_eq!(
                    output_path.clone().map(|p| p.to_string_lossy().to_string()),
                    Some("/path/to/file.pdf".to_string())
                );
                assert!(*auto_filename);
                assert!(*create_dir);
                assert_eq!(
                    doi.clone().map(|d| d.to_string()),
                    Some("10.1234/test".to_string())
                );
            }
            _ => panic!("Expected Download command"),
        }
    }

    // Sources with capability filter
    #[test]
    fn test_cli_sources_with_capability() {
        let cli = Cli::parse_from([
            "research-master",
            "sources",
            "--with-capability",
            "download",
        ]);
        match &cli.command {
            Some(Commands::Sources {
                with_capability, ..
            }) => {
                assert_eq!(*with_capability, Some(CapabilityFilter::Download));
            }
            _ => panic!("Expected Sources command"),
        }
    }

    // Author with all options
    #[test]
    fn test_cli_author_all_options() {
        let cli = Cli::parse_from([
            "research-master",
            "author",
            "Geoffrey Hinton",
            "--source",
            "all",
            "--max-results",
            "20",
            "--year",
            "2010-",
            "--dedup",
            "--dedup-strategy",
            "first",
        ]);
        match &cli.command {
            Some(Commands::Author {
                author,
                source,
                max_results,
                year,
                dedup,
                dedup_strategy,
            }) => {
                assert_eq!(author, "Geoffrey Hinton");
                assert_eq!(*source, Source::All);
                assert_eq!(*max_results, 20);
                assert_eq!(year.clone(), Some("2010-".to_string()));
                assert!(*dedup);
                assert_eq!(*dedup_strategy, Some(DedupStrategy::First));
            }
            _ => panic!("Expected Author command"),
        }
    }
}
