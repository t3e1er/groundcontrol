---
title: "Implementation & Systems Architecture Hub"
description: "Internals of groundcontrol: hexagonal ports and adapters, Tree-sitter cAST parsing, GPU governor, and federation."
category: "implementation"
status: "active"
tags: ["implementation", "internals", "architecture", "hexagonal", "cast", "gpu", "directml"]
related:
  - "[[docs/index]]"
  - "[[docs/architecture/implementation/hexagonal-architecture]]"
  - "[[docs/architecture/implementation/cast-chunking]]"
  - "[[docs/architecture/implementation/gpu-and-directml]]"
  - "[[docs/architecture/implementation/zero-copy-storage]]"
  - "[[docs/architecture/implementation/cross-corpus-federation]]"
  - "[[docs/architecture/implementation/mcp-transport]]"
  - "[[docs/architecture/implementation/graphview]]"
  - "[[docs/architecture/implementation/file-exclusion]]"
---

# Implementation & Systems Architecture Hub

`groundcontrol` is engineered in pure safe Rust (`#![forbid(unsafe_code)]`) with a clean hexagonal architecture. This hub details internal systems design, hardware memory governors, AST parsing engines, and zero-copy storage layouts.

---

## Architectural Pillars

* **[[docs/architecture/implementation/hexagonal-architecture]]**: Ports & adapters pattern; zero leaking of Tantivy, rusqlite, or ONNX types across module boundaries.
* **[[docs/architecture/implementation/algorithm-substrate]]**: Pluggable algorithm components (`bm25`, `binary`, `graph`, `dense`, `composite`) conforming to `RetrievalAlgorithm`.
* **[[docs/architecture/implementation/file-exclusion]]**: Central corpus gitignore-equivalent pattern matcher, subtree pruning, and multi-layered indexing skips.
* **[[docs/architecture/implementation/cast-chunking]]**: Tree-sitter cAST polyglot parsing across 16+ languages with parent scope breadcrumb injection.
* **[[docs/architecture/implementation/gpu-and-directml]]**: DirectX 12 DirectML acceleration, AIMD VRAM governor, and TDR driver watchdog resilience.
* **[[docs/architecture/implementation/zero-copy-storage]]**: Packed binary vector files, zero-copy byte offsets, and SQLite catalog design.
* **[[docs/architecture/implementation/cross-corpus-federation]]**: Multi-corpus routing, external symbol reconciliation, and federated graph traversal.
* **[[docs/architecture/implementation/mcp-transport]]**: Stdio JSON-RPC framing, HTTP SSE streaming, and the authoritative 17-tool registry.
* **[[docs/architecture/implementation/graphview]]**: Standalone 3D visualization sidecar architecture, binary wire protocol, parallel Barnes-Hut layout, and agent SSE telemetry.

---

## Crate Dependency Topology

```
                  ┌───────────────────────┐
                  │      groundcontrol-cli     │ (Composition Root)
                  └───────────┬───────────┘
                              │
                  ┌───────────▼───────────┐
                  │      groundcontrol-mcp     │ (Transport & 17 Tools)
                  └───────────┬───────────┘
                              │
                  ┌───────────▼───────────┐
                  │     groundcontrol-core     │ (Engine & Backend Adapters)
                  └───────────┬───────────┘
                              │
                  ┌───────────▼───────────┐
                  │    groundcontrol-common    │ (Domain Types & Port Traits)
                  └───────────────────────┘
```
