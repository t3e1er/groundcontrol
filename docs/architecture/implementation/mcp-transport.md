---
title: "MCP Transport & Authoritative Tool Registry"
description: "Stdio framing, Axum HTTP SSE transport, and the authoritative 17-tool handler registration in groundcontrol."
category: "implementation"
status: "active"
tags: ["mcp", "transport", "stdio", "http", "sse", "registry", "17-tools"]
related:
  - "[[docs/architecture/implementation/index]]"
  - "[[docs/concepts/progressive-disclosure/tool-profiles]]"
  - "[[docs/architecture/adr/adr-011-readonly-readwrite-handler-model]]"
---

# MCP Transport & Authoritative Tool Registry

`groundcontrol-mcp` implements the **Model Context Protocol (MCP)** specification with dual transports: standard input/output (stdio JSON-RPC) for local subagents, and Axum HTTP with Server-Sent Events (SSE) for distributed swarms.

---

## 1. Concurrency Model: ReadOnly vs ReadWrite Handlers

To maximize agent throughput, tool handlers are registered as either:
* **`ReadOnly(fn(&Engine, Value))`**: Can execute concurrently across multiple threads without locking the engine. (e.g. `search`, `get_snippet`, `read_file`, `graph_match`).
* **`ReadWrite(fn(&mut Engine, Value))`**: Acquires an exclusive write lock to perform atomic updates. (e.g. `write_note`, `delete_note`, `sync_corpus`).

This prevents read requests from stalling behind background indexing jobs.

---

## 2. Authoritative 17-Tool Registry

The authoritative registry in `crates/groundcontrol-mcp/src/tools/mod.rs` defines the complete protocol surface:

```rust
// Authoritative 17 Tools across 5 Functional Domains:
// Read:       read_file, get_snippet, list_notes
// Search:     search, search_related
// Graph:      graph_match, graph_communities
// Write:      write_note, delete_note, move_note
// Validation: validate, list_templates
// System:     status, list_corpora, sync_corpus, index_corpus, unload_corpus
```

Each tool handler performs strict schema validation on incoming JSON-RPC payloads, returning actionable error diagnostics (such as available taxonomy values or valid line slices) if arguments are invalid.

---

## 3. Client Identity, `x-api-key` Authentication & Telemetry Correlation

To track multi-agent swarm activity in real time, `groundcontrol-mcp` resolves incoming connections against the client registry ([`ClientsRegistry`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/client.rs)):

### Authentication Model & Zero-Auth Default
* **Default Zero-Auth**: Out of the box, `require_auth` is disabled (`false`). Local developers can connect without configuring secret keys.
* **Authentication Header**: Clients supply `x-api-key: <token>` (or `Authorization: Bearer <token>`). In CLI environments, `GROUNDCONTROL_API_KEY` can be used.
* **Strict Rejection**: When `require_auth: true` is configured in `clients.json` or enabled via `--require-auth` / `GROUNDCONTROL_REQUIRE_AUTH=true`, requests lacking a valid matching key are rejected with `401 Unauthorized` and JSON-RPC error code `-32000`:
  ```json
  {
    "jsonrpc": "2.0",
    "id": null,
    "error": {
      "code": -32000,
      "message": "Unauthorized: missing or invalid x-api-key"
    }
  }
  ```

### Telemetry Correlation
When a valid API key is resolved, the agent's identity (`client_id`, `client_name`, `client_color`) is attached to telemetry events (`AgentActivation`) streamed via SSE to the 3D GraphView visualizer.

### Client Configuration Management (`groundcontrol client`)
Client profiles and keys can be self-configured via the CLI:
* `groundcontrol client init`: Generates a local `clients.json` template populated with cryptographically random API keys and an internal `daemon_key` for GraphView relay.
* `groundcontrol client list`: Displays active client profiles, theme colors, and authentication status.
