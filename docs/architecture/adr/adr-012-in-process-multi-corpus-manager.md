---
title: "ADR 012: In-Process Multi-Corpus Serving via CorpusManager"
category: "mcp-modes"
status: "accepted"
tags: ["adr", "multi-corpus", "corpus-manager", "architecture", "decision"]
related:
  - "[[docs/mcp-modes/index]]"
  - "[[docs/mcp-modes/multi-corpus-serving]]"
---

# ADR 012: In-Process Multi-Corpus Serving via CorpusManager

## Status
Accepted / Implemented

## Context
Developers working on complex systems often require access to multiple corpora simultaneously (e.g. documentation vault, microservice repo, shared library). Spawning a separate OS process for each corpus multiplies VRAM consumption (each process loading its own ONNX embedding model, ~550 MB each) and prevents cross-corpus symbol resolution.

## Decision
We implemented **[`CorpusManager`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/corpus_manager.rs)**:
1. A single persistent MCP process manages $N$ independent index roots.
2. The ONNX embedding model instance and thread pool are shared across corpora, avoiding VRAM duplication.
3. Every engine manages its own isolated index directory, defaulting to central cache (`${GROUNDCONTROL_CACHE_DIR}/corpora/<name>/`) containing `meta.db`, `tantivy/`, `vectors.bin`, and `graph.bin`, keeping source repos pristine. Local `.index/` is used only if already present on disk.
4. Repositories can commit `.groundcontrol/vault.tar.zst` artifacts for automated zero-reindex bootstrapping.
5. On server startup without explicit `--corpus` arguments, all cached corpora in central storage are auto-mounted. If none exist, the server starts cleanly with 0 corpora (no CWD fallback).
6. Cross-corpus fan-out queries and unambiguous symbol linking are performed in-memory.

## Consequences

### Positive
- Memory footprint is amortized: serving 5 corpora consumes roughly the same VRAM as serving 1 corpus.
- Cross-corpus symbol linking enables seamless cross-modal navigation between architecture notes and code repositories.
- CLI ergonomics: `groundcontrol index <path>` and `groundcontrol sync` manage central storage directly, while `--corpus name=path` can be specified multiple times on server invocations.
- Git repositories remain completely clean of temporary indexing artifacts.

### Trade-offs
- A crash or panic in one corpus engine could terminate the shared process (mitigated by `#![forbid(unsafe_code)]` and pure Rust error handling).
