# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- ClinicalTrials.gov source (36th source, 32nd in the default build): study search, NCT id lookup and auto-detection; an explicit DOI query returns the first study citing the DOI and is excluded from cross-source `lookup_by_doi`
- Zotero sync: `zotero_save` resolves a paper into your Zotero library and `zotero_search` keyword-searches it (`ZOTERO_API_KEY` required, plus `ZOTERO_USER_ID` or `ZOTERO_GROUP_ID`; `RESEARCH_MASTER_ZOTERO_API_URL` overrides the API base)
- Systematic-review screening: `screening_start`, `screening_next`, `screening_decide` and `screening_summary` tools with PRISMA counting, plus the `systematic_review_screening` prompt; statistics extracted from abstracts (effect sizes, odds/risk/hazard ratios, CIs, p-values, sample sizes) appear in `screening_next`
- Retraction flags: `library_add` records OpenAlex retraction status for DOI-bearing papers, `library_search` gained `retracted_only`, and watch alerts annotate retracted new hits
- `web_extract` MCP tool: fetch a URL and return readable text (HTML reduced to title plus block text; PDF URLs run through the PDF extractor)
- `github_search` MCP tool: find repositories with a paper's code, datasets, and tools (`GITHUB_API_KEY` optional; a rejected key falls back to anonymous search)
- `web_search` multi-provider fallback: tries Exa, then Tavily (`TAVILY_API_KEY`), then Brave (`BRAVE_API_KEY`) using the first configured key, and falls through on provider failure
- Selectable MCP transports: `research-master mcp --transport stdio|sse|http` (stdio default; `sse` serves the legacy HTTP+SSE protocol with `GET /sse` and `POST /messages?sessionId=`; `http` is streamable HTTP; `--http`/`--stdio` keep working)
- `--no-web` global flag disables the local history panel
- DataCite, INSPIRE-HEP and NASA ADS sources
- License filter: `--license open|any|ignore` on CLI search and a `license` parameter on MCP `search_papers`; `open` keeps only works with a recognized open reuse license
- Local history panel: `research-master panel` serves recent searches and downloads as a web UI on loopback (default port 3001)
- `OPENALEX_API_KEY` support; OpenAlex meters authenticated search endpoints

### Changed
- MCP tools: 42 (up from 36)
- `search_papers` type filtering: `include_types`/`exclude_types`/`strict_types` plus a generic `filters` object; PubMed maps recognized tokens to `[pt]` tags, accepts MeSH clauses via `filters.mesh`, and reports `paper_type` from publication types, OSF accepts a hosted-provider `filters.provider` and reports all results as `preprint`, and OpenAlex reports types verbatim with venue metadata
- `RESEARCH_MASTER_PROXY` sets one proxy URL for both schemes when `HTTP_PROXY`/`HTTPS_PROXY` are unset; socks5/socks5h URLs are supported and the `web_search`, `web_extract` and `github_search` clients honor proxy configuration
- SciSpace, Connected Papers and WorldWideScience removed from default features; their public endpoints no longer resolve (verified 2026-11-02). Still buildable with `--features scispace,connected_papers,worldwidescience`
- Sources with a missing API key now return an error naming the key and signup URL instead of silently returning empty results; network failures and blocks propagate instead of masking as zero hits
- MCP `search_papers` deduplicates results across sources
- Google Scholar capabilities reduced to search only (no longer claims DOI lookup)

### Fixed
- OpenAIRE: every result parsed with an empty title, abstract, authors and DOI — the structs modeled the wrong shapes (title/description/creator/webresource-url are objects or object arrays with the payload under `$`, and numeric `$` values like pmids exist), and serde's `visit_seq` fallback let an untagged `One(T)` variant greedily match a result array as a one-element sequence, silently defaulting the record. Variants reordered (`Many` before `One`), shapes modeled from a live capture, and a guard refuses DOI lookups whose parsed record carries no DOI pid
- PubMed: search silently returned nothing for every query — the efetch XML parser never matched attributes (quick-xml requires `@Name` renames) and inline markup like `<italic>` split `$text` into duplicate events; year filters now use `mindate`/`maxdate` bounds (the old `reldate` values were invalid day counts) and the author filter is ANDed into the single `term` parameter instead of emitting a duplicate one
- OSF: search silently returned nothing because the API rejects literal `[`/`]` in query URLs (now percent-encoded); DOI lookup was removed from the cross-source fan-out and returns an honest error (the API has no filterable `doi` field)
- ClinicalTrials.gov: `lookup_by_doi` fan-out exclusion (a citing study is not the DOI's paper)
- Self-update and `doctor --check-connectivity` requests now honor `RESEARCH_MASTER_PROXY` like every lookup client
- The stdio MCP server now exits after the client closes stdin (pmcp's main loop parks forever after transport close); a short grace period lets the final response flush
- Screening statistics extraction recognizes U+2212 minus signs
- Dimensions: user query is properly JSON-escaped into the GraphQL payload; total result count no longer reports the capped page size
- Expanded unit and integration test coverage; codebase cleanup pass

Versions 0.1.10 through 0.1.39 were not tracked in this file (mostly release packaging and CI workflow fixes).

## [0.1.42] - 2026-01-26

### Added
- 9 new MCP tools: `get_related_papers`, `list_sources`, `author_profile`, `batch_get_papers`, `citation_graph`, `export_papers`, `recommend_papers`, `write_paper`, `web_search` (requires `EXA_API_KEY`)
- ORCID source (author search and record lookup)
- `default_disabled_sources` setting to skip slow sources unless explicitly requested

## [0.1.41] - 2026-01-25

### Added
- `cite` command: format citations in APA, MLA, Chicago, or BibTeX
- HTTP/HTTPS proxy support (`--http-proxy`, `--https-proxy`, `--no-proxy`)
- Configurable user agent (`RESEARCH_MASTER_USER_AGENT`)

### Changed
- `serve` command renamed to `mcp` (`serve` kept as an alias)
- Modern CLI output: colors, icons, and terminal-aware table formatting

## [0.1.40] - 2026-01-25

### Added
- `completions`, `config`, `export`, `bulk-download` and `api-keys` CLI commands

### Changed
- Renamed crate and binary from `research-master-mcp` to `research-master`

## [0.1.9] - 2026-01-23

### Added
- Windows-specific changes for self-updates

### Fixed
- Windows build compatibility and runtime tool detection

## [0.1.8] - 2026-01-23

### Added
- Windows self-update support with SHA256 verification
- Enhanced pre-commit hook for Rust projects

## [0.1.7] - 2026-01-23

### Added
- Self-update command with SHA256 verification

## [0.1.6] - 2026-01-23

### Added
- Self-update command with SHA256 verification

### Fixed
- Release workflow issues

## [0.1.5] - 2026-01-23

### Fixed
- Release workflow issues

## [0.1.4] - 2026-01-23

### Added
- Self-update command with SHA256 verification

## [0.1.3] - 2026-01-23

### Fixed
- Release workflow issues

## [0.1.2] - 2026-01-23

### Added
- Europe PMC source implementation for searching European PubMed Central
- lopdf fallback for PDF extraction (when poppler unavailable)
- Debian, RedHat, and Alpine package builds to release workflow
- Docker and GitHub Packages publishing to release workflow
- is-terminal crate for terminal detection (replaces atty)
- cargo-husky for Rust-native git hooks

### Fixed
- Replace atty with is-terminal crate
- Code formatting applied to recent changes
- macOS runner updated from deprecated macos-13 to macos-14

## [0.1.0] - 2024-01-XX

### Added
- Initial release
