# Configuration

Research Master MCP supports configuration via environment variables and/or a configuration file. Environment variables take precedence over file-based settings.

## Configuration File

The application searches for a configuration file in the following locations (in order):

| Priority | Location | Platform |
|----------|----------|----------|
| 1 | `./research-master.toml` | All |
| 2 | `./.research-master.toml` | All |
| 3 | `$XDG_CONFIG_HOME/research-master/config.toml` | Linux/BSD (if XDG_CONFIG_HOME set) |
| 4 | `~/Library/Application Support/research-master/config.toml` | macOS |
| 5 | `~/.config/research-master/config.toml` | Linux/BSD |
| 6 | `%APPDATA%\research-master\config.toml` | Windows |

You can also specify a custom config file path using the `--config` CLI option:

```bash
research-master --config /path/to/config.toml search "machine learning"
```

## Configuration File Format (TOML)

```toml
# Download settings
[downloads]
default_path = "./downloads"
organize_by_source = true
max_file_size_mb = 100

# Rate limiting settings
[rate_limits]
default_requests_per_second = 5
max_concurrent_requests = 10

# API Keys
[api_keys]
semantic_scholar = "your-semantic-scholar-api-key"
core = "your-core-api-key"
openalex_email = "your@email.com"

# Source filtering (same as environment variables)
[sources]
# Sources to enable (if set, only these are used)
enabled_sources = "arxiv,pubmed,semantic"

# Sources to always disable (takes precedence over enabled_sources)
disabled_sources = "dblp,jstor"

# Sources to disable by default (slow/high-latency sources)
# Set to empty string "" to enable all sources by default
default_disabled_sources = "biorxiv,pmc,pubmed"
```

## Environment Variables

All settings can be overridden using environment variables with the `RESEARCH_MASTER_` prefix.

### Source Filtering

| Variable | Description | Default |
|----------|-------------|---------|
| `RESEARCH_MASTER_ENABLED_SOURCES` | Comma-separated list of sources to enable | (not set) |
| `RESEARCH_MASTER_DISABLED_SOURCES` | Comma-separated list of sources to always disable | (none) |
| `RESEARCH_MASTER_DEFAULT_DISABLED_SOURCES` | Sources disabled by default (slow/high-latency) | `biorxiv,pmc,pubmed` |

**Slow Sources (disabled by default):**
The following sources are disabled by default due to high latency (3-15 seconds per request):
- `biorxiv` - ~15 seconds
- `pmc` - ~5 seconds
- `pubmed` - ~3 seconds

**Priority Logic:**
1. `DISABLED_SOURCES` always takes precedence - those sources are never used
2. If `ENABLED_SOURCES` is set: only those sources are used (unless also in `DISABLED_SOURCES`)
3. If `ENABLED_SOURCES` is not set: all sources except those in `DEFAULT_DISABLED_SOURCES` are used
4. `DEFAULT_DISABLED_SOURCES` only applies when `ENABLED_SOURCES` is not set

**Examples:**
```bash
# Default behavior - slow sources disabled automatically
# (no action needed)

# Enable slow sources explicitly
export RESEARCH_MASTER_ENABLED_SOURCES="arxiv,semantic,openalex,biorxiv,pmc,pubmed"

# Disable a fast source you don't need
export RESEARCH_MASTER_DISABLED_SOURCES="jstor,dblp"

# Use specific sources only
export RESEARCH_MASTER_ENABLED_SOURCES="arxiv,semantic"

# Enable ALL sources (disable default behavior)
export RESEARCH_MASTER_DEFAULT_DISABLED_SOURCES=""

# Combine: specific sources plus slow ones
export RESEARCH_MASTER_ENABLED_SOURCES="arxiv,semantic,openalex"
export RESEARCH_MASTER_DEFAULT_DISABLED_SOURCES="biorxiv,pmc,pubmed"
```

**Available source IDs:**
`arxiv`, `pubmed`, `biorxiv`, `semantic`, `openalex`, `crossref`, `iacr`, `pmc`, `hal`, `dblp`, `ssrn`, `core`, `europe_pmc`, `dimensions`, `ieee_xplore`, `zenodo`, `unpaywall`, `mdpi`, `jstor`, `scispace`, `acm`, `connected_papers`, `doaj`, `worldwidescience`, `osf`, `base`, `springer`, `google_scholar`

### API Keys (Optional)

| Variable | Description |
|----------|-------------|
| `SEMANTIC_SCHOLAR_API_KEY` | API key for Semantic Scholar (higher rate limits) |
| `CORE_API_KEY` | API key for CORE service |
| `OPENALEX_EMAIL` | Email identifying the caller to OpenAlex (metadata only; the polite pool was retired) |
| `OPENALEX_API_KEY` | Free API key for OpenAlex's own daily budget; keyless list/search calls share a small per-IP allowance. Create at openalex.org/settings/api |
| `IEEEXPLORE_API_KEY` | API key for IEEE Xplore |
| `JSTOR_API_KEY` | API key for JSTOR |
| `ACM_API_KEY` | API key for ACM Digital Library |
| `SPRINGER_API_KEY` | API key for Springer |

**Note:** Sources work without API keys but may have lower rate limits. If a source requires an API key that isn't provided, it will be automatically disabled during initialization.

### Source-Specific Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `GOOGLE_SCHOLAR_ENABLED` | Enable Google Scholar (requires compile-time feature) | `false` |

**Note:** Google Scholar is disabled by default both at compile-time and runtime. To enable it:
1. Build with `--features google_scholar`
2. Set `GOOGLE_SCHOLAR_ENABLED=true` at runtime

### Source-Specific Rate Limits

| Variable | Description | Default |
|----------|-------------|---------|
| `SEMANTIC_SCHOLAR_RATE_LIMIT` | Semantic Scholar requests per second | `1` |
| `IEEEXPLORE_RATE_LIMIT` | IEEE Xplore requests per second | `3` |
| `ACM_RATE_LIMIT` | ACM Digital Library requests per second | `3` |

**Note:** Without an API key, Semantic Scholar limits you to 1 request per second. Set to a higher value if you have an API key.

### Global Rate Limiting

| Variable | Description | Default |
|----------|-------------|---------|
| `RESEARCH_MASTER_RATE_LIMITS_DEFAULT_REQUESTS_PER_SECOND` | Global requests per second for all HTTP requests | `5` |
| `RESEARCH_MASTER_RATE_LIMITS_MAX_CONCURRENT_REQUESTS` | Maximum concurrent requests | `10` |

**Disable rate limiting entirely:**
```bash
export RESEARCH_MASTER_RATE_LIMITS_DEFAULT_REQUESTS_PER_SECOND=0
```

### Download Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `RESEARCH_MASTER_DOWNLOADS_DEFAULT_PATH` | Default directory for PDF downloads | `./downloads` |
| `RESEARCH_MASTER_DOWNLOADS_ORGANIZE_BY_SOURCE` | Create subdirectories per source | `true` |
| `RESEARCH_MASTER_DOWNLOADS_MAX_FILE_SIZE_MB` | Maximum file size for downloads (MB) | `100` |

### Logging

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Logging level (e.g., `debug`, `info`, `warn`, `error`) |

## Configuration Precedence

Configuration values are loaded in this order (later values override earlier ones):

1. Configuration file in default search locations
2. Configuration file specified via `--config` option
3. Environment variables (always take final precedence)

## Related Documentation

- [Sources](sources.md) - Source-specific configuration and API keys
- [Installation](installation.md) - Installing with custom features
- [Usage](usage.md) - CLI options
- [MCP Clients](mcp-clients.md) - Client configuration with environment variables
