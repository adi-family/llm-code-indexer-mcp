# ADI Code Indexer - MCP

Model Context Protocol (MCP) server for ADI Code Indexer - enabling AI assistants to search and understand codebases.

## Overview

`adi-mcp` implements the Model Context Protocol, allowing AI assistants like Claude to directly query and explore indexed codebases. It provides semantic code search, symbol lookup, and codebase navigation capabilities.

## Installation

```bash
cargo build --release
# Binary available at: target/release/adi-mcp
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `search` | Semantic search across indexed code |
| `symbols` | List and filter code symbols |
| `files` | Browse indexed files |
| `show` | Get detailed symbol information |
| `tree` | Navigate code structure |

## Usage with Claude

Add to your Claude configuration:

```json
{
  "mcpServers": {
    "adi": {
      "command": "/path/to/adi-mcp",
      "args": ["--project", "/path/to/your/project"]
    }
  }
}
```

## Quick Start

```bash
# Index your project first
adi init && adi index

# Run MCP server (typically launched by AI assistant)
adi-mcp --project /path/to/project
```

## Protocol

Communicates via JSON-RPC over stdio, following the MCP specification.

## License

BSL-1.1 - See [LICENSE](LICENSE) for details.
