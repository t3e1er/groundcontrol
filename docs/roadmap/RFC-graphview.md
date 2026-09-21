# RFC: Standalone High-Performance 3D GraphView & Multi-Agent Activation Substrate

**Status**: Accepted / Implementing  
**Author**: Architecture Team  
**Scope**: `crates/groundcontrol-graphview`, `crates/groundcontrol-mcp`, `crates/groundcontrol-cli`, `crates/groundcontrol-core`  
**Date**: September 2026  
**Target Version**: `0.1.0`+  
**Inspiration / Benchmark**: `codebase-memory-mcp` (Graph UI)  
**Related Documents**: 
- [[docs/architecture/implementation/graphview]]
- [[docs/concepts/progressive-disclosure/graphview-lod]]
- [[docs/architecture/implementation/mcp-transport]]
- [[docs/roadmap/coderoadmap]]

---

## 1. Executive Summary

This RFC specifies **GraphView**, a high-performance, standalone 3D knowledge graph visualization dashboard and real-time multi-agent activity substrate for `groundcontrol`. 

While `groundcontrol`'s primary mission is headless, ultra-low-latency semantic retrieval for autonomous AI coding swarms, human developers and swarm orchestrators require visual intuition regarding repository topology, cross-corpus coupling, and agent focus areas.

GraphView introduces a dedicated sidecar service (`gc graphview`) that:
1. Renders unified documentation, code ASTs, and cross-corpus links in an interactive 3D WebGL scene.
2. Supports massive scale ($1\text{M}+$ nodes and $3\text{M}+$ edges) from Day 1 via parallel server-side Barnes-Hut layout, packed binary wire serialization, and 4-tier visual Level-of-Detail (LOD).
3. Connects directly to the core MCP server over Server-Sent Events (SSE) to display real-time animated activation pulses as agents navigate and crystallize repository memory.
4. Preserves absolute zero-overhead isolation on the core `gc` MCP daemon runtime.

---

## 2. Motivation & Success Criteria

### Functional Requirements
1. **Multi-Modal Views**: User can toggle between `docs` (knowledge notes, ADRs), `code` (functions, structs, modules), and `hybrid` (cross-modal wikilinks and docstring ties) modes.
2. **Entity Type Filtering & Highlights**: A persistent side panel allows selecting and isolating specific entity classes (e.g., functions, traits, modules, ADR notes) with dynamic neon highlighting.
3. **Exposed Query Engine**: Live search bar connected to `groundcontrol`'s query engine (BM25, hybrid, graph match), dynamically highlighting matched subgraphs and community clusters.
4. **Rich Aesthetic Experience**: Modern dark-space design featuring custom bloom shaders, curated color palettes, glow halos, and smooth camera transitions.
5. **Multi-Agent Temporal Activations**: Real-time visualization of agent activity over SSE, showing animated activation waves on accessed nodes with configurable persistence/decay controls.
6. **Multi-Corpus & Galaxy Selection**: Ability to view a single repository corpus or federate all loaded corpora into a galaxy view with cluster separation.

### Non-Functional Requirements (NFR)
- **Scale Guarantee ($1\text{M}+$ Nodes Day 1)**: Must handle $1\text{M}+$ nodes and $3\text{M}+$ edges without crashing the browser or freezing the UI.
- **Pure Rust Safety (`#![forbid(unsafe_code)]`)**: Backend layout and networking written in 100% safe Rust.
- **Non-Disruptive Sidecar**: The visualization engine must never run within the MCP query hot path or lock write operations in SQLite or petgraph.

---

## 3. Architecture Evaluation: Daemon-Embedded vs Standalone Sidecar

We evaluated three potential deployment topologies:

| Evaluation Dimension | Option A: In Core Daemon | Option B: In-Process Thread | Option C: Standalone Sidecar (Selected) |
|---|---|---|---|
| **Impact on Core MCP Binary** | Bloats binary with HTTP asset bundling & Three.js static files | Bloats binary, shares heap memory | **Zero bloating. Core remains lean and focused** |
| **CPU/RAM Isolation** | Layout calculation ($O(N \log N)$) steals CPU from agent retrieval | Layout steals threads from Rayon pool | **Complete OS process isolation. Can run on separate cores** |
| **Crash Safety** | WebGL/HTTP panic crashes active agent sessions | Thread panic risks poison locks | **Sidecar crash has zero effect on MCP daemon** |
| **Database Access** | In-memory pointer to petgraph | In-memory pointer | **Read-only zero-copy disk ingest (`graph.bin` + WAL `meta.db`)** |
| **Agent Telemetry** | In-memory channel | In-memory channel | **Lightweight HTTP SSE event stream relay** |

**Architectural Decision**: **Option C (Standalone Sidecar)** is selected. It completely preserves the core runtime's greenfield purity while granting the visualizer unlimited architectural freedom.

---

## 4. Technical Design

### 4.1 Storage & Snapshot Ingest
The sidecar loads graph topology directly from the cache directory:
- **`graph.bin`**: Deserialized via `postcard` into a read-only [`KnowledgeGraph`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/graph/mod.rs).
- **`meta.db`**: Opened with `rusqlite` using `OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI` and `PRAGMA busy_timeout = 5000`.

### 4.2 Parallel Barnes-Hut 3D Layout
Layout is computed ahead-of-time in Rust using an arena-allocated octree with `rayon` parallelism:
1. Populate `Vec<OctreeNode>` flat arena.
2. Partition node space across Rayon worker threads.
3. Compute repulsive forces with opening angle $\theta = 0.8$.
4. Compute attractive forces along graph edges.
5. Apply community centroid bias to separate multi-corpus clusters.

### 4.3 Packed Binary Wire Protocol
Data is streamed to the browser via `/api/graph/corpus/:name` with `Content-Type: application/octet-stream`:
- **Header**: 16 bytes (Magic, Version, NodeCount, EdgeCount).
- **Nodes**: 32 bytes/node (`id: u32`, `type: u8`, `community: u16`, `pos: [f32; 3]`, `size: f32`).
- **Edges**: 12 bytes/edge (`source: u32`, `target: u32`, `type: u16`, `weight: u16`).
- **String Table**: Length-prefixed UTF-8 identifiers appended at the tail.

### 4.4 Multi-Agent Telemetry Relay
1. MCP daemon emits `AgentActivation` JSON payloads on `/events/activations`.
2. GraphView sidecar maintains a background Tokio task subscribed to the stream.
3. Sidecar relays events to all connected web clients via its own SSE endpoint `/api/events/activations`.

---

## 5. Implementation Plan & Milestones

1. **M1: Crate Setup & Data Ingest**: Create `crates/groundcontrol-graphview`, add dependencies, and implement read-only snapshot loading.
2. **M2: Layout Engine**: Implement arena Barnes-Hut octree in pure safe Rust with Rayon parallelism.
3. **M3: Binary Serializer & Axum Server**: Build binary wire protocol serializer and Axum REST/SSE endpoints.
4. **M4: Agent Telemetry Hook**: Add lightweight event emission in `groundcontrol-mcp` HTTP transport.
5. **M5: React 19 / Three.js Frontend**: Develop the dashboard SPA with 4-tier LOD, particle shaders, and bloom effects.
6. **M6: CLI Integration & Verification**: Add `gc graphview` command to `groundcontrol-cli` and verify 60 FPS performance.
