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

`groundcontrol` communicates natively over standard input/output (stdio JSON-RPC) and Server-Sent Events (HTTP SSE), adhering strictly to the Model Context Protocol specification.

* **Agent Auto-Installer**: [`crates/groundcontrol-cli/src/installer/mod.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-cli/src/installer/mod.rs)
* **Stdio Transport**: [`crates/groundcontrol-mcp/src/transport/stdio.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/transport/stdio.rs)
* **HTTP SSE Transport**: [`crates/groundcontrol-mcp/src/transport/http.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/transport/http.rs)

---

## 1. Automated Setup (`groundcontrol install`)

`groundcontrol` includes an auto-detection installer that discovers installed coding agents and registers a zero-arg `groundcontrol` entry:

```bash
# Standard zero-arg local launcher setup
groundcontrol install -y

# Setup with dedicated client API keys (GROUNDCONTROL_API_KEY)
groundcontrol install -y --auth
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

Because `groundcontrol` automatically mounts cached corpora from `${GROUNDCONTROL_CACHE_DIR}/corpora/` and probes the local working directory for `groundcontrol.toml`, IDE configurations no longer require complex or brittle path arguments.

### Standard Zero-Arg Configuration
```json
{
  "mcpServers": {
    "groundcontrol": {
      "command": "groundcontrol",
      "args": []
    }
  }
}
```

### Authenticated Configuration (`--auth`)
When running `groundcontrol install --auth` or when connecting to a remote server with `require_auth = true`:
```json
{
  "mcpServers": {
    "groundcontrol": {
      "command": "groundcontrol",
      "args": [],
      "env": {
        "GROUNDCONTROL_API_KEY": "ag_sec_908f9a"
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
    "groundcontrol": {
      "command": "groundcontrol",
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
    "groundcontrol": {
      "command": "groundcontrol",
      "args": []
    }
  }
}
```

### Antigravity IDE & Gemini CLI (`mcp_config.json`)
```json
{
  "mcpServers": {
    "groundcontrol": {
      "command": "groundcontrol",
      "args": []
    }
  }
}
```

### Zed Editor (`~/.config/zed/settings.json`)
```json
{
  "context_servers": {
    "groundcontrol": {
      "command": "groundcontrol",
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
