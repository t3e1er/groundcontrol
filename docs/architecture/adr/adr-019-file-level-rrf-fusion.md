---
title: "ADR 019: File-Level RRF Fusion Key for Code Search"
category: "code-architecture"
status: "accepted"
tags: ["adr", "rrf", "search", "hybrid-search", "fusion", "code-retrieval"]
related:
  - "[[docs/architecture/adr/adr-001-rrf-vs-learned-fusion]]"
  - "[[docs/architecture/adr/adr-018-file-skeleton-map-chunking]]"
  - "[[docs/architecture/adr/adr-010-unified-modal-search-tool]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
---

# ADR 019: File-Level RRF Fusion Key for Code Search

## Status
Accepted / In Implementation

## Context

### The Granularity Bug

`search_hybrid_full_single` in `crates/groundcontrol-core/src/search/mod.rs` keys the RRF fusion map
at `(path, chunk_index)` for code modality:

```rust
let key = if modality == Modality::Code {
    (r.path.clone(), r.chunk_index)   // BM25: symbol-level chunk
} else {
    (r.path.clone(), None)            // Docs: file-level
};
// ... and for vector results:
let key = if modality == Modality::Code {
    (vr.doc_path.clone(), vr.chunk_index)  // Vector: file-skeleton chunk
} else {
    (vr.doc_path.clone(), None)
};
```

BM25 returns `chunk_index=5` (the `authenticate` function). The vector index returns `chunk_index=0`
(the file skeleton aggregate for the same file). **These are different keys — they never merge in
the RRF map**. The 3-signal fusion is silently non-functional for code results. BM25 and vector
run parallel ranking pipelines that never intersect.

Markdown docs correctly use `(path, None)` — file-level RRF — and genuinely fuse all three signals.

### Why This Matters

The consequence is that reported score breakdowns are misleading: a code result showing
`bm25: 14.2, vector: 0.0` does not mean the vector index found nothing — it means the vector
index found a *different chunk* for the same file, which landed in a separate RRF bucket.

## Decision

For code modality, change the RRF fusion key from `(path, chunk_index)` to `(path, None)` —
matching the existing docs behaviour. Track the best BM25 chunk per file separately as the
Tier 1 representative result.

### Algorithm Change (search_hybrid_full_single)

**Step 3 (BM25 accumulation):**
```rust
// NEW: key at file level for all code results
let rrf_key = (r.path.clone(), None);
let entry = rrf_map.entry(rrf_key).or_insert(...);
entry.0 += rrf_score;
entry.1 = r.score.max(entry.1);  // keep highest raw BM25 score

// Track best BM25 chunk per file for Tier 1 snippet selection
best_bm25_chunk.entry(r.path.clone())
    .and_modify(|e| if r.score > e.1 { *e = (r.chunk_index, r.score, r.snippet.clone()); })
    .or_insert((r.chunk_index, r.score, r.snippet.clone()));
```

**Step 4 (vector accumulation):** Vector results already store `(path, chunk_index)` in
`VectorMeta`. Use `(vr.doc_path.clone(), None)` as the RRF key to merge into the same file bucket.

**Step 7 (result construction):** For each winning `(path, None)` entry, attach the
`best_bm25_chunk[path]` as `chunk_index` and `snippet`. If no BM25 chunk exists for the file
(pure vector or graph hit), `chunk_index = None`, snippet from file skeleton chunk text.

## Consequences

### Positive

- **Genuine 3-signal fusion for code**: BM25 rank + vector cosine + graph boost now all
  contribute to the same RRF score for the same file.
- **Accurate score breakdowns**: `score_components` will show real non-zero values across all
  three signals when all three agree on a file.
- **Progressive disclosure preserved**: returned `chunk_index` is still the most BM25-relevant
  symbol in the matched file. Tier 2 `get_snippet(path, chunk_index)` unchanged.
- **Vector boosts files BM25 can't reach**: a file matching the semantic query but lacking the
  exact query tokens now correctly surfaces and contributes its vector score to RRF.

### Trade-offs

- If a file has 50 symbol chunks and BM25 ranks chunk #12 highest, the result returns chunk #12
  as the representative. Other high-scoring chunks in the file are not surfaced in the same result
  slot. Agents wanting additional symbols in the file use `get_snippet(name=...)` or graph traversal.
- Pure graph-expanded results (no BM25 or vector hit) continue to have `chunk_index = None` and
  return the file path only — unchanged from current behaviour.

## Migration

No index migration required. The change is entirely in `search_hybrid_full_single` query execution.
Existing `vectors.json`, `meta.db`, `graph.bin` are unaffected.
