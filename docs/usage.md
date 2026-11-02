# Usage

This guide covers CLI commands and MCP server usage.

## MCP Server Mode

### STDIO Mode (Recommended for most clients)

For integration with Claude Desktop or other MCP clients:

```bash
research-master mcp --stdio
```

### HTTP/SSE Mode

For clients that support HTTP-based MCP connections:

```bash
research-master mcp --transport sse --port 3000 --host 127.0.0.1
```

Then configure your client to connect to `http://localhost:3000/sse`. Use `--transport http` for streamable HTTP instead. `--host` and `--port` apply to both HTTP transports.

## CLI Commands

### Search Command (`search` or `s`)

Search for papers across all available research sources.

```bash
research-master search "transformer architecture" --year 2020-
```

**Options:**

| Option | Description |
|--------|-------------|
| `query` (required) | Search query string |
| `-s, --source <SOURCE>` | Source to search (default: all) |
| `-m, --max-results <N>` | Maximum results (default: 10) |
| `-y, --year <YEAR>` | Year filter (e.g., "2020", "2018-2022", "2010-", "-2015") |
| `--sort-by <FIELD>` | Sort by: relevance, date, citations, title, author |
| `--order <ORDER>` | Sort order: asc, desc |
| `-c, --category <CAT>` | Category/subject filter |
| `--author <NAME>` | Author name filter |
| `--include-types <TYPES>` | Include comma-separated types; repeatable |
| `--exclude-types <TYPES>` | Exclude comma-separated types; repeatable |
| `--strict-types` | Exclude unclassified papers when using include types |
| `--fulltext` | Search full text where supported |
| `--filter <KEY=VALUE>` | Source-specific filter; repeatable |
| `--dedup` | Deduplicate results |
| `--dedup-strategy <STRAT>` | Deduplication strategy: first, last, mark |
| `--fetch-details` | Fetch detailed information (slower but more complete, default: true) |

### Author Command (`author` or `a`)

Search for papers by a specific author.

```bash
research-master author "Geoffrey Hinton" --source semantic
```

**Options:**

| Option | Description |
|--------|-------------|
| `author` (required) | Author name |
| `-s, --source <SOURCE>` | Source to search (default: all with author search) |
| `-m, --max-results <N>` | Maximum results per source (default: 10) |
| `-y, --year <YEAR>` | Year filter |
| `--dedup` | Deduplicate results |
| `--dedup-strategy <STRAT>` | Deduplication strategy |

**Supported sources:** arxiv, semantic, openalex, pubmed, biorxiv, pmc, hal, iacr, ssrn

### Download Command (`download` or `d`)

Download a paper PDF to your local filesystem.

```bash
research-master download 2301.12345 --source arxiv --output-path ./papers
```

**Options:**

| Option | Description |
|--------|-------------|
| `paper_id` (required) | Paper identifier |
| `-s, --source <SOURCE>` | Paper source (required for CLI) |
| `--output-path <PATH>` | Save path (default: ./downloads) |
| `--auto-filename` | Auto-generate filename from title (default: true) |
| `--create-dir` | Create parent directory if needed |
| `--doi <DOI>` | Paper DOI (optional, for verification) |

### Read Command (`read` or `r`)

Read and extract text from a paper's PDF.

```bash
research-master read 2301.12345 --source arxiv --path ./paper.pdf --output-file extracted.txt
```

**Options:**

| Option | Description |
|--------|-------------|
| `paper_id` (required) | Paper identifier |
| `-s, --source <SOURCE>` | Paper source |
| `-p, --path <PATH>` | Path to PDF or where to download (default: ./downloads) |
| `--download-if-missing` | Download PDF if not found locally (default: true) |
| `--pages <N>` | Number of pages to extract (0 = all) |
| `-O, --output-file <PATH>` | Write extracted text to file |

**Note:** Requires poppler to be installed for PDF text extraction.

### Citations Command (`citations` or `c`)

Get papers that cite a specific paper.

```bash
research-master citations 2301.12345 --source arxiv --max-results 20
```

**Options:**

| Option | Description |
|--------|-------------|
| `paper_id` (required) | Paper identifier |
| `-s, --source <SOURCE>` | Source to search (default: semantic) |
| `-m, --max-results <N>` | Maximum results (default: 20) |

### References Command (`references` or `ref`)

Get papers referenced by a specific paper.

```bash
research-master references 1706.03762 --source semantic
```

**Options:**

| Option | Description |
|--------|-------------|
| `paper_id` (required) | Paper identifier |
| `-s, --source <SOURCE>` | Source to search (default: semantic) |
| `-m, --max-results <N>` | Maximum results (default: 20) |

### Related Command (`related` or `rel`)

Get related/similar papers.

