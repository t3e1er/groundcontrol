---
title: "Auto-Daemon & Shared Server Deployment"
description: "Deploying groundcontrol as a background daemon, hosting multi-corpus servers, and script automation."
category: "building"
status: "active"
tags: ["daemon", "server", "http", "sse", "multi-corpus", "concurrency", "config"]
related:
  - "[[docs/architecture/building/index]]"
  - "[[docs/architecture/implementation/cross-corpus-federation]]"
  - "[[docs/architecture/implementation/mcp-transport]]"
---

# Auto-Daemon & Shared Server Deployment

`groundcontrol` supports multiple operational modes: standard per-agent stdio processes, transparent background daemons, dedicated multi-corpus HTTP servers, and centralized corpus lifecycle commands.

---

## 1. Background Daemon Lifecycle (`groundcontrol daemon`)

When multiple subagents or editor windows run concurrently on the same machine, spawning multiple independent index engines wastes memory and locks SQLite databases.

The **Daemon Subsystem** manages a shared background process serving the indexed corpora over localhost HTTP JSON-RPC and SSE:

```bash
# Start background daemon (no-sync by default for instant startup)
groundcontrol daemon start

# Start background daemon and trigger an immediate delta sync
groundcontrol daemon start --sync

# Check daemon health, uptime, PID, active corpora, and in-flight indexing progress
groundcontrol daemon status

# Trigger incremental delta sync inside running daemon
groundcontrol daemon sync
groundcontrol daemon sync --corpus groundcontrol

# Gracefully terminate daemon (SQLite checkpoint flush & PID cleanup)
groundcontrol daemon stop

# Restart daemon
groundcontrol daemon restart --sync
```

### Auto-Daemon Transparent Stdio Proxy
When launched with zero arguments (e.g. from an IDE MCP configuration), `groundcontrol` operates in **Auto Mode**:
1. Checks if a local daemon is listening on port `9090`.
2. If absent, it automatically detaches a shared background daemon.
3. The foreground CLI bridges standard stdio JSON-RPC to the daemon over HTTP with zero agent configuration changes.

---

## 2. Dedicated Multi-Corpus Server Mode (`groundcontrol server`)

For team environments, sandboxed CI agents, or multi-agent swarms, run `groundcontrol` as a foreground server hosting $N$ distinct index roots:

```bash
groundcontrol server --bind 127.0.0.1:9090 \
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
* `GET /status`: Extended status reporting daemon uptime, PID, loaded corpora list, and active indexing progress.
* `POST /sync`: Triggers in-daemon incremental delta scan across one or all mounted corpora.
* `POST /shutdown`: Gracefully flushes WAL logs, checkpoints SQLite, and shuts down the daemon.

---

## 3. Central Corpus Management (`groundcontrol corpus`)

Manage repository registration, indexing, synchronization, and storage footprints across the machine:

```bash
# List all registered corpora with disk footprints and indexing modes
groundcontrol corpus list

# Register a repository and index it immediately
groundcontrol corpus add C:\dev\my-project

# Register in fast mode (BM25 + Graph only, zero ONNX embeddings)
groundcontrol corpus add C:\dev\my-project --mode fast

# Register without immediate indexing
groundcontrol corpus add C:\dev\my-project --no-index

# Deregister a corpus and optionally purge on-disk index cache
groundcontrol corpus remove my-project
groundcontrol corpus remove my-project --purge

# View or set the default active corpus
groundcontrol corpus default
groundcontrol corpus default my-project

# Synchronize one or all corpora with terminal progress ticker
groundcontrol corpus sync
groundcontrol corpus sync groundcontrol
```

---

## 4. Scripted Tool Execution (`groundcontrol call`)

Interact with a running daemon or local engine directly from shell scripts or CI pipelines without an MCP editor:

```bash
# Search using hybrid mode with query shorthand
groundcontrol call search --query "authentication token"

# Search with detailed JSON arguments
groundcontrol call search --args '{"query": "authentication token", "mode": "hybrid", "snippets": 3}'

# Inspect multi-hop graph lineage
groundcontrol call graph_match --args '{"pattern": "(:CodeSymbol {name: \"verify_jwt\"})-[:calls*1..2]->(target)"}'
```

---

## 5. Portable Team Sharing Artifacts (`corpus export` / `corpus import`)

Package derived indices (SQLite metadata, Tantivy BM25, and Petgraph graph) into compressed `.groundcontrol/vault.tar.zst` archives for team onboarding and CI caching without reindexing:

```bash
# Export active repository index to .groundcontrol/vault.tar.zst
groundcontrol corpus export

# Export specific corpus to custom destination
groundcontrol corpus export groundcontrol -o team-vault.tar.zst

# Import bundle into central storage
groundcontrol corpus import
groundcontrol corpus import -i team-vault.tar.zst groundcontrol
```

* **Zero Repository Pollution**: Index artifacts default to `${GROUNDCONTROL_CACHE_DIR}/corpora/<name>/` (`meta.db`, `tantivy/`, `vectors.bin`, `graph.bin`), keeping git repositories clean.
* **Auto-Bootstrapping**: If a repository contains `.groundcontrol/vault.tar.zst` and has not yet been indexed in central storage, `groundcontrol` automatically unpacks and mounts the bundle upon discovery.
* **Zero CWD Fallback**: On server startup without explicit `--corpus` arguments, `groundcontrol` auto-mounts all existing central corpora. If no cached corpora exist, it starts cleanly with 0 corpora rather than arbitrarily mounting the caller's working directory.
