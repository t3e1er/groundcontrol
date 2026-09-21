---
title: "ADR 011: Segregated ReadOnly vs ReadWrite Handler Concurrency Model"
category: "mcp-modes"
status: "accepted"
tags: ["adr", "mcp", "concurrency", "rwlock", "threading", "decision"]
related:
  - "[[docs/mcp-modes/index]]"
  - "[[docs/mcp-modes/tool-surface-catalog]]"
  - "[[docs/agentic-strategy/swarm-topologies]]"
---

# ADR 011: Segregated ReadOnly vs ReadWrite Handler Concurrency Model

## Status
Accepted / Implemented

## Context
In multi-agent swarm environments, dozens of Scout and Reader agents query the MCP server concurrently. If every tool call locked the underlying `Engine` with a standard mutex, read requests would queue behind each other, introducing severe artificial latency. Conversely, allowing concurrent writes risks corrupting SQLite WAL transactions and Petgraph index state.

## Decision
We segregated tool handler functions in `crates/groundcontrol-mcp/src/tools/mod.rs` into two distinct types:
1. `ReadOnly(fn(&Engine, Value))`: Invoked under a shared reader lock (`Arc<RwLock<Engine>>::read()`).
2. `ReadWrite(fn(&mut Engine, Value))`: Invoked under an exclusive writer lock (`Arc<RwLock<Engine>>::write()`).

The tool registry enforces this distinction at compile time.

## Consequences

### Positive
- Lock-free reader concurrency: Multiple agents can run `search`, `get_snippet`, `graph_path`, and `backlinks` simultaneously across CPU cores.
- Thread-safe mutations: Write operations (`create_note`, `update_note`, `reindex_corpus`) safely serialize without race conditions.

### Trade-offs
- Long-running write operations (e.g. `reindex_corpus`) block incoming readers until the write completes. (Mitigated by granular delta syncing via `sync_corpus`).
