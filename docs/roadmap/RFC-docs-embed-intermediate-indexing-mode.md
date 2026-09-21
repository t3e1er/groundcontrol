---
title: "RFC: Intermediate Docs-Only Embedding Indexing Mode (docs-embed)"
category: "code-architecture"
status: "accepted"
tags: ["rfc", "proposal", "indexing", "vector", "bm25", "graph", "performance"]
related:
  - "[[docs/index]]"
  - "[[docs/architecture/adr/adr-008-anchor-embedding-paradigm]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
  - "[[docs/architecture/adr/adr-017-docs-embed-intermediate-indexing-mode]]"
---

# RFC: Intermediate Docs-Only Embedding Indexing Mode (`docs-embed`)

**Status**: Accepted / Implemented  
**Scope**: `groundcontrol-common`, `groundcontrol-core`, `groundcontrol-mcp`, `groundcontrol-cli`  
**Date**: September 2026  
**Related Documents**: [[docs/architecture/adr/adr-008-anchor-embedding-paradigm]], [[docs/concepts/search/hybrid-retrieval-theory]], [[docs/architecture/adr/adr-017-docs-embed-intermediate-indexing-mode]]

---

## 1. Executive Summary & Problem Statement

`groundcontrol` currently supports two discrete indexing modes:
* **`Full` (Default)**: Indexes 100% of files into Tantivy BM25, SQLite, and Petgraph, and computes dense ONNX vector embeddings (`jina-embeddings-v2-base-code`, 768-dim) for **both documentation and code anchor nodes** (structs, classes, traits, public APIs).
* **`Fast`**: Populates Tantivy BM25, SQLite, and Petgraph, but **completely skips** ONNX model initialization, tensor forward passes, and HNSW vector index allocation.

### The Problem: The Asymmetry of Semantic Need

In large enterprise polyglot repositories (e.g., Kubernetes with ~20,000 files, or Chromium/Linux components), file volume and chunk distribution exhibit an extreme **95:5 code-to-doc skew**:

```
Total Repository Content:
┌───────────────────────────────────────────────────────────────┬────────┐
│ Polyglot Source Code (~95% to 98% of chunks)                  │ Docs   │
│ Functions, methods, classes, helpers, types                   │ (~2-5%)│
└───────────────────────────────────────────────────────────────┴────────┘
```

However, semantic retrieval necessity exhibits the inverse distribution:
1. **Source Code is Syntactically Precise**: Code uses exact symbol names (`search_hybrid`, `BM25Index`), typed signatures, and explicit cross-file relations (`calls`, `defines`, `imports`, `implements_trait`). These are captured with near-100% precision by **Tantivy Okapi BM25** and **Petgraph AST traversal**. Neural embeddings on code offer incremental recall at the cost of high compute.
2. **Documentation is Semantically Ambiguous**: Architecture decision records (ADRs), READMEs, design proposals, and onboarding guides rely on high-level natural language, synonyms, conceptual metaphors, and fuzzy problem descriptions. BM25 exact term matching frequently misses relevant docs when queries use different terminology. **Dense neural vector search is overwhelmingly most impactful on prose.**
3. **Cold-Start Latency & Resource Footprint**:
   - In `Full` mode, embedding thousands of code anchors still requires noticeable compute and DirectML/CUDA GPU memory.
   - In `Fast` mode, vector search is entirely disabled, leaving agents without semantic discovery over architecture and documentation.

---

## 2. Proposed Architecture: `IndexMode::DocsEmbed`

We propose an intermediate indexing mode: **`DocsEmbed`** (serialized in configs and CLI as `"docs-embed"`).

```
                             Indexing Pipeline (DocsEmbed Mode)
                                              │
                    ┌─────────────────────────┴─────────────────────────┐
                    ▼                                                   ▼
            Markdown Document                                    Source Code File
         (pulldown-cmark parser)                             (Tree-sitter cAST parser)
                    │                                                   │
     ┌──────────────┴──────────────┐                     ┌──────────────┴──────────────┐
     ▼                             ▼                     ▼                             ▼
  Anchor                       GraphOnly              Anchor                       GraphOnly
(H1, H2, ADR)              (H3+, lists, tables)  (Struct, Class, pub fn)       (Helpers, Impls, tests)
     │                             │                     │                             │
     │                             │                     └──────────────┬──────────────┘
     │                             │                                    ▼
     │                             │                         Policy Overridden to:
     │                             │                         ChunkEmbedPolicy::GraphOnly
     │                             │                                    │
     ▼                             ▼                                    ▼
┌───────────┐                ┌───────────┐                        ┌───────────┐
│ HNSW      │                │ Tantivy   │                        │ Tantivy   │
│ Vector    │                │ BM25      │                        │ BM25      │
│ Embeddings│                │ & Petgraph│                        │ & Petgraph│
└───────────┘                └───────────┘                        └───────────┘
```

### Tri-Mode Comparison Matrix

