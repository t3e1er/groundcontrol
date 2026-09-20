---
title: "MCP Client & IDE Integration"
description: "Configuring Cursor, Claude Desktop, Antigravity IDE, Windsurf, Zed, and VS Code."
category: "building"
status: "active"
tags: ["mcp", "cursor", "claude", "antigravity", "windsurf", "zed", "vscode", "configuration", "auth"]
related:
  - "[[docs/architecture/building/index]]"
  - "[[docs/architecture/building/installation]]"
  - "[[docs/concepts/progressive-disclosure/tool-profiles]]"
  - "[[docs/architecture/implementation/mcp-transport]]"
---

# MCP Client & IDE Integration

`ctxvault` communicates natively over standard input/output (stdio JSON-RPC) and Server-Sent Events (HTTP SSE), adhering strictly to the Model Context Protocol specification.

* **Agent Auto-Installer**: [`crates/ctxvault-cli/src/installer/mod.rs`](file:///c:/dev/ctx/ctxvault/crates/ctxvault-cli/src/installer/mod.rs)
* **Stdio Transport**: [`crates/ctxvault-mcp/src/transport/stdio.rs`](file:///c:/dev/ctx/ctxvault/crates/ctxvault-mcp/src/transport/stdio.rs)
* **HTTP SSE Transport**: [`crates/ctxvault-mcp/src/transport/http.rs`](file:///c:/dev/ctx/ctxvault/crates/ctxvault-mcp/src/transport/http.rs)

---

## 1. Automated Setup (`ctxvault install`)

`ctxvault` includes an auto-detection installer that discovers installed coding agents and registers a zero-arg `ctxvault` entry:

```bash
# Standard zero-arg local launcher setup
ctxvault install -y

# Setup with dedicated client API keys (CTXV_API_KEY)
ctxvault install -y --auth
```

This automatically scans and configures:
* Antigravity IDE & Gemini CLI (`mcp_config.json`)
* Cursor (`~/.cursor/mcp.json` or `%USERPROFILE%\.cursor\mcp.json`)
* Claude Desktop (`claude_desktop_config.json`)
* Claude Code (`config.json`)
* Windsurf (`~/.codeium/windsurf/mcp_config.json`)
* VS Code (`settings.json` / cline / roo)
* Zed (`settings.json`)

---

## 2. Zero-Argument Launcher Architecture

Because `ctxvault` automatically mounts cached corpora from `${CTXV_CACHE_DIR}/corpora/` and probes the local working directory for `ctxvault.toml`, IDE configurations no longer require complex or brittle path arguments.

### Standard Zero-Arg Configuration
```json
{
  "mcpServers": {
    "ctxvault": {
      "command": "ctxvault",
      "args": []
    }
  }
}
```

### Authenticated Configuration (`--auth`)
When running `ctxvault install --auth` or when connecting to a remote server with `require_auth = true`:
```json
{
  "mcpServers": {
    "ctxvault": {
      "command": "ctxvault",
      "args": [],
      "env": {
        "CTXV_API_KEY": "ag_sec_908f9a"
      }
    }
  }
}
```

---

## 3. Manual IDE Configurations

### Cursor (`.cursor/mcp.json`)
```json
{
  "mcpServers": {
    "ctxvault": {
      "command": "ctxvault",
      "args": []
    }
  }
}
```

### Claude Desktop (`claude_desktop_config.json`)
Location:
* **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
* **Windows**: `%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "ctxvault": {
      "command": "ctxvault",
      "args": []
    }
  }
}
```

### Antigravity IDE & Gemini CLI (`mcp_config.json`)
```json
{
  "mcpServers": {
    "ctxvault": {
      "command": "ctxvault",
      "args": []
    }
  }
}
```

### Zed Editor (`~/.config/zed/settings.json`)
```json
{
  "context_servers": {
    "ctxvault": {
      "command": "ctxvault",
      "args": []
    }
  }
}
```

---

## 4. Role-Based Profiles (`--profile`)

Control which tools are advertised to your agent:
* `--profile scout`: Exposes only read-only retrieval tools (`search`, `get_snippet`, `read_file`, `list_notes`, `status`). Ideal for lightweight code exploration.
* `--profile analysis`: Adds graph traversal and validation tools (`graph_match`, `graph_communities`, `validate`, `list_templates`).
* `--profile all` (default): Exposes all 17 authoritative tools including mutating writes (`write_note`, `delete_note`, `move_note`, `sync_corpus`).
