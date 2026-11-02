# MCP Client Integration

Research Master is compatible with any Model Context Protocol client. Below are setup instructions for popular MCP-compatible applications.

## Claude Desktop

**macOS:** `~/Library/Application Support/Claude/claude_desktop_config.json`

**Windows:** `%APPDATA%/Claude/claude_desktop_config.json`

**Linux:** `~/.config/Claude/claude_desktop_config.json`

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

## Zed Editor

Add to **project settings** (`.zed/settings.json`) or **global settings** (`~/.config/zed/settings.json`):

```json
{
  "model_context_provider": {
    "servers": {
      "research-master": {
        "command": "research-master",
        "args": ["mcp"]
      }
    }
  }
}
```

For per-project configuration, create `.zed/settings.json` in your project root.

## Continue (VS Code / JetBrains)

Add to **Continue config** (`~/.continue/config.json` or project `.continue/config.json`):

```json
{
  "models": [
    {
      "name": "claude",
      "provider": "anthropic"
    }
  ],
  "mcpServers": {
    "research-master": {
      "command": "research-master",
      "args": ["mcp"]
    }
  }
}
```

The `~` prefix expands to your home directory. Config file location:
- **VS Code:** `~/.continue/config.json`
- **JetBrains:** `~/.continue/config.json` or project `.continue/config.json`

## Cursor

Cursor is compatible with Claude Desktop config. Edit `~/.cursor/mcp.json`:

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

Or use **Settings > Features > MCP** to configure via UI.

## Goose

Goose uses the same config format as Claude Desktop. Edit `~/.config/goose/mcp_config.json`:

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

Ensure the binary is in your PATH or use absolute path:
```json
{
  "mcpServers": {
    "research-master": {
      "command": "/usr/local/bin/research-master",
      "args": ["mcp"]
    }
  }
}
```

## Tabby

Tabby supports MCP servers via its configuration. Edit `~/.tabby/mcp/servers.json`:

```json
{
  "servers": {
    "research-master": {
      "command": "research-master",
      "args": ["mcp"]
    }
  }
}
```

Restart Tabby after configuration changes.

## CLI with MCP Proxy

Use with any MCP proxy tool (e.g., `mcp-cli`, `glama-cli`):

```bash
# Install proxy
pip install mcp-cli

# Run with proxy
mcp-cli run --command "research-master" --args "mcp"
```

Or use with Smithery for easy MCP server discovery:

```bash
# Install Smithery CLI
npm install -g @smithery/cli

# Add Research Master
smithery add research-master
```

## Homebrew Cask (macOS)

If installed via Homebrew cask, the binary is already in your PATH:

```bash
# Using Homebrew to install
brew tap hongkongkiwi/research-master
brew install --cask research-master

# Verify installation
research-master --version
```

The MCP server command in configs can simply be `"research-master"`.

## Docker

Run via Docker for isolated execution:

```bash
# Build image
docker build -t research-master .

# Run with stdio mode
docker run --rm -i research-master mcp --stdio

# Or use pre-built image (includes Poppler for PDF text extraction)
docker run --rm -i ghcr.io/hongkongkiwi/research-master mcp --stdio

# OCR variant (adds Tesseract for scanned PDFs)
docker run --rm -i ghcr.io/hongkongkiwi/research-master-ocr mcp --stdio

# Build OCR image with extra languages (e.g., English + German)
docker build -f Dockerfile.ocr -t research-master-ocr --build-arg OCR_LANGS="eng deu" .
```

For persistent configuration, mount volumes:
```bash
docker run --rm -i \
  -v ~/.config/research-master:/root/.config/research-master \
  -v ./downloads:/downloads \
  ghcr.io/hongkongkiwi/research-master mcp --stdio
```

## Cline (VS Code / JetBrains)

Cline supports MCP servers via `~/.cline/mcp_servers.json` or project `.cline/mcp_servers.json`:

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

**Or use absolute path:**

```json
{
  "mcpServers": {
    "research-master": {
      "command": "/usr/local/bin/research-master",
      "args": ["mcp"]
    }
  }
}
```

## Roo Code

Roo Code (formerly Rui) uses the same MCP config format as Claude Desktop. Edit `~/.config/roo/mcp_config.json`:

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

