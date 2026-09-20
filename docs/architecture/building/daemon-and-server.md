---
title: "Auto-Daemon & Shared Server Deployment"
description: "Deploying ctxvault as a background daemon, hosting multi-corpus servers, and script automation."
category: "building"
status: "active"
tags: ["daemon", "server", "http", "sse", "multi-corpus", "concurrency", "config"]
related:
  - "[[docs/architecture/building/index]]"
  - "[[docs/architecture/implementation/cross-corpus-federation]]"
  - "[[docs/architecture/implementation/mcp-transport]]"
---

# Auto-Daemon & Shared Server Deployment

`ctxvault` supports multiple operational modes: standard per-agent stdio processes, transparent background daemons, and dedicated multi-corpus HTTP servers.

---

## 1. The Auto-Daemon Pattern

When multiple subagents or editor windows run concurrently on the same machine, spawning multiple independent index engines wastes memory and locks SQLite databases.

The **Auto-Daemon** solves this transparently:
1. When launched with `--daemon`, the process checks if a local daemon is listening on port `9090`.
2. If absent, it detaches a shared background process serving the indexed corpora.
3. The foreground CLI bridges standard stdio JSON-RPC to the daemon over HTTP SSE with zero subagent configuration changes.

```bash
ctxvault --corpus /path/to/project --daemon
```

---

## 2. Dedicated Multi-Corpus Server Mode

For team environments, sandboxed CI agents, or multi-agent swarms, run `ctxvault` as a persistent standalone service hosting $N$ distinct index roots:

```bash
ctxvault --mode server --bind 0.0.70:9090 \
  --corpus docs=/opt/knowledge/docs \
  --corpus backend=/opt/services/backend \
  --corpus frontend=/opt/services/frontend \
  --default-corpus backend \
  --profile all \
  --sync
```

### Endpoints
* `POST /v1/mcp`: Standard MCP JSON-RPC 2.0 request/response handling.
* `GET /v1/sse`: Server-Sent Events stream for asynchronous agent notifications and progress reporting.
* `GET /health`: Health probe returning corpus status, VRAM usage, and active connections.

---

## 3. Central Configuration (`${CTXV_CACHE_DIR}/config.toml`)

All machine-wide settings (daemon port, authentication keys, GraphView telemetry relay, and persistent corpus mounts) are consolidated into a single central configuration file:

```toml
# ==============================================================================
# ctxvault Central Machine Configuration (${CTXV_CACHE_DIR}/config.toml)
# ==============================================================================

[server]
bind = "127.0.0.1:9090"
idle_timeout_mins = 30
log_level = "info"
auto_index = true
index_mode = "full"

[auth]
# Require valid x-api-key on incoming HTTP MCP requests (/v1/mcp, /v1/sse)
require_auth = false
# Shared secret for core daemon-to-graphview telemetry relay
daemon_key = "ctxv_relay_sec_89dfa8"

[[auth.clients]]
id = "antigravity"
name = "Antigravity Agent"
key = "ag_sec_908f9a"
color = "#38bdf8"

[graphview]
bind = "127.0.0.1:9091"
daemon = "http://127.0.0.1:9090"
daemon_key = "ctxv_relay_sec_89dfa8"

[corpora]
default = "ctxvault"

[corpora.ctxvault]
path = "C:/dev/ctx/ctxvault"
index_mode = "full"
```

Configure these settings interactively via the CLI:
```bash
ctxvault config list
ctxvault config get server.bind
ctxvault config set server.idle_timeout_mins 60
ctxvault config set auth.require_auth true
```

---

## 4. Scripted CLI Client Mode

Interact with a running daemon or local engine directly from shell scripts or CI pipelines without an MCP editor:

```bash
# Search using hybrid mode
ctxvault --mode client --server http://127.0.0.1:9090 \
  --call search \
  --args '{"query": "authentication token", "mode": "hybrid", "snippets": 3}'

# Inspect multi-hop graph lineage
ctxvault --mode client --server http://127.0.0.1:9090 \
  --call graph_match \
  --args '{"pattern": "(:CodeSymbol {name: \"verify_jwt\"})-[:calls*1..2]->(target)"}'
```

---

## 5. Direct CLI Indexing & Incremental Sync

In addition to serving MCP connections, the `ctxvault` CLI provides direct subcommands for building and updating central index stores without launching a daemon:

```bash
# Initialize a new repository with ctxvault.toml & gitignore migration
ctxvault init

# Index a repository into central storage (~/.cache/ctxvault/corpora/<name>)
ctxvault index /path/to/project

# Fast indexing (BM25 + Graph only, skip embeddings)
ctxvault index /path/to/project --fast

# Incremental delta scan for all cached corpora
ctxvault sync

# Sync a specific corpus
ctxvault sync --corpus project
```

---

## 6. Central Storage & SCM Control

* **Zero Repository Pollution**: Index artifacts default to `${CTXV_CACHE_DIR}/corpora/<name>/` (`meta.db`, `tantivy/`, `vectors.bin`, `graph.bin`), keeping git repositories clean. Local `.index/` is used only if already present on disk.
* **SCM Team Sharing**: Export compact, reproducible index bundles into `.ctxvault/vault.tar.zst` for git tracking or CI artifacts:
  ```bash
  # Export active repository index to .ctxvault/vault.tar.zst
  ctxvault export-artifact

  # Import bundle into central cache
  ctxvault import-artifact
  ```
* **Auto-Bootstrapping**: If a repository contains `.ctxvault/vault.tar.zst` and has not yet been indexed locally in central storage, `ctxvault` automatically unpacks and mounts the bundle upon discovery, avoiding expensive reindexing.
* **Zero CWD Fallback**: On server startup without explicit `--corpus` arguments, `ctxvault` auto-mounts all existing central corpora. If no cached corpora exist, it starts cleanly with 0 corpora rather than arbitrarily mounting the caller's working directory.
* **Continuous File Watching**: When `--watch` is enabled, `ctxvault` actively monitors all mounted corpora and automatically attaches file watchers to any new corpora mounted dynamically via `index_corpus`.
