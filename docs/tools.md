# Available MCP Tools

Once connected via MCP, the following **18 unified tools** are available. These tools automatically select the best source based on your input, or you can specify a source explicitly.

## Core Research Tools

### search_papers

Search for papers across all available research sources or a specific source.

**Parameters:**
- `query` (required): Search query string
- `source` (optional): Specific source to search (e.g., "arxiv", "semantic", "pubmed")
- `max_results` (optional): Maximum number of results (default: 10)
- `year` (optional): Year filter (e.g., "2020", "2018-2022", "2010-", "-2015")
- `category` (optional): Category/subject filter
- `license` (optional): `any` (default) returns all results; `open` retains works with a recognized open reuse license and excludes works without license metadata; `ignore` skips the license filter. This controls work metadata, not the API provider's access terms. To exclude providers, set `RESEARCH_MASTER_DISABLED_SOURCES` or specify `source`.

Search results are cached per source when `RESEARCH_MASTER_CACHE_ENABLED=true`. The `max_results` value applies per source, not to the merged response.

**Example:**
```json
{
  "query": "transformer architecture",
  "year": "2020-",
  "max_results": 20
}
```

### search_by_author

Search for papers by a specific author across sources that support author search.

**Parameters:**
- `author` (required): Author name
- `source` (optional): Specific source to search
- `max_results` (optional): Maximum results per source (default: 10)

**Supported sources:** semantic, openalex, orcid (sources with author-search capability)

### author_profile

Get an author's publication profile: paper counts, most cited works, and publication history, aggregated across sources.

**Parameters:**
- `author` (required): Author name
- `max_results` (optional): Maximum papers to analyze per source (default: 20)

### get_paper

Get detailed metadata for a specific paper. The source is auto-detected from the paper ID format.

**Parameters:**
- `paper_id` (required): Paper identifier (e.g., "2301.12345", "arXiv:2301.12345", "PMC12345678")
- `source` (optional): Override auto-detection and use specific source

**Auto-detection patterns:**
- `arXiv:xxxx` or `xxxx.xxxx` (numeric) → arXiv
- `PMCxxxxxxx` → PMC
- `10.xxxx/xxxxxx` (DOI) → Source with DOI lookup capability
- `CORX:xxxxx` → CORE
- `hal-xxxxxxx` → HAL
- `xxxx/xxxx` (IACR format) → IACR
- `DOI:10.xxxx` → DOI-based lookup
- etc.

### download_paper

Download a paper PDF to your local filesystem.

**Parameters:**
- `paper_id` (required): Paper identifier
- `source` (optional): Override auto-detection
- `output_path` (optional): Save path (default: ./downloads)
- `auto_filename` (optional): Auto-generate filename from title (default: true)

### read_paper

Extract and return the full text content from a paper PDF.

**Parameters:**
- `paper_id` (required): Paper identifier
- `source` (optional): Override auto-detection

**Note:** Requires poppler to be installed. Returns an error if PDF extraction fails.

## Citation & Reference Tools

### get_citations

Get papers that cite a specific paper. Prefers Semantic Scholar for best results.

**Parameters:**
- `paper_id` (required): Paper identifier
- `source` (optional): Specific source (default: "semantic")
- `max_results` (optional): Maximum results (default: 20)

### get_references

Get papers referenced by a specific paper. Prefers Semantic Scholar.

**Parameters:**
- `paper_id` (required): Paper identifier
- `source` (optional): Specific source (default: "semantic")
- `max_results` (optional): Maximum results (default: 20)

### get_related_papers

Get papers related or similar to a specific paper. The source is auto-detected from the paper ID format.

**Parameters:**
- `paper_id` (required): Paper identifier
- `source` (optional): Override auto-detection and use specific source
- `max_results` (optional): Maximum results (default: 20)

## Lookup Tools

### lookup_by_doi

Look up a paper by its DOI across all sources that support DOI lookup.

**Parameters:**
- `doi` (required): Digital Object Identifier (e.g., "10.48550/arXiv.2301.12345")
- `source` (optional): Specific source to query (default: all)

**Supported sources:** all enabled sources with DOI lookup capability. In the default build: semantic, openalex, crossref, datacite, inspire, ads, citeseerx, dimensions, ieee_xplore, core, zenodo, openaire, jstor, doaj, osf, springer, mdpi, acm, base, unpaywall

## Utility Tools

### deduplicate_papers

Remove duplicate papers from a list using DOI matching and title similarity.

**Parameters:**
- `papers` (required): Array of paper objects
- `strategy` (optional): Deduplication strategy - "first" (keep first), "last" (keep last), or "mark" (add `is_duplicate` flag)

**Deduplication criteria:**
- Exact DOI match
- Title similarity > 0.95 (Jaro-Winkler algorithm)
- Author verification

### list_sources

List all available research sources with their capabilities.

**Parameters:** None

### batch_get_papers

Look up multiple papers by their IDs/DOIs in one call.

**Parameters:**
- `paper_ids` (required): Array of paper identifiers (arXiv IDs, DOIs, PMC IDs, etc.)
- `max_per_source` (optional): Maximum results per paper lookup (default: 3)

