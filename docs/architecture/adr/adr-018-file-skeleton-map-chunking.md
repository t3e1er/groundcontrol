---
title: "ADR 018: File Skeleton Map Chunking for Skeleton Indexing Mode"
category: "code-architecture"
status: "superseded"
tags: ["adr", "skeleton-mode", "embedding", "chunking", "performance", "indexing", "superseded"]
related:
  - "[[docs/architecture/adr/adr-008-anchor-embedding-paradigm]]"
  - "[[docs/architecture/adr/adr-017-docs-embed-intermediate-indexing-mode]]"
  - "[[docs/architecture/adr/adr-019-file-level-rrf-fusion]]"
  - "[[docs/concepts/search/binary-hamming-embedding]]"
---

# ADR 018: File Skeleton Map Chunking for Skeleton Indexing Mode

## Status
Superseded by 2-Mode Modality Convergence (`IndexMode::Full` and `IndexMode::Fast`).

> **Note**: As of the Modality Convergence, `IndexMode::Skeleton` has been removed. Code modality is handled by Algorithmic Binary Hamming distance matching in CPU registers without requiring any neural vector embeddings.

## Context

`IndexMode::Skeleton` was designed to provide semantic bridging to code on commodity hardware ("genuine
cross-platform mode") by embedding code symbol skeletons (signature + docstring + scope breadcrumb)
rather than full function bodies. The intent was to rival the throughput of fast-mode tools while
retaining meaningful semantic code search.

### The Volume Problem

Analysis of `classify_embed_policy` in `crates/groundcontrol-core/src/parser/code/chunker.rs` revealed
that skeleton mode embeds **one vector per qualifying AST symbol**, not one per file:

- **Python**: all non-`_`-prefixed, non-test functions → `Anchor`
- **TypeScript/JS**: all `export *` → `Anchor`
- **Java/C#**: all `public` methods → `Anchor`
- **Generic languages**: catch-all `Anchor`

For a 10K-file mixed codebase this produces **100K–300K ONNX forward passes** through
`jina-embeddings-v2-base-code` — hours on a GTX 1070 under DirectML's single-stream constraint.

### Industry Precedent

Industry code retrieval systems (Continue.dev, Sourcegraph Cody, GitHub Copilot Workspace) uniformly
use **file-level or section-level chunks** as the unit of dense embedding. Symbol-level precision
is delegated to BM25 and AST graph traversal:

- **Continue.dev**: Tree-sitter collapse-then-slide; produces 1–5 chunks per file, each containing
  multiple collapsed function signatures with bodies replaced by `{ ... }`.
- **Sourcegraph Cody**: Fixed-size overlapping file windows; no per-function embedding.
- **GitHub Copilot Workspace**: Two-stage — file-level dense retrieval, then symbol-level BM25.

### The RRF Granularity Discovery

Separately, `search_hybrid_full_single` in `crates/groundcontrol-core/src/search/mod.rs` was found to
key its RRF fusion map at `(path, chunk_index)` for code results. BM25 returns symbol-level
chunk_indices; vector search returns different chunk_indices for the same file. These never merge in
the RRF map — the 3-signal fusion is silently broken for code. Docs correctly key at `(path, None)`.
This is addressed in [[docs/architecture/adr/adr-019-file-level-rrf-fusion]].

## Decision

Replace per-symbol `PendingChunk` emission with **file skeleton map aggregation** in
`IndexMode::Skeleton`:

1. After AST parsing produces N per-symbol `Chunk` records (unchanged for BM25/graph/SQLite),
   aggregate their `skeleton_text` fields into 1–K file-level `PendingChunk` entries for embedding.

2. **Aggregation strategy**:
   - Group symbols by their top-level container scope (all methods/associated functions of a
     `struct`/`class`/`trait`/`impl` rendered together).
   - Within each scope group, order: type definition → public functions → internal functions.
   - Pack greedily into chunks of `max_tokens` (default 512). If a single scope group exceeds
     the limit, split at function boundaries with a breadcrumb prefix on each continuation chunk.
   - Result: a file skeleton map chunk reads like an Aider-style repo map section — all signatures
     visible, no bodies.

3. **Embedding target**: The aggregated skeleton chunk text is sent to the ONNX pipeline as a
   single `PendingChunk` with `embed_policy = ChunkEmbedPolicy::Anchor`. The `chunk_index` stored
   in `VectorMeta` references the first symbol's original chunk index (used as a file-level handle).

4. **No change to BM25, SQLite, or graph indexing**: per-symbol chunks remain unchanged. Only the
   vector embedding path changes in skeleton mode.

5. **Query-side unchanged**: `get_snippet(name=...)` and `get_snippet(path, chunk_index)` continue
   to address per-symbol chunks via the unchanged symbol index.

## Consequences

### Positive

- **10–20× reduction in ONNX forward passes**: a 10K-file codebase goes from ~100K–300K to
  ~10K–30K embedding calls. On a GTX 1070: hours → 15–30 minutes. On an M3 Pro: 30–60min → 3–5min.
- **No quality regression**: Jina reads all public function signatures in one chunk — the semantic
  content is richer than per-symbol (the inter-function vocabulary context improves retrieval).
- **Correct RRF fusion** (combined with ADR-019): vector and BM25 signals now merge at the file
  level, producing genuine 3-signal scores instead of parallel non-overlapping results.
- **Progressive disclosure preserved**: agents still receive `(path, chunk_index, snippet)` in
  Tier 1. `chunk_index` resolves to the best BM25 symbol in the matched file via the new
  file-level RRF architecture. Tier 2 `get_snippet` is unchanged.

### Trade-offs

- `get_snippet(path, chunk_index=N)` where N is a file-skeleton aggregate chunk returns the
  aggregated skeleton text rather than a single function body. This is acceptable and often more
  useful — the agent sees the full API surface, then uses `get_snippet(name=...)` for a specific body.
- Files with very large public APIs (>1500 tokens of signatures) produce 2–3 skeleton chunks
  rather than 1. This is bounded and predictable.

## Alternatives Considered

### BoW + Random Indexing (codebase-memory approach)

Evaluated as a zero-GPU alternative. Conclusion: BoW adds ~40–50% of Jina's marginal value over
BM25. For skeleton mode's core purpose — **semantic bridging to code for agents exploring unfamiliar
codebases, and cross-modal doc-to-code linking** — BoW is insufficient. The queries that justify
semantic code embedding are abstract conceptual queries ("find the authentication layer") where the
codebase vocabulary does not include the query term. This is exactly where BoW degrades relative to
Jina. See analysis in `docs/concepts/search/skeleton-embedding-analysis.md`.

### Tighter classify_embed_policy for Skeleton mode (types only)

Only embedding domain type definitions (Struct, Enum, Trait, Interface) in skeleton mode would
reduce volume by ~60–80% but sacrifices function-level semantic signal. An agent querying "find
where JWT tokens are validated" needs function signatures, not just type names. Rejected in favour
of file skeleton aggregation which includes all signatures at 1/N the cost.
