---
title: "Hybrid Retrieval Theory"
description: "Why single-modality retrieval systems fail in real-world codebases and how 4-modality hybrid fusion succeeds."
category: "search"
status: "active"
tags: ["retrieval-theory", "hybrid-search", "bm25", "dense-vectors", "graph-rag"]
related:
  - "[[docs/concepts/search/index]]"
  - "[[docs/concepts/search/rrf-mathematics]]"
  - "[[docs/concepts/search/search-modes-modalities]]"
  - "[[docs/architecture/adr/adr-001-rrf-vs-learned-fusion]]"
---

# Hybrid Retrieval Theory

Single-modality retrieval engines exhibit distinct, predictable failure modes when applied to polyglot software repositories.

---

## The Trilemma of Code Retrieval

```
             Tantivy BM25 (Exact Identifiers)
                       ▲
                      / \
                     /   \
                    /     \
                   /       \
                  /  gc   \
                 /  Hybrid   \
                /   Engine    \
               /               \
              /                 \
Dense Vector Space ───────────── Petgraph / SQLite CTE
(Semantic Concepts)             (Structural AST Graph)
```

### 1. Vector-Only Failure Modes
Dense embeddings encode high-level semantic meaning into continuous Euclidean vector spaces. However:
* **The Identifier Blindspot**: A query for `WorkerPoolConfigV2` will often rank `WorkerPoolConfigV1` or general threadpool discussions higher because their cosine similarity is nearly 0.98.
* **Out-of-Vocabulary (OOV) Tokens**: Compiler error codes, UUIDs, hex offsets, and specific flags are poorly resolved by subword tokenizers.

### 2. Keyword-Only (BM25) Failure Modes
Okapi BM25 scores documents based on exact term frequencies and inverse document frequencies. However:
* **The Synonym Blindspot**: Searching for *"rate limiting"* will miss an implementation named `TokenBucketThrottler` or `LeakyBucketLease`.
* **Conceptual Abstractions**: It cannot map intent ("where do we handle database failover?") to code unless verbatim terms exist in comments.

### 3. Graph-Only Failure Modes
Knowledge graphs map explicit syntax relationships (`A calls B`). However:
* **Cold Entry Problem**: Graph traversal requires a starting node. Without lexical or semantic retrieval to find the initial anchor, graph BFS cannot begin.

---

## The 4-Modality Solution

`groundcontrol` eliminates all three failure modes by executing **lexical, vector, and graph retrieval in parallel**, then fusing the rank distributions into a single, high-confidence result set via Reciprocal Rank Fusion (RRF).
