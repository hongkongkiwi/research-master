# Research Master

A Model Context Protocol (MCP) server for searching and downloading academic papers from multiple research sources.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org/)
[![Crates.io](https://img.shields.io/crates/v/research-master)](https://crates.io/crates/research-master)
[![GitHub Release](https://img.shields.io/github/v/release/hongkongkiwi/research-master)](https://github.com/hongkongkiwi/research-master/releases)

## Overview

Research Master gives AI assistants unified access to **36 research sources** (32 enabled by default) through the Model Context Protocol (MCP). It works with Claude Desktop, Zed, Cursor, and other MCP clients for literature search, paper discovery, download, and citation analysis.

## Quick Start

### 1. Install

**macOS (Homebrew):**
```bash
brew tap hongkongkiwi/research-master
brew install research-master
```

**Other methods:** See [Installation](docs/installation.md) for Linux packages, Docker, and building from source.

### 2. Configure Your MCP Client

Add to your Claude Desktop configuration (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "research-master": {
      "command": "research-master",
      "args": ["mcp"]
    }
  }
}
```

See [MCP Clients](docs/mcp-clients.md) for setup instructions for Claude Desktop, Zed, Cursor, Continue, and 15+ other clients.

### 3. Start Searching

Once configured, use natural language with your AI assistant:

```
Search for papers about "transformer architecture" from 2020 onwards
Download the paper 1706.03762 and find what papers cite it
Find papers by Geoffrey Hinton on deep learning
```

See [Tools](docs/tools.md) for all available MCP tools.

## Features

- **36 research sources** (32 by default): arXiv, Semantic Scholar, OpenAlex, PubMed, ClinicalTrials.gov, PMC, bioRxiv, DataCite, INSPIRE-HEP, NASA ADS, and [more](docs/sources.md). SciSpace, Connected Papers, WorldWideScience (retired public endpoints) and Google Scholar are opt-in via `--features`
- **Unified Search**: One query fans out across all enabled sources, with results deduplicated across sources
- **Smart Source Detection**: Automatically identifies paper IDs (arXiv, PMC, DOI, HAL, IACR, Zenodo, etc.)
- **PDF Download and Text Extraction**: Save papers locally and extract full text (poppler, with lopdf fallback)
- **Citation Analysis and Evidence**: Forward citations, references, related papers, citation graphs, Scite-style tallies and retraction flags
- **Bounded Reading**: Section outlines and per-section reads, plus arXiv LaTeX source, that stay within LLM context limits
- **Local Library and Watches**: Save papers to collections, search them, and poll arXiv for new papers on saved topics
- **General Science Workflows**: Systematic-review screening with PRISMA counts, retraction flags in the library and watch alerts, Zotero sync, and ClinicalTrials.gov study search
- **CSL Styles**: Format citations in thousands of archived CSL styles via hayagriva
- **License Filter**: `--license open` keeps only works with a recognized open reuse license
- **Export and Citation Formatting**: BibTeX, CSV, JSON, RIS export; APA, MLA, Chicago, BibTeX cite styles
- **Local History Panel**: Browse recent searches and downloads in a local web UI (`research-master panel`)
- **Honest Failures**: A source missing its API key returns an error naming the key and signup URL instead of empty results
- **Rate Limiting and Caching**: Per-source configurable rate limits, local response cache, and a `doctor` health check

See [Sources](docs/sources.md) for supported databases, API requirements, and rate limits. [Free API and license notes](docs/free-apis.md) cover DataCite, INSPIRE-HEP, NASA ADS, work licenses, and the local history panel.

## Documentation

| Topic | Description |
|-------|-------------|
| [Installation](docs/installation.md) | Install via Homebrew, Docker, packages, or source |
| [Sources](docs/sources.md) | Supported research databases, API keys, rate limits |
| [Usage](docs/usage.md) | CLI commands and options |
| [Tools](docs/tools.md) | Available MCP tools reference |
| [MCP Clients](docs/mcp-clients.md) | Configuration for Claude Desktop, Zed, Cursor, etc. |
| [Configuration](docs/configuration.md) | Environment variables and config file |
| [Development](docs/development.md) | Project structure, adding new sources |

## Common Commands

```bash
# Search for papers
research-master search "transformer architecture" --year 2020-

# Search by author
research-master author "Geoffrey Hinton"

# Download a paper
research-master download 2301.12345 --source arxiv --output-path ./papers

# Look up by DOI
research-master lookup-by-doi 10.48550/arXiv.2301.12345

# Format a citation
research-master cite 10.48550/arXiv.1706.03762 --style bibtex

# Check configuration and source health
research-master doctor

# Start MCP server
research-master mcp --stdio

# Start MCP server over HTTP: --transport sse (legacy /sse + /messages)
# or --transport http (streamable HTTP)
research-master mcp --transport sse --port 3000

# Browse recent searches and downloads (loopback only; --no-web disables)
research-master panel --host 127.0.0.1 --port 3001

# Restrict results to known open reuse licenses
research-master search "transformer" --license open

# Show all environment variables
research-master --env
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

### Ways to Contribute

- Report bugs and issues
- Suggest new features
- Add new research sources
- Improve documentation
- Submit pull requests

See [Development](docs/development.md) for the project structure and how to add new sources.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Built with [Rust](https://www.rust-lang.org/)
- Implements the [Model Context Protocol](https://modelcontextprotocol.io/)
- Integrates with numerous academic research APIs and services

## Contact

- GitHub: [@hongkongkiwi](https://github.com/hongkongkiwi)
- Repository: [https://github.com/hongkongkiwi/research-master](https://github.com/hongkongkiwi/research-master)