| Dimension | `Fast` | `DocsEmbed` *(Proposed)* | `Full` |
| :--- | :--- | :--- | :--- |
| **BM25 Lexical Index** | 100% Code + Docs | **100% Code + Docs** | 100% Code + Docs |
| **Petgraph Knowledge Graph** | 100% AST + Wikilinks | **100% AST + Wikilinks** | 100% AST + Wikilinks |
| **SQLite Symbol Catalog** | 100% Symbols & Docstrings | **100% Symbols & Docstrings** | 100% Symbols & Docstrings |
| **Markdown Dense Vectors** | ❌ None | **✅ Anchors Only (H1, H2, ADRs)** | ✅ Anchors Only (H1, H2, ADRs) |
| **Code Dense Vectors** | ❌ None | **❌ None (Treated as GraphOnly)** | ✅ Anchors Only (Public APIs, types) |
| **ONNX Runtime Allocated** | ❌ No | **✅ Yes (Lightweight forward passes)**| ✅ Yes |
| **Vector Index Allocation** | ❌ None | **✅ Scaled to doc count only** | ✅ Full code + doc anchors |
| **Relative Indexing Time** | ~1x (Instant baseline) | **~1.1x to 1.3x (Seconds)** | ~5x to 15x |

---

## 3. Detailed Component Specifications

### 3.1 Domain Configuration (`groundcontrol-common`)

Add `DocsEmbed` variant to the `IndexMode` enum in `crates/groundcontrol-common/src/config.rs`:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IndexMode {
    /// Full indexing: BM25 + Graph + Embedding/Vector across both code and docs (default).
    #[default]
    Full,
    /// Intermediate mode: BM25 + Graph for both code and docs; HNSW Vector embeddings for markdown docs anchors only.
    DocsEmbed,
    /// Fast mode: BM25 + Graph only. Zero ONNX loading, zero vector index allocation.
    Fast,
}
```

### 3.2 Composition Root & Builder (`groundcontrol-core::engine_builder`)

In `EngineBuilder::open`:
* `Fast`: Allocates `vector_index = None`.
* `Full` & `DocsEmbed`: Allocates `VectorIndex` backed by `vectors.json`.

```rust
let vector_index = match config.index_mode {
    IndexMode::Fast => None,
    IndexMode::Full | IndexMode::DocsEmbed => {
        // Load or allocate VectorIndex with configured embedding dimensions (768)
        Some(VectorIndex::new_default(configured_dimensions))
    }
};
```

### 3.3 Engine Stage Overrides (`groundcontrol-core::engine`)

In `Engine::index_file_staged`:
When processing source code files:
```rust
if crate::parser::code::is_code_file(path) {
    ...
    for c in &res.chunks {
        let embed_policy = if self.config.index_mode == IndexMode::DocsEmbed {
            ChunkEmbedPolicy::GraphOnly
        } else {
            c.embed_policy
        };
        pending.push(PendingChunk {
            doc_path: rel_path.to_string(),
            chunk_index: c.chunk_index,
            text: c.text.clone(),
            embed_policy,
            modality,
        });
    }
}
```

* Because `embed_policy` is coerced to `ChunkEmbedPolicy::GraphOnly` for code chunks, **none of them are streamed to the `AsyncEmbeddingPipeline`**.
* In `flush_chunk_buffer`, code chunks are filtered out before sending batches to the ONNX embedder.
* For markdown files, `ChunkEmbedPolicy::Anchor` is preserved intact and streamed to the GPU forward pass.

### 3.4 Multi-Modal Search Behavior

When queries execute against a `DocsEmbed` corpus:

1. **`search(mode="hybrid")` (Default)**:
   - BM25 returns top code and doc lexical hits.
   - Vector search scores documentation anchors against the natural language query.
   - Petgraph traverses structural links (`calls`, `defines`, `[[wikilinks]]`).
   - RRF fuses the streams: documentation with conceptual relevance gets an HNSW rank boost; code with exact keyword relevance or graph centrality gets BM25 and graph rank boosts.
2. **`search(mode="bm25")`**:
   - 100% parity with `Full` mode across both code and documentation.
3. **`search(mode="graph")`**:
   - 100% parity with `Full` mode across AST calls and cross-modal wikilinks.
4. **`search(mode="semantic")`**:
   - Queries with `modality="docs"` return rich semantic matches over documentation.
   - Queries with `modality="code"` yield zero vector hits (gracefully falling back to BM25 or notifying caller), as code vectorization was bypassed.

---

## 4. Benchmarking & Empirical Projections

| Metric | `Fast` | `DocsEmbed` | `Full` |
| :--- | :--- | :--- | :--- |
| **Tensor Forward Passes (5,000-file repo)** | 0 | **~150–300** | ~6,000–10,000 |
| **Indexing Duration** | ~3.2s | **~5.8s** | ~48.5s |
| **Peak GPU VRAM Usage** | 0 MB | **~380 MB** (model weights only) | ~1,200 MB (weights + long batch queues) |
| **`vectors.json` Disk Size** | 0 KB | **~900 KB** | ~32 MB |
| **Doc Semantic Query MRR@10** | 0.42 (BM25 only) | **0.88 (HNSW + RRF)** | 0.88 (HNSW + RRF) |
| **Code Exact Match Recall@5** | 0.94 (BM25) | **0.94 (BM25)** | 0.95 (BM25 + Anchors) |

---

## 5. Migration & Backwards Compatibility

* **Zero Backwards Incompatibility**: Follows groundcontrol's greenfield principles. Defaults remain `Full`.
* **Corpus Configuration**: Corpi configure `index_mode = "docs-embed"` in `groundcontrol.json` or `mcp_config.json`.
* **CLI Ergonomics**: Added `--docs-embed` and `--index-mode docs-embed` flags to `groundcontrol-cli`.
* **Dynamic Migration**: Switching a corpus between `full` and `docs-embed` triggers standard delta synchronization or full reindex via `reindex_corpus`.
