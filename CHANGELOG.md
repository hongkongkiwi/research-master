# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
- SciSpace, Connected Papers and WorldWideScience removed from default features; their public endpoints no longer resolve (verified 2026-11-02). Still buildable with `--features scispace,connected_papers,worldwidescience`
- Sources with a missing API key now return an error naming the key and signup URL instead of silently returning empty results; network failures and blocks propagate instead of masking as zero hits
- MCP `search_papers` deduplicates results across sources
- Google Scholar capabilities reduced to search only (no longer claims DOI lookup)

### Fixed
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
