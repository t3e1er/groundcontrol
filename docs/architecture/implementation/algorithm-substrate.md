---
title: "Modular Algorithm Substrate & Retrieval Architecture"
description: "How groundcontrol decomposes retrieval backends into isolated, pluggable algorithm components conforming to RetrievalAlgorithm."
category: "implementation"
status: "active"
tags: ["algorithm-substrate", "retrieval", "ports", "bm25", "binary", "graph", "dense", "composite"]
related:
  - "[[docs/architecture/implementation/index]]"
  - "[[docs/architecture/implementation/hexagonal-architecture]]"
  - "[[docs/architecture/adr/adr-021-modular-retrieval-algorithm-substrate]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
---

# Modular Algorithm Substrate & Retrieval Architecture

`groundcontrol` decomposes indexing and retrieval into an **Algorithm Substrate** — an architecture of self-contained components conforming to the [`RetrievalAlgorithm`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-common/src/ports/algorithm.rs) port trait.

Rather than running monolithic orchestration logic in the core engine, each algorithm owns its internal indexing schema, document lifecycle, persistence, and execution strategy.

---

## 1. The `RetrievalAlgorithm` Port Trait

Defined in [`crates/groundcontrol-common/src/ports/algorithm.rs`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-common/src/ports/algorithm.rs):

```rust
pub trait RetrievalAlgorithm: Send + Sync {
    fn name(&self) -> &'static str;
    fn index_document(&mut self, artifact: &ParsedArtifact) -> Result<()>;
    fn remove_document(&mut self, path: &str) -> Result<()>;
    fn clear(&mut self) -> Result<()>;
    fn commit(&mut self) -> Result<()>;
    fn search(&self, query: &AlgorithmQuery) -> Result<Vec<AlgorithmHit>>;
}
```

### The Intermediate Representation: `ParsedArtifact`

AST and Markdown parsing in `groundcontrol-core::engine::indexer` produces an immutable, pure CPU extraction intermediate representation: [`ParsedArtifact`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-common/src/types/artifact.rs).

`ParsedArtifact` bundles:
- `path`: Normalized relative file path.
- `content`: Full source text.
- `modality`: `Doc` or `Code`.
- `chunks`: List of AST code blocks or Markdown sections with line ranges and scopes.
- `symbols`: Extracted symbol definitions (classes, functions, traits).
- `grammar_semantics`: Deep syntactic metadata (weighted tokens, grammar transitions, dataflow paths/sinks).
- `frontmatter_edges`: Statically extracted documentation edges and wikilinks.

---

## 2. Decomposed Algorithm Components

Algorithms are strictly organized into dedicated subdirectories under [`crates/groundcontrol-core/src/algorithm/`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/):

Each directory contains:
- `mod.rs`: Struct definition and [`RetrievalAlgorithm`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-common/src/ports/algorithm.rs) implementation.
- `types.rs`: Isolated configuration and query types.
- `tests.rs`: Comprehensive unit tests exercising indexing, query evaluation, and removal.

| Component | Path | Description |
|---|---|---|
| **BM25** | [`crates/groundcontrol-core/src/algorithm/bm25/`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/bm25/) | Lexical keyword indexer backed by Tantivy Okapi BM25 schema (`doc_code_schema`). |
| **Binary** | [`crates/groundcontrol-core/src/algorithm/binary/`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binary/) | Sub-millisecond algorithmic semantic search backed by `BinarySearchIndex` (256-bit Hamming projection over grammar tokens). |
| **BinaryV2** | [`crates/groundcontrol-core/src/algorithm/binaryv2/`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/) | Enhanced 4-channel 256-bit Hamming retrieval with code-agnostic subword tokenization, unsupervised Reflective Random Indexing (RRI), and AST context. |
| **Graph** | [`crates/groundcontrol-core/src/algorithm/graph/`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/graph/) | Structural AST relationship indexer (`calls`, `defines`, `imports`, `implements`) and doc wikilink topology backed by Petgraph. |
| **Dense** | [`crates/groundcontrol-core/src/algorithm/dense/`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/dense/) | Dense vector indexing (`jina-code-768d`) backed by HNSW vector store and hardware-accelerated DirectML/ONNX inference. |
| **Composite** | [`crates/groundcontrol-core/src/algorithm/composite/`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/composite/) | Multi-algorithm orchestrator fusing rankings across sub-algorithms via Reciprocal Rank Fusion (RRF) with parallel evaluation. |
| **Eval** | [`crates/groundcontrol-core/src/algorithm/eval/`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/eval/) | In-process evaluation substrate (`AlgorithmicIndex`, `AlgoHit`, query sanitization, and isolated algorithm runners). |

---

## 3. Engine Lifecycle Broadcasting

The [`Engine`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/engine/mod.rs) maintains concrete algorithms and coordinates their lifecycles:

* **Artifact Broadcasting** (`broadcast_artifact`): When a file is indexed or modified, the engine extracts a `ParsedArtifact` once, persists file/chunk records into SQLite, and broadcasts the artifact to all active algorithms (`bm25`, `binary`, `graph`, and `dense`).
* **Document Removal** (`remove_artifact`): Broadcasts `remove_document(path)` across all indices.
* **Commit** (`commit_algorithms`): Coordinates two-phase commit across SQLite, Tantivy, and writes serialized state (`graph.bin`, `vectors.bin`, `fingerprints.bin`) to disk.
* **Algorithm Registry** (`algorithm(name)`): Enables runtime dynamic lookup and programmatic dispatch by string handle (e.g. for ablation runs).

---

## 4. Evaluation via `groundtruth` Sister Repository

The decoupled architecture directly empowers the sister repository [`groundtruth`](file:///c:/dev/semantic/groundtruth).
Via `groundtruth-harness::backend::algo::AlgoBackend`, the evaluation suite interacts directly with `groundcontrol-core::algorithm::eval` without booting full MCP daemon processes, measuring raw p50/p90/p99 query latencies and ablation metrics (BM25 vs Binary vs Dense vs PPR) in microsecond isolation.