**Or use Settings UI:**
- Open Roo Code Settings
- Navigate to **Extensions > MCP**
- Add server configuration manually

**With environment variables:**

```json
{
  "mcpServers": {
    "research-master": {
      "command": "research-master",
      "args": ["mcp"],
      "env": {
        "RESEARCH_MASTER_ENABLED_SOURCES": "arxiv,semantic"
      }
    }
  }
}
```

## Kilo Code

Kilo Code supports MCP in `~/.config/kilo/mcp.json`:

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

**Or per-project:** Create `.kilo/mcp.json` in your project root.

## VS Code (Direct MCP)

VS Code requires the **MCP for VS Code** extension or use with Continue extension (see above).

**Using MCP for VS Code extension:**
1. Install "MCP" extension from marketplace
2. Open Settings (Ctrl+,)
3. Search for "MCP Servers"
4. Add configuration:

```json
{
  "mcp.servers": {
    "research-master": {
      "command": "research-master",
      "args": ["mcp"]
    }
  }
}
```

## 1MCP

1MCP is an MCP proxy/aggregator. Configure in `~/.config/1mcp/servers.json`:

```json
{
  "servers": {
    "research-master": {
      "command": "research-master",
      "args": ["mcp"]
    }
  }
}
```

Run 1MCP with your preferred client:

```bash
# Start 1MCP proxy
1mcp serve --port 3000

# Or with custom config
1mcp serve --config ~/.config/1mcp/config.json
```

## OpenAI Codex CLI

Codex CLI uses MCP configuration via `~/.config/codex/mcp.json`:

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

**Or use environment variable:**

```bash
export MCP_SERVERS='{"research-master": {"command": "research-master", "args": ["mcp"]}}'
```

## Gemini CLI

Gemini CLI supports MCP via `~/.config/gemini/mcp.json`:

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

**With API keys in environment:**

```json
{
  "mcpServers": {
    "research-master": {
      "command": "research-master",
      "args": ["mcp"],
      "env": {
        "SEMANTIC_SCHOLAR_API_KEY": "${SEMANTIC_SCHOLAR_API_KEY}",
        "RESEARCH_MASTER_RATE_LIMITS_DEFAULT_REQUESTS_PER_SECOND": "10"
      }
    }
  }
}
```

## OpenCode

OpenCode supports MCP servers. Configure via **Settings > MCP**:

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

**Or via config file:** `~/.opencode/mcp.json`

```json
{
  "servers": {
    "research-master": {
      "type": "stdio",
      "command": "research-master",
      "args": ["mcp"]
    }
  }
}
```

## Other MCP Clients

Research Master works with any MCP-compatible client. General configuration pattern:

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

**Key clients known to work:**
- **Aider**: `~/.aider.mcp.json`
- **CopilotKit**: Uses environment variables
- **AgentOps**: Configure via dashboard
- **LangChain agents**: Pass via MCPConfig

For HTTP/SSE mode (alternative to stdio):

```bash
research-master mcp --port 3000 --host 0.0.0.0
```

Then configure with HTTP endpoint:
```json
{
  "mcpServers": {
    "research-master": {
      "url": "http://localhost:3000/sse"
    }
  }
}
```

## Common Configuration Options

| Option | Description |
|--------|-------------|
| `command` | Binary name or full path to `research-master` |
| `args` | `["mcp"]` for stdio mode, `["mcp", "--port", "3000"]` for SSE |
| `env` | Optional environment variables (API keys, rate limits) |

**Example with environment variables:**

```json
{
  "mcpServers": {
    "research-master": {
      "command": "research-master",
      "args": ["mcp"],
      "env": {
        "RESEARCH_MASTER_ENABLED_SOURCES": "arxiv,semantic,openalex",
        "RESEARCH_MASTER_RATE_LIMITS_DEFAULT_REQUESTS_PER_SECOND": "10"
      }
    }
  }
}
```

## Related Documentation

- [Installation](installation.md) - Installing Research Master
- [Tools](tools.md) - Available MCP tools reference
- [Usage](usage.md) - CLI commands and options
- [Configuration](configuration.md) - Environment variables