```bash
research-master related 1706.03762 --source connected_papers
```

**Options:**

| Option | Description |
|--------|-------------|
| `paper_id` (required) | Paper identifier |
| `-s, --source <SOURCE>` | Source to search (default: connected_papers) |
| `-m, --max-results <N>` | Maximum results (default: 20) |

### Lookup Command (`lookup-by-doi` or `doi`)

Look up a paper by its DOI.

```bash
research-master lookup-by-doi 10.48550/arXiv.2301.12345
```

**Options:**

| Option | Description |
|--------|-------------|
| `doi` (required) | Digital Object Identifier |
| `-s, --source <SOURCE>` | Source to search (default: all with DOI lookup) |
| `-j, --json` | Output as JSON |

**Supported sources:** semantic, openalex, crossref, hal, doaj, osf, springer, mdpi, acm, base, unpaywall

### Sources Command (`sources` or `ls`)

List available sources and their capabilities.

```bash
research-master sources --detailed
```

**Options:**

| Option | Description |
|--------|-------------|
| `-d, --detailed` | Show detailed information about each source |
| `--with-capability <CAP>` | Filter sources by capability (search, download, read, citations, doi_lookup, author_search) |

### Cache Command (`cache`)

Manage local cache.

```bash
# Show cache status
research-master cache status

# Clear all cached data
research-master cache clear

# Clear only search cache
research-master cache clear-searches

# Clear only citation cache
research-master cache clear-citations
```

### Doctor Command (`doctor` or `diag`)

Check configuration and source health.

```bash
# Basic check
research-master doctor

# Check connectivity to all sources
research-master doctor --check-connectivity

# Check API keys
research-master doctor --check-api-keys

# Verbose output
research-master doctor --check-connectivity --check-api-keys --verbose
```

### Update Command (`update`)

Update to the latest version.

```bash
# Check for updates
research-master update

# Force update even if already at latest
research-master update --force

# Preview what would be updated
research-master update --dry-run
```

### Dedupe Command (`dedupe`)

Remove duplicate papers from a JSON file.

```bash
research-master dedupe papers.json --strategy first --output-file deduplicated.json

# Just show duplicates without removing
research-master dedupe papers.json --show
```

**Options:**

| Option | Description |
|--------|-------------|
| `input` (required) | Input JSON file containing papers |
| `-O, --output-file <PATH>` | Output file (default: overwrite input) |
| `-s, --strategy <STRAT>` | Deduplication strategy: first, last, mark (default: first) |
| `--show` | Show duplicate groups without removing |

### History Command (`history` or `hist`)

Show search and download history.

```bash
# Show recent history (default: 20 entries)
research-master history

# Show only searches
research-master history --searches

# Show only downloads
research-master history --downloads

# Limit to 10 entries
research-master history --limit 10

# Clear history
research-master history --clear
```

**Options:**

| Option | Description |
|--------|-------------|
| `-l, --limit <N>` | Maximum entries to show (default: 20) |
| `--searches` | Show only search history |
| `--downloads` | Show only download history |
| `--clear` | Clear all history |

### Clear Command (`clear`)

Clear cache, history, or downloads.

```bash
# Clear everything
research-master clear --all

# Clear only cache
research-master clear --cache

# Clear only history
research-master clear --history

# Clear only downloads
research-master clear --downloads
```

**Options:**

| Option | Description |
|--------|-------------|
| `--cache` | Clear all cached data |
| `--history` | Clear search and download history |
| `--downloads` | Clear downloaded papers |
| `-a, --all` | Clear everything (cache, history, downloads) |

### Cite Command (`cite`)

Format a paper citation in various styles (APA, MLA, Chicago, BibTeX).

```bash
# Get citation in APA format (default)
research-master cite 10.48550/arXiv.2301.12345

# Get citation in MLA format
research-master cite 2301.12345 --style mla

# Get citation in Chicago format
research-master cite 2301.12345 --style chicago

# Get BibTeX entry
research-master cite 10.1038/nature12373 --style bibtex

# Output as JSON
research-master cite 10.1038/nature12373 --format json

# Specify source explicitly
research-master cite 2301.12345 --source arxiv --style apa
```

**Options:**

| Option | Description |
|--------|-------------|
| `<PAPER_ID>` (required) | Paper ID (DOI, arXiv ID, PMC ID, etc.) |
| `--style <STYLE>` | Citation style: `apa`, `mla`, `chicago`, `bibtex` (default: apa) |
| `--source <SOURCE>` | Source to search (auto-detected if not specified) |
| `--format <FORMAT>` | Output format: `text`, `bibtex`, `json` (default: text) |

## Global Options

