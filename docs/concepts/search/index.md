---
title: "Search & Multimodal Retrieval Hub"
description: "Theory, mathematics, and implementation of groundcontrol 4-modality hybrid retrieval and Reciprocal Rank Fusion."
category: "search"
status: "active"
tags: ["search", "hybrid", "bm25", "vectors", "graph", "rrf", "multimodal"]
related:
  - "[[docs/index]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
  - "[[docs/concepts/search/binary-hamming-embedding]]"
  - "[[docs/concepts/search/binaryv2-semantic-bridging]]"
  - "[[docs/concepts/search/rrf-mathematics]]"
  - "[[docs/concepts/search/bm25-lexical]]"
  - "[[docs/concepts/search/embeddings-vector]]"
  - "[[docs/concepts/search/graph-traversal]]"
  - "[[docs/concepts/search/search-modes-modalities]]"
  - "[[docs/concepts/search/benchmarking-harness]]"
  - "[[docs/concepts/search/evaluation-methodology]]"
---

# Search & Multimodal Retrieval Hub

Code and technical documentation are fundamentally heterogeneous. A pure dense vector model excels at abstract conceptual queries ("how does rate limiting work?") but fails catastrophically on exact code identifiers (`AuthTokenClaims`, `ERR_CONNECTION_RESET`). Conversely, pure keyword search fails when synonyms are used.

`groundcontrol` implements a **4-modality hybrid retrieval architecture** unified via **3-Way Reciprocal Rank Fusion (RRF)**.

---

## Retrieval Modalities

```
                                USER QUERY
                                    │
       ┌────────────────────────────┼────────────────────────────┐
       ▼                            ▼                            ▼
┌───────────────┐            ┌───────────────┐            ┌───────────────┐
│ Tantivy BM25  │            │ ONNX Dense    │            │ Petgraph /    │
│  (Lexical)    │            │ Vector (Jina) │            │ SQLite CTE    │
└───────┬───────┘            └───────┬───────┘            └───────┬───────┘
        │                            │                            │
        └────────────────────────────┼────────────────────────────┘
                                     ▼
                       ┌───────────────────────────┐
                       │ 3-Way RRF Fusion (k = 60) │
                       └─────────────┬─────────────┘
                                     ▼
                       Partitioned Docs & Code Hits
```

* **[[docs/concepts/search/binary-hamming-embedding]]**: Sub-millisecond algorithmic retrieval using FWHT rotation, 64-bit random hyperplane quantization, and LSH candidate indexing.
* **[[docs/concepts/search/binaryv2-semantic-bridging]]**: Enhanced 4-channel 256-bit Hamming retrieval with code-agnostic subword tokenization, unsupervised Reflective Random Indexing (RRI), and AST context.
* **[[docs/concepts/search/hybrid-retrieval-theory]]**: Why single-modality retrieval fails in polyglot codebases.
* **[[docs/concepts/search/rrf-mathematics]]**: Formal mathematics and proofs of Reciprocal Rank Fusion ($k=60$).
* **[[docs/concepts/search/bm25-lexical]]**: High-performance Tantivy Okapi BM25 for exact tokens and identifiers.
* **[[docs/concepts/search/embeddings-vector]]**: 768-dimensional Jina Code v2 ONNX embeddings with DirectML acceleration.
* **[[docs/concepts/search/graph-traversal]]**: Petgraph and recursive SQLite CTEs for typed AST dependency paths.
* **[[docs/concepts/search/search-modes-modalities]]**: Parameterizing `mode="hybrid"|"bm25"|"semantic"|"graph"|"explain"` and `modality="code"|"docs"|"both"`.
* **[[docs/concepts/search/benchmarking-harness]]**: Architecture and usage of the dedicated `groundcontrol-bench` workspace crate and CLI (`gc-bench`).
* **[[docs/concepts/search/evaluation-methodology]]**: ArXiv-grade IR evaluation standards, public benchmarks (CodeSearchNet, RepoBench, SWE-bench), significance testing, and Turn-1 orientation metrics.
