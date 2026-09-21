---
title: "Skeleton Mode Embedding: Analysis and Design"
category: "search"
status: "active"
tags: ["skeleton-mode", "embedding", "performance", "bow", "rrf", "indexing", "code-retrieval"]
related:
  - "[[docs/architecture/adr/adr-018-file-skeleton-map-chunking]]"
  - "[[docs/architecture/adr/adr-019-file-level-rrf-fusion]]"
  - "[[docs/architecture/adr/adr-008-anchor-embedding-paradigm]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
  - "[[docs/concepts/search/embeddings-vector]]"
---

# Skeleton Mode Embedding: Analysis and Design

A detailed technical analysis of `IndexMode::Skeleton` embedding performance, industry comparisons,
and the architectural decisions made to resolve throughput and fusion correctness issues.

## 1. The Problem: Volume vs Intent

`IndexMode::Skeleton` was designed to provide semantic bridging to code on commodity hardware —
signatures + docstrings instead of full bodies, targeted at laptops and mid-range GPUs. The intent
was ~10× fewer embeddings than `Full` mode.

**What actually happened**: the `classify_embed_policy` function in
[`chunker.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/code/chunker.rs)
assigns `ChunkEmbedPolicy::Anchor` to:

- All non-private, non-test Python functions
- All `export *` TypeScript/JavaScript symbols  
- All `public` Java/C# methods
- All unknown/generic language symbols (catch-all)
- All Rust `pub fn` with docstrings or at top-level

For a 10K-file mixed codebase: **100K–300K ONNX forward passes** — hours on a GTX 1070 under
DirectML's single-stream Windows constraint. Skeleton mode and Full mode have nearly identical
embedding volume. The only saving is shorter token sequences per chunk.

## 2. What Industry Does

Every major code retrieval system uses **file-level or section-level chunks** as the vector
embedding unit. Per-function embedding is not used at scale.

| System | Embedding unit | Functions included? |
|---|---|---|
| Continue.dev | 1–5 chunks/file (collapsed AST) | Signatures only, bodies `{ ... }` |
| Sourcegraph Cody | Fixed-size file windows | All text, sliding window |
| GitHub Copilot Workspace | File-level dense + symbol BM25 | File only |
| Aider repomap | No embedding (graph PageRank) | All signatures, no embedding |
| codebase-memory fast | BoW per-symbol, CPU-only static | Token lookup table |

### The Universal Pattern

```
Query
  ↓ Dense vector (file/section level)   → find relevant files
  ↓ BM25 + graph traversal              → find precise symbols
  ↓ read_file(line_range)               → fetch exact code
```

Dense embedding is a **coarse filter**. Symbol precision is BM25's and the graph's job.

## 3. BoW vs Jina at File Level: Is Simpler Better?

codebase-memory-mcp uses **Bag of Words with static pretrained token vectors** (int8 nomic-embed-code
distillate + Reflective Random Indexing co-occurrence enrichment). No neural inference. Indexing
the Linux kernel in 72 seconds.

### What BoW adds over BM25

BoW's marginal value over BM25 is **pre-trained synonymy**: tokens that co-occurred in nomic's
training corpus are close in vector space. `authenticate` ≈ `login` ≈ `oauth`. A file containing
`jwt`, `bearer`, `session_store` but not "authenticate" surfaces for the query "authentication logic".

| Query type | BM25 | BM25 + BoW | BM25 + Jina |
|---|---|---|---|
| Exact identifier | ✅ perfect | ✅ same | ✅ same |
| Code synonym | ❌ | ✅ good | ✅ excellent |
| Abstract concept | ❌ | ⚠️ partial | ✅ good |
| Multi-concept composition | ❌ | ⚠️ weak | ✅ better |
| Prose doc → code cross-modal | ❌ | ❌ poor | ✅ this is the goal |

**BoW adds ~40–50% of Jina's marginal value over BM25.** It captures synonym bridging but not
abstract composition or cross-modal semantic transfer.

### Why BoW is insufficient for skeleton mode's stated purpose

Skeleton mode exists for **semantic bridging to code** — specifically for agents exploring unfamiliar
codebases and for cross-modal doc-to-code linking. These queries are:

- "Find the authentication layer" (agent doesn't know the identifiers)
- "What code implements the architecture described in this ADR?" (cross-modal)

These are exactly the abstract conceptual queries where BoW degrades most relative to Jina.
BoW handles synonym bridging; it cannot handle conceptual composition across dissimilar vocabulary.

### Conclusion: Jina at file level, not BoW

The right answer is not to replace Jina with BoW. It is to **reduce the number of files we call
Jina on** — from N per file to 1–3 per file. Same quality per call, dramatically fewer calls.

## 4. The RRF Granularity Bug

Independent of embedding volume, `search_hybrid_full_single` has a correctness bug: it keys the
RRF fusion map at `(path, chunk_index)` for code. BM25 hits `chunk_index=5`, vector hits
`chunk_index=0` (file skeleton). These never merge — all code search results show either BM25
or vector score, never both. Markdown docs correctly key at `(path, None)`.

This means the "3-signal hybrid score" displayed for code results is misleading. See
[[docs/architecture/adr/adr-019-file-level-rrf-fusion]] for the fix.

File skeleton map chunking makes this concrete and forces the correct fix: once all code embeddings
are file-level, the natural RRF key is `(path, None)`, matching docs behaviour.

## 5. Progressive Disclosure Compatibility

The 3-tier progressive disclosure contract is unchanged:

| Tier | Before | After |
|---|---|---|
| T1 `search()` | Returns `(path, chunk_index=5, snippet="fn authenticate...")` | Returns `(path, chunk_index=5, snippet="fn authenticate...")` |
| T2 `get_snippet(path, chunk_index)` | Returns function body | Unchanged |
| T2 `get_snippet(name="authenticate")` | Returns function body | Unchanged |
| T3 `read_file(path, line_range)` | Returns raw file slice | Unchanged |

The `chunk_index` in T1 is now the **highest-BM25-ranked symbol** within the matched file, selected
post-RRF. The agent sees the same precision handle as before. The difference is that the file was
surfaced by genuinely fused 3-signal RRF rather than BM25-only or vector-only ranking.

## 6. Quantitative Impact

| Metric | Current Skeleton | File Skeleton Map |
|---|---|---|
| ONNX calls (10K file codebase) | ~100K–300K | ~10K–30K |
| GTX 1070 cold index time | ~hours | ~15–30 min |
| M3 Pro cold index time | ~30–60 min | ~3–5 min |
| RRF signal fusion | Silently broken for code | Genuine 3-signal |
| T1 snippet quality | Single function | Best BM25 function in matched file |
| Semantic quality | High (Jina per-symbol) | High (Jina per-file-skeleton) |
| BM25 precision | Unchanged | Unchanged |
| Graph precision | Unchanged | Unchanged |