| Option | Description |
|--------|-------------|
| `-v, --verbose` | Enable verbose logging (can be repeated: -v, -vv, -vvv) |
| `-q, --quiet` | Suppress non-error output |
| `-o, --output <FORMAT>` | Output format: `auto`, `table`, `json`, `plain` (default: auto) |
| `--config <PATH>` | Path to configuration file |
| `--timeout <SECONDS>` | Request timeout in seconds (default: 30) |
| `--http-proxy <URL>` | HTTP proxy URL (e.g., http://proxy:8080) |
| `--https-proxy <URL>` | HTTPS proxy URL (e.g., https://proxy:8080) |
| `--no-proxy <HOSTS>` | Comma-separated list of hosts to bypass proxy |
| `--env` | Show all environment variables and exit |
| `--no-cache` | Disable caching for this command |

## Proxy Configuration

Proxy settings can be configured via CLI flags, environment variables, or config file:

```bash
# Using CLI flags
research-master search "transformer" --http-proxy http://proxy:8080 --https-proxy https://proxy:8080

# Using environment variables
export HTTP_PROXY=http://proxy:8080
export HTTPS_PROXY=https://proxy:8080
export NO_PROXY=localhost,127.0.0.1
research-master search "transformer"

# Using config file (~/.config/research-master/config.toml)
# [proxy]
# http = "http://proxy:8080"
# https = "https://proxy:8080"
# no_proxy = "localhost,127.0.0.1"
```

CLI flags take precedence over environment variables, which take precedence over config file settings.

## Available Sources

| Source | ID | Search | Download | Read | Citations | DOI Lookup | Author Search |
|--------|-----|--------|----------|------|-----------|------------|---------------|
| arXiv | arxiv | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| PubMed | pubmed | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| bioRxiv | biorxiv | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| Semantic Scholar | semantic | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| OpenAlex | openalex | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| CrossRef | crossref | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ |
| IACR ePrint | iacr | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| PMC | pmc | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| HAL | hal | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| DBLP | dblp | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| SSRN | ssrn | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| Dimensions | dimensions | ✅ | ❌ | ❌ | ✅ | ❌ | ❌ |
| IEEE Xplore | ieee_xplore | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Europe PMC | europe_pmc | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ |
| CORE | core | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| Zenodo | zenodo | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| Unpaywall | unpaywall | ❌ | ✅ | ❌ | ❌ | ✅ | ❌ |
| MDPI | mdpi | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| JSTOR | jstor | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| SciSpace | scispace | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| ACM DL | acm | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| Connected Papers | connected_papers | ✅ | ❌ | ❌ | ✅ | ❌ | ❌ |
| DOAJ | doaj | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| WorldWideScience | worldwidescience | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| OSF Preprints | osf | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| BASE | base | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| Springer | springer | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| Google Scholar | google_scholar | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |

## Output Formats

### Auto Format (Default)

Automatically chooses between table and plain text based on terminal:

```bash
research-master search "transformer" --output auto
```

### Table Format

Tabular output for easy reading:

```bash
research-master search "transformer" --output table
```

### JSON Format

Machine-readable JSON output:

```bash
research-master search "transformer" --output json
```

### Plain Format

Simple line-by-line output:

```bash
research-master search "transformer" --output plain
```

## Quick Examples

```bash
# Search for papers
research-master search "transformer architecture" --year 2020-

# Search by author
research-master author "Geoffrey Hinton"

# Download a paper
research-master download 2301.12345 --source arxiv --output-path ./papers

# Read extracted text from a PDF
research-master read 2301.12345 --source arxiv --path ./paper.pdf --output-file extracted.txt

# Look up by DOI
research-master lookup-by-doi 10.48550/arXiv.2301.12345

# Get citations
research-master citations 2301.12345 --source arxiv

# Get references
research-master references 1706.03762

# Get related papers
research-master related 1706.03762 --source connected_papers

# Format citation in APA style
research-master cite 10.1038/nature12373 --style apa

# Format citation in BibTeX
research-master cite 10.1038/nature12373 --style bibtex

# List all sources with capabilities
research-master sources --detailed --with-capability download

# Check system health
research-master doctor --check-connectivity

# Deduplicate papers
research-master dedupe papers.json --strategy first

# Show search/download history
research-master history

# Clear cache, history, and downloads
research-master clear --all

# Show all environment variables
research-master --env

# Start MCP server
research-master mcp --stdio
```

## Next Steps

- [Set up your MCP client](mcp-clients.md) for integration with AI assistants
- [Configure sources](sources.md) with API keys for better rate limits
- [Configure environment variables](configuration.md)

## Related Documentation

- [Sources](sources.md) - Supported research sources
- [MCP Clients](mcp-clients.md) - Configuration for MCP clients
- [Configuration](configuration.md) - Environment variables and config file
- [Tools](tools.md) - Available MCP tools reference