### citation_graph

Get both forward citations and backward references for a paper in a single call.

**Parameters:**
- `paper_id` (required): Paper identifier
- `source` (optional): Specific source (default: "semantic")
- `max_results` (optional): Maximum results for both citations and references (default: 20)

### export_papers

Export a list of papers in a citation format.

**Parameters:**
- `papers` (required): Array of paper objects
- `format` (optional): Export format - "bibtex", "apa", "mla", "chicago", "csv", or "json" (default: "bibtex")

### recommend_papers

Get AI-powered paper recommendations similar to a given paper. Tries Semantic Scholar, Connected Papers, then OpenAlex.

**Parameters:**
- `paper_id` (required): Paper identifier to find similar papers for
- `max_results` (optional): Maximum recommendations (default: 20)

### write_paper

Save paper metadata to a local JSON library file for later reference, with optional notes. Papers are appended to `library.json` in the platform data directory (for example `~/Library/Application Support/research-master/library.json` on macOS).

**Parameters:**
- `papers` (required): Array of paper objects to save
- `notes` (optional): Notes to attach to the saved papers

### web_search

Search the general web for information, as a fallback when paper sources don't have enough results. Requires the `EXA_API_KEY` environment variable.

**Parameters:**
- `query` (required): Search query
- `max_results` (optional): Maximum results (default: 5)

The unified tools use intelligent source auto-detection:

| Paper ID Format | Detected Source |
|-----------------|-----------------|
| `arXiv:1234.5678` or `1234.5678` | arXiv |
| `PMC12345678` | PMC |
| `10.xxxx/xxxxxx` | Source with DOI lookup |
| `CORX:xxxxx` | CORE |
| `hal-xxxxxxx` | HAL |
| `xxxx/xxxx` (IACR format) | IACR |
| `doi:10.xxxx` or `https://doi.org/10.xxxx` | DOI lookup |
| `ZENODO:xxxxx` | Zenodo |
| etc. |

You can always override auto-detection by specifying the `source` parameter explicitly.

## Example Usage with Claude

Once configured with Claude Desktop, you can interact with the research sources naturally using the unified tools:

```
User: Search for papers about "transformer architecture" from 2020 onwards
Claude: [Uses search_papers tool with query="transformer architecture", year="2020-"]

User: Find papers by Geoffrey Hinton on deep learning
Claude: [Uses search_by_author tool with author="Geoffrey Hinton"]

User: Download the paper "Attention Is All You Need" and find what papers cite it
Claude: [Uses download_paper with paper_id="1706.03762" (arXiv),
         then get_citations to find citing papers]

User: Read the abstract and introduction from this paper
Claude: [Uses read_paper tool with paper_id="1706.03762" to extract PDF text]

User: Look up this paper by its DOI
Claude: [Uses lookup_by_doi with doi="10.48550/arXiv.1706.03762"]
```

The unified tools automatically detect the appropriate source, so you don't need to remember which source has which paper. You can simply provide the paper ID, DOI, or search query, and the tool handles the rest.

## Reading and Evidence Tools

- `get_paper_outline` / `read_paper_section`: extract a section outline from a paper's PDF text, then read one section at a time (bounded to 12000 characters per call).
- `get_paper_latex`: fetch the LaTeX source of an arXiv paper, the reliable path for equations.
- `get_citation_tallies`: Scite-style counts (supporting / contradicting / mentioning) per DOI, plus OpenAlex retraction status.
- `extract_pdf_references` / `extract_pdf_header`: parse a local PDF's references or header metadata via a self-hosted GROBID service (`GROBID_URL`, default http://localhost:8070).

## Local Library

- `library_add` / `library_list` / `library_remove`: persistent saved papers (JSONL at the user config dir) with optional collections and tags; re-saving within the same collection updates in place.
- `library_search` / `library_ask`: keyword retrieval over saved papers with fuzzy term matching and matched-term reporting. Retrieval only, no generated synthesis.

## Watches

- `watch_topic` / `list_watches` / `check_alerts` / `unwatch_topic`: save topic queries against the arXiv announcement feed and poll for new papers since the last check. State is JSONL at the user config dir.

## Citation Styles and Prompts

- `format_citation_style`: in-text citation in any CSL style from the Zotero archive (e.g. `nature`, `ieee`, `vancouver`), powered by hayagriva.
- Built-in MCP prompts: `literature_review`, `deep_paper_analysis`, `research_discovery`, `citation_audit`. Clients list them via the standard prompts API.

## Env Overrides

- `RESEARCH_MASTER_HTTP_TOKEN`: bearer token required on every request to `mcp --http` when set.
- `RESEARCH_MASTER_LIBRARY_PATH`, `RESEARCH_MASTER_WATCHES_PATH`: directory overrides for the JSONL stores.

## Related Documentation

- [Sources](sources.md) - Supported research sources
- [Free APIs and licenses](free-apis.md) - API access notes and license policy
- [Usage](usage.md) - CLI commands and options
- [MCP Clients](mcp-clients.md) - Client configuration
- [Configuration](configuration.md) - Environment variables
