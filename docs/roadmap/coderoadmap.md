# Codebase Semantic Indexing & Cross-Modal Retrieval Roadmap (`CODEROADMAP.md`)

This roadmap defines the architectural specification, academic foundations, tooling evaluation, and phased engineering plan for integrating **polyglot codebases** into the **Enterprise Semantic MCP** engine (`groundcontrol-core`, `groundcontrol-common`, `groundcontrol-mcp`).

---

## 1. Executive Summary & Vision

The goal is to expand the engine from indexing Markdown documentation vaults to unifying **semi-structured natural-language documentation** (ADRs, RFCs, design docs, Obsidian notes) and **multi-language source code repositories** (Rust, TypeScript/JavaScript, Python, Go, C/C++, Java, etc.) within a **single, unified hybrid retrieval graph**.

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                   Unified Documentation & Code Knowledge Engine                  │
├──────────────────────────────────────────────────────────────────────────────────┤
│                                                                                  │
│   [Document: ADR-004] ──────(implements)─────► [CodeFile: src/search/engine.rs]  │
│           │                                                    │                 │
│      (supersedes)                                          (defines)             │
│           ▼                                                    ▼                 │
│   [Document: ADR-001]                               [CodeSymbol: SearchEngine]   │
│                                                                │                 │
│                                                             (calls)              │
│                                                                ▼                 │
│                                                      [CodeSymbol: rrf_fuse]      │
│                                                                                  │
├──────────────────────────────────────────────────────────────────────────────────┤
│    Modality 1: Tantivy BM25 (Exact Identifiers & Text)                           │
│    Modality 2: fastembed BGE-small / HNSW (Dense Cross-Modal Embeddings)         │
│    Modality 3: Petgraph Directed Typed Graph (Call, Import & Lineage Hops)       │
│    Modality 4: Multi-Way Reciprocal Rank Fusion (RRF) with Modal Scoring         │
└──────────────────────────────────────────────────────────────────────────────────┘
```

### Core Invariants Preserved
1. **Source of Truth**: Plain text files on disk (`.md`, `.rs`, `.py`, `.ts`, etc.) remain authoritative. All indices (Tantivy, HNSW, SQLite, Petgraph) are derived, durable, and disposable.
2. **Pure Rust Runtime**: Memory-safe, `#![forbid(unsafe_code)]` at our crate boundary, zero unvetted dependencies, and strict `cargo-deny` compliance (MIT / Apache-2.0).
3. **Continuous Hybrid Retrieval**: Retains the 4-modality retrieval pipeline (BM25 + Vector + Graph Traversal + RRF) while adding deterministic AST/graph lookup tools.
4. **Principle 3 Crystallization**: Extends knowledge crystallization to support bidirectional lineage between design decisions and source code entities.

---

## 2. Comprehensive Literature Review & Academic State-of-the-Art

### 2.1 AST-Aware Structural Chunking (`cAST` Pattern)
Traditional text chunkers (fixed token/character windows) cause severe **context starvation** and **syntax fragmentation** when applied to code, splitting functions mid-statement and discarding vital scope headers.

* **cAST: High-Density Structural Chunking for Code RAG (2025)**:
  Demonstrated that recursive AST-guided decomposition using Tree-sitter achieves an 18–35% relative gain in Pass@1 on code comprehension and SWE-bench benchmarks. cAST ensures that:
  - Every chunk represents a complete, syntactically valid AST node (function, method, class, interface, module block).
  - Docstrings (`///`, `/** */`, `"""`) stay bound to their associated symbol definitions.
  - Sibling statements below token thresholds are merged, and oversized functions are partitioned only at inner logical block boundaries (`match`, `if-else`, loop blocks).
* **RepoCoder (Zhang et al., 2023) & RepoBench (Liu et al., 2023)**:
  Proved that prepending **AST Scope Breadcrumbs** (e.g., `// Scope: crate::search::Engine > search_hybrid`) directly into chunk text bridges lexical and conceptual gaps for both sparse BM25 and dense bi-encoders.
* **CoIR: Code Information Retrieval Benchmark (Li et al., 2024)**:
  Evaluated code retrieval across 10 distinct datasets and 8 tasks. Core empirical finding: Hybrid sparse-dense retrieval with RRF significantly outperforms either pure dense embeddings or lexical BM25 alone. Sparse BM25 excels at exact symbol/identifier queries, while dense bi-encoders capture natural-language semantic intent.

### 2.2 Graph-Based Code Indexing & Semantic Navigation
* **Scope Graphs & Stack Graphs (Creager et al., GitHub / OOPSLA 2021)**:
  Formalized language-agnostic name resolution by mapping source code to graph-based scope structures using Tree-sitter AST queries without requiring full compilation or type checking. Enabled zero-build jump-to-definition and reference resolution across files.
* **RepoGraph (2025) & RANGER (2025/2026)**:
  Constructed multi-relational code knowledge graphs (nodes: files, classes, methods, variables; edges: `calls`, `defines`, `imports`, `contains`). Used $k$-hop ego-network retrieval seeded by hybrid search to give LLMs structural context that prevents reasoning errors on cross-file dependencies.
* **Aider’s Repo Map (Gauthier, 2023–2025)**:
  Industrial state-of-the-art for lightweight repository mapping. Uses Tree-sitter to extract definitions and identifier references into a bipartite graph, applies **PageRank** to rank the most architecturally central symbols, and packs high-ranking signatures into a compact structural map.
* **Source Code Intelligence Protocol (SCIP / LSIF - Sourcegraph)**:
  Defines an open, Protobuf-based index format for compiler-exact symbol definitions, occurrences, relationships, and docstrings.

---

## 3. Rust Tooling Ecosystem Evaluation

| Crate / Tool | License | Multi-Language | Role & Capability | Assessment & Decision for `groundcontrol` |
| :--- | :--- | :--- | :--- | :--- |
| **`tree-sitter`** (v0.22+) | MIT | Yes (100+ langs) | Fast incremental C-CST parser with safe Rust bindings | **Core Substrate**: Universal parser for syntax tree generation. |
| **`tree-sitter-language-pack`** | MIT / Apache | Yes (370+ langs) | Bundled pre-compiled grammars for instant polyglot support | **Recommended**: Eliminates managing individual grammar crates in `Cargo.toml`. |
| **`tree-sitter-tags`** | MIT | Yes (Polyglot) | Query-based extraction of symbol definitions, refs, and docstrings | **Recommended**: High-performance extraction of symbol tables and doc comments without compiler overhead. |
| **`ast-grep-core`** | MIT | Yes (Polyglot) | Structural AST pattern search and rewrite engine | **Alternative**: Useful for custom AST pattern extraction rules. |
| **`stack-graphs`** | MIT / Apache | Yes (Polyglot) | Incremental scope-graph name resolution | **Reference Only**: Upstream archived late 2025; adopt lightweight scope resolution directly in Petgraph. |
| **`scip`** | Apache-2.0 | Yes (Polyglot via CLI) | Protobuf parser for compiler-generated code indexes | **Optional Phase 4**: Ingests compiler-precise `.scip` files if pre-generated in CI. |
| **`petgraph`** | MIT / Apache | N/A (Graph Engine) | In-memory directed typed graph store and algorithms | **Core Substrate**: Already integrated in `groundcontrol-core`; houses code and doc edges. |

---

## 4. Benchmark & Comparative Analysis: `codebase-memory-mcp`

The open-source **`DeusData/codebase-memory-mcp`** represents an industry baseline for code-focused MCP servers. Below is a head-to-head architectural comparison:

| Dimension | `codebase-memory-mcp` | Our Unified `groundcontrol-core` Architecture |
| :--- | :--- | :--- |
| **Core Philosophy** | **Code-only structural property graph** | **Unified Cross-Modal Doc + Code Knowledge Engine** |
| **Implementation Language** | Static C binary | **Pure Rust** (`groundcontrol-core`, `groundcontrol-mcp`, `#![forbid(unsafe_code)]`) |
| **Retrieval Mechanism** | **Discrete graph queries** (Cypher-like queries, caller/callee traces) | **4-Modality Continuous Hybrid Retrieval** (BM25 + Dense Vector + Graph Proximity + RRF) |
| **Documentation Handling** | Basic ADR records in SQLite | **Full Markdown Vault Indexing** (ADRs, RFCs, wikilinks `[[...]]`, tags, templates, Principle 3 crystallization) |
| **Cross-Modal Lineage** | Explicit parameter links | **Native Graph Lineage** (`implements`, `documents`, `supersedes`) bridging docs and code |
| **Vector Search** | Secondary / None | **First-Class HNSW Vectors** (fastembed BGE-small with AST breadcrumbs and max-pooling) |
| **Full-Text Lexical Search** | SQLite `LIKE` / basic FTS | **Tantivy Okapi BM25** with custom code/doc tokenizers |
| **Graph Storage** | Relational tables in SQLite | **In-memory Petgraph** (`graph.bin` via postcard) + SQLite metadata catalog |
| **Agent Experience** | Multi-hop tool exploration | **Single-shot hybrid retrieval** + deterministic navigation tools |

### Key Ideas Adopted from `codebase-memory-mcp`:
1. **Lightweight "Hybrid LSP" Import Resolution**: Resolving `import`/`use` statements across the SQLite symbol table to connect cross-file `calls` and `implements` edges without compiler passes.
2. **Community Detection (Louvain / Infomap)**: Clustering symbols in Petgraph to generate architectural module summaries automatically.
3. **Targeted Structural MCP Tools**: Exposing `get_symbol_definition`, `find_callers`, and `get_module_graph` alongside continuous hybrid search.

> 📄 **Detailed Analysis**: For the full 162-grammar audit, missing language breakdown, and LSP feasibility analysis, see [RFC: Polyglot Tree-sitter Grammar Expansion & LSP Integration Analysis](file:///c:/dev/semantic/groundcontrol/docs/RFC-treesitter-expansion-and-lsp-analysis.md).

---

## 5. Architectural Specification & Data Model

### 5.1 Unified Entity Discrimination (`groundcontrol-common`)
Every indexed item is tagged with an `EntityKind` to prevent index pollution and enable precise filtering:

```rust
/// Discriminates between documentation notes and polyglot source code entities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// Markdown documentation note, RFC, or ADR.
    Documentation,
    /// Whole source code file (e.g. `src/engine.rs`).
    CodeFile { 
        language: String 
    },
    /// Distinct code symbol (function, struct, class, trait, interface).
    CodeSymbol {
        language: String,
        symbol_type: CodeSymbolType, // Struct, Function, Trait, Enum, Method, Class
        scope_path: String,          // e.g. "kb_core::search::SearchEngine"
        signature: String,           // e.g. "pub fn search_hybrid(&self, query: &str)"
    },
    /// Syntactically coherent AST chunk for vector/BM25 indexing.
    CodeChunk {
        language: String,
        scope_path: String,
        start_line: usize,
        end_line: usize,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeSymbolType {
    Function,
    Method,
    Struct,
    Class,
    Trait,
    Interface,
    Enum,
    Module,
    Constant,
    TypeAlias,
}
```

### 5.2 Extended Graph Edge Schema (`groundcontrol.toml` & `groundcontrol-core`)
The graph engine is extended with code-specific and cross-modal relationship types:

| Edge Type | Source Node | Target Node | Default Weight | Description |
| :--- | :--- | :--- | :--- | :--- |
| `defines` | `CodeFile` | `CodeSymbol` | 1.0 | File declares the symbol. |
| `imports` | `CodeFile` | `CodeFile` / `Module` | 0.6 | File imports another module/file. |
| `calls` | `CodeSymbol` | `CodeSymbol` | 0.8 | Function/method invokes another symbol. |
| `implements_trait` | `CodeSymbol` (Struct/Class) | `CodeSymbol` (Trait/Interface) | 0.9 | Type implements an interface/trait. |
| `documents` | `Document` (Doc/ADR) | `CodeFile` / `CodeSymbol` | 1.0 | Markdown document specifies or documents code. |
| `implements_adr` | `CodeFile` / `CodeSymbol` | `Document` (ADR) | 1.0 | Code entity implements an architecture decision. |

---

## 6. AST-Aware Code Chunking Engine (`CodeChunker`)

```
Source Code (.rs, .py, .ts, .go, .java)
   │
   ▼
[Tree-sitter Parser] ──► Concrete Syntax Tree (CST)
   │
   ▼
[AST Traversal & Tag Extraction]
   ├─ Module Headers & Import Blocks
   ├─ Types, Structs, Classes & Trait Declarations
   └─ Functions & Methods (with bound docstrings `///` or `"""`)
   │
   ▼
[Scope Breadcrumb Enrichment]
   Prepends: "// Scope: crate::search::SearchEngine > search_hybrid\n// Language: rust\n"
   │
   ▼
[AST-Aligned Chunks] (Syntactically complete, byte-offset tracked, ready for Tantivy & HNSW)
```

---

## 7. Multi-Modal Hybrid Search & Query Routing

```
                           User Query
                                │
                ┌───────────────┴───────────────┐
                ▼                               ▼
      [Natural Language Query]         [Symbol / Code Query]
    "how is RRF score calculated?"    "pub fn rrf_fuse doc_rank"
                │                               │
                ▼                               ▼
       [Boost Documentation]           [Boost Code Chunks]
                │                               │
                └───────────────┬───────────────┘
                                ▼
          [4-Modality Hybrid Retrieval (BM25 + Vector + Graph)]
                                │
                                ▼
           [Cross-Modal Lineage & Graph Link Expansion]
         (Doc -> Implemented Code / Code -> Explaining Docs)
                                │
                                ▼
                    [Reciprocal Rank Fusion]
```

### Search Modes Supported:
1. **Faceted / Filtered Search**:
   - `search(query: "...", filter: { entity_kind: ["documentation"] })` $\rightarrow$ Docs only.
   - `search(query: "...", filter: { entity_kind: ["code_symbol", "code_chunk"], language: ["rust"] })` $\rightarrow$ Rust code only.
2. **Unified Cross-Modal Retrieval (Default)**:
   - Evaluates BM25 and Dense Vector against all chunks (docs + code).
   - Seeds graph traversal from top hits:
     - Top Doc hit $\rightarrow$ traverses `implements` $\rightarrow$ returns implementing code chunks.
     - Top Code hit $\rightarrow$ traverses `documented_by` $\rightarrow$ returns explaining ADRs/RFCs.
   - RRF fuses scores across text, vector, and graph proximity with lineage annotations.

---

## 8. Phased Implementation Roadmap

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            Engineering Roadmap                              │
├─────────────────────────────────────────────────────────────────────────────┤
│  Phase 1: Polyglot Parsing & AST-Aware Semantic Chunker                     │
│  Phase 2: Code Graph Extractor & Lightweight Import Resolver                │
│  Phase 3: Multi-Modal Search & Query Discrimination Engine                  │
│  Phase 4: Specialized Structural MCP Tools & Architecture Overview         │
│  Phase 5: Principle 3 Cross-Modal Knowledge Crystallization                 │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Phase 1: Polyglot Parsing & AST-Aware Semantic Chunker
* **Crates Impacted**: `groundcontrol-common`, `groundcontrol-core`
* **Deliverables**:
  - Add `tree-sitter` and `tree-sitter-language-pack` to `Cargo.toml`.
  - Implement `CodeChunker` in `groundcontrol-core/src/parser/code/chunker.rs`.
  - Support top 6 languages: Rust, TypeScript/JavaScript, Python, Go, Java, C/C++.
  - Inject AST scope breadcrumbs into chunk text for Tantivy and fastembed embedding passes.
* **Verification**: Unit tests validating that AST chunks never split functions mid-expression and docstrings remain bound to signatures.

### Phase 2: Code Graph Extractor & Lightweight Import Resolver
* **Crates Impacted**: `groundcontrol-core`
* **Deliverables**:
  - Implement `CodeGraphExtractor` in `groundcontrol-core/src/graph/code.rs` using `tree-sitter-tags`.
  - Extract `defines`, `imports`, and `calls` relationships.
  - Implement a lightweight import resolver pass across SQLite symbol tables to connect cross-file call sites.
  - Ingest code nodes and edges into `petgraph` (`graph.bin`).
* **Verification**: Integration tests confirming cross-file graph traversal from caller function to callee function in a multi-file project.

### Phase 3: Multi-Modal Search & Query Discrimination Engine
* **Crates Impacted**: `groundcontrol-core`, `groundcontrol-mcp`
* **Deliverables**:
  - Add `EntityKind` filtering to Tantivy index schema and HNSW metadata.
  - Update `SearchEngine` to perform cross-modal seed-then-traverse graph expansion.
  - Update `groundcontrol-mcp` search tool parameters: `query`, `entity_types`, `languages`, `depth`.
* **Verification**: Benchmark evaluation verifying that natural language queries retrieve documentation while surfacing relevant code via graph hops.

### Phase 4: Structural MCP Tools & Architecture Overview
* **Crates Impacted**: `groundcontrol-mcp`, `groundcontrol-core`
* **Deliverables**:
  - Add deterministic structural tools to MCP server:
    - `get_symbol_definition(symbol_path)`
    - `find_callers(symbol_name, max_depth)`
    - `get_module_graph(module_path)`
  - Implement Louvain community detection in `groundcontrol-core` to generate automated architectural module summaries.
* **Verification**: End-to-end MCP JSON-RPC test suite for all new structural tools.

### Phase 5: Principle 3 Cross-Modal Knowledge Crystallization
* **Crates Impacted**: `groundcontrol-core`, `groundcontrol-mcp`
* **Deliverables**:
  - Extend `promote_concept` tool to accept code symbol links and synthesize `implements`/`documents` lineage edges.
  - Add automated **Code Drift Detection**: scan indexed code to alert when an ADR references deprecated or renamed symbols/functions.
* **Verification**: Crystallization benchmark testing lineage integrity between ADR notes and source code.

---

## 9. Delivered: Multi-Corpus, Cross-Modal & Progressive-Disclosure Enhancements

The multi-corpus upgrade (branch `feature/codebase-semantic-indexing`) extended the code
roadmap above with cross-cutting capabilities that apply to both code and docs:

* **Multi-corpus from one MCP.** `CorpusManager` serves N roots; read tools take
  `corpus`/`corpora` and cross-corpus queries RRF-merge with per-hit corpus tagging.
* **Cross-corpus symbol/edge linking.** A doc's `implements`/`documents` target (or a code
  import) resolves to a code symbol in another corpus by qualified name — only on a unique
  match, never producing a false edge — carrying a `ResolutionConfidence` band.
* **Import-resolution confidence bands.** `calls` edges are tagged `High` (unique in-file /
  unique in-workspace), `Medium` (same-directory disambiguation), or `Speculative`
  (first-of-many); `imports` are `Speculative`; `defines`/`implements_trait` are `High`.
  `find_callers` surfaces the band per caller.
* **Bi-modal search.** `modality` = `docs`|`code`|`both` threads through BM25 (indexed
  field), vector (post-filter), and graph (code-path classifier), consistently in the fused
  hybrid path.
* **Progressive disclosure.** `search` (handles) → `get_snippet` (one symbol/chunk, bounded,
  neighbors) → `read_note`/`read_code_file`/`read_multiple` (whole file), encoded in tool
  descriptions.
* **Consolidated surface + profiles.** `search` (`mode`) and `status` (`scope`) replace the
  former per-mode/per-status families; `--profile scout|analysis|all` gates `tools/list`.
* **Leiden community refinement.** `detect_communities_leiden` refines the Louvain partition
  so every community is internally connected (deterministic); `get_architecture` uses it,
  `graph_communities` accepts `algorithm=louvain` for the raw partition.
* **`check_index_coverage`.** Reports index/parse coverage for given paths or prefixes
  (distinct from the query-driven `coverage_report`).

---

## 10. Language Grammar & Code Intelligence Roadmap (Tiers 1, 2, 3)

### 10.1 Tier 1: Crates.io Native Grammars & In-Engine Intelligence (Delivered)
Branch `feature/treesitter-expansion-and-lsp` expands code intelligence to **47 programming and configuration languages** in 100% pure Rust without external compiler daemons:

| Domain | Languages Supported (47 Total) |
|---|---|
| **Core Systems & Backend** | Rust, C, C++, Go, Java, C#, Zig, D, CUDA |
| **Web & Scripting** | TypeScript, TSX, JavaScript, Python, Ruby, PHP, Lua, Bash, PowerShell |
| **Functional & Emerging** | Elixir, OCaml, Haskell, Scala, Kotlin, Swift, Dart, Julia, Gleam, R |
| **Hardware & Formal** | Verilog / SystemVerilog, TLA+ |
| **Config, Build & Cloud** | HCL / Terraform, Azure Bicep, Nix, Starlark / Bazel, CMake, Make, Dockerfile, YAML, TOML, JSON |
| **Schemas, Data & Web Tech** | SQL, Protocol Buffers, Solidity, GraphQL, HTML, CSS, WGSL (WebGPU) |

#### Key Capabilities Delivered
1. **Declarative `LanguageSpec` Architecture**:
   Unified declarative table in `crates/groundcontrol-core/src/parser/code/spec.rs` mapping AST node kinds to `CodeSymbolType`, call expressions, and scope breadcrumbs across all 47 languages.
2. **In-Engine Pure-Rust "Hybrid LSP"**:
   Zero-daemon static analysis engine with `TypeEnvironment` variable tracking and receiver method call disambiguation (`x.method()` $\to$ `Type::method`), upgrading graph call edges from `Speculative` to `ResolutionConfidence::High` in sub-millisecond time.
3. **Offline SCIP Protobuf Index Ingestion**:
   Direct ingestion of compiler-grade `.scip` dumps (generated via `scip-rust`, `scip-typescript`, `scip-python`, etc.) via `Engine::ingest_scip` and CLI `--scip <PATH>`, importing 100% compiler-accurate symbol definitions, calls, and references in <200ms.
4. **Tree-Sitter 0.25 Modernization**:
   Upgraded tree-sitter core runtime to 0.25 to support modern ABI 14 and ABI 15 grammars while preserving `#![forbid(unsafe_code)]` and zero compiler warnings.

---

### 10.2 Tier 2: Upstream C/C++ Tree-Sitter Grammars via `cc` in `build.rs` (Roadmap)
For languages lacking maintained pure-Rust crates on crates.io, Tier 2 will vendor upstream C grammars directly:

* **Target Languages**:
  Clojure, Nim, Odin, Fortran, COBOL, Ada, Apex, Pascal, Perl, Erlang, Fish, V, Reason, Scheme, Common Lisp, Racket, Standard ML.
* **Compilation Mechanism**:
  Vendor `parser.c` and `scanner.c` into `vendored/grammars/<lang>/` and compile via `cc::Build` in `crates/groundcontrol-core/build.rs`.
* **Safety & Invariant Preservation**:
  Maintain `#![forbid(unsafe_code)]` at our crate boundary. Isolate raw `extern "C"` FFI declarations inside a dedicated FFI module that validates grammar ABI versions and produces safe `tree_sitter::Language` handles.
* **Activation Trigger**:
  Introduced when indexing enterprise repositories that rely on legacy mainframe (COBOL, Fortran, Ada), enterprise CRM (Apex), or specialized Lisp/ML stacks.

---

### 10.3 Tier 3: Compiler-Grade Code Intelligence & LSP Daemon Integrations (Roadmap)
Tier 3 addresses scenarios where heuristic and syntactic extraction is insufficient and exact macro expansions or cross-crate monomorphizations are required:

1. **Deep SCIP Ecosystem Automation**:
   - **Automated CI / Indexing Hooks**: Provide automated toolchain wrappers that invoke `scip-rust`, `scip-typescript`, `scip-python`, or `scip-clang` prior to indexing when compiler environments are present.
   - **Incremental SCIP Diffs**: Support incremental patching of the SQLite symbol table and Petgraph graph using partial SCIP documents without re-ingesting whole workspaces.
   - **Cross-Corpus SCIP Namespace Resolution**: Map SCIP global symbol identifiers across multiple corpus roots to enable compiler-exact cross-corpus symbol navigation.
2. **External LSP Daemon Socket Integration (Zero-Daemon Overhead)**:
   - **No Embedded Daemon Supervision**: Explicitly reject running live LSP daemons (`rust-analyzer`, `pyright`, `gopls`) as child processes within the MCP server process, avoiding 2–6 GB memory overhead, 15–90s cold starts, and host environment failures.
   - **Opt-in Socket Connector**: Provide an opt-in client (`--lsp-socket <lang>:<addr>`) that connects to an *already-running* IDE or editor language server socket over standard JSON-RPC. Allows querying real-time hover documentation and call hierarchies on demand while keeping `groundcontrol` ultra-lightweight and immediately available.

---

### 10.4 High-Throughput cAST Optimization & Centrality-Guided Anchoring (Roadmap)
Identified during the 177k-file multi-corpus benchmark (Kubernetes, Rust, TypeScript) to address dense embedding compute bottlenecks on large codebases and non-Tensor-Core hardware (Pascal, APUs, CPUs):

1. **Two-Pass Centrality-Guided Anchoring (PageRank / Degree Filter)**:
   - *Architecture*: Parse ASTs and construct the Petgraph code graph upfront in Pass 1 (pure Rust, sub-minute execution). In Pass 2, calculate in-degree and PageRank on all extracted symbols.
   - *Policy*: Only promote symbols with $\text{in\_degree} \ge K$ or top-level exported trait/interface definitions to `ChunkEmbedPolicy::Anchor`.
   - *Impact*: Reduces anchor vector density from ~5.6 anchors/file down to ~0.8 anchors/file (**5x–10x reduction in neural embedding volume**), while preserving full multi-hop structural traversal via `graph_match`.

2. **cAST Skeleton & Signature-Only Embedding**:
   - Strip function/method bodies prior to ONNX tokenization, embedding only `signature + docstring + scope breadcrumbs`.
   - Compresses sequence length from ~350 tokens down to ~45 tokens, accelerating quadratic transformer attention ($\mathcal{O}(L^2)$) by **4x–5x** on the GPU while retaining full bodies in Tantivy BM25.

3. **File-Level & Module Outline Anchors**:
   - Synthesize a single outline anchor chunk per source file (file docstring + top-level symbol outline) instead of vectorizing every declared symbol.
   - Restricts vector index cardinality to exactly 1 vector per file, slashing cold-indexing neural forward passes by 80%+.

---

### 10.5 Storage Footprint Optimization: Zero-Copy File-Offset Architecture & Binary Vectors (Delivered)
*Authoritative Implementation Doc*: [[docs/architecture/implementation/zero-copy-storage]]
*Reference RFC*: [[docs/roadmap/RFC-zero-copy-file-offsets-and-binary-vectors]]

Identified during disk footprint profiling on the 14k-file Kubernetes index run (where the `.index/` footprint reached 2.15 GB across SQLite, Tantivy, and JSON vectors):

1. **Binary Vector Serialization (`vectors.bin`)**:
   - *Problem*: `vectors.json` stores 768-dim `f32` vectors as human-readable ASCII arrays (`[0.02341, ...]`), consuming ~6.8 KB per vector instead of 3.0 KB in raw binary (**~2.2x serialization bloat**; 555 MB for 81k vectors).
   - *Solution*: Serialize vectors via `safetensors` or raw binary memory-mappable slices (`vectors.bin`).
   - *Impact*: Cuts vector file size from 555 MB to **249 MB immediately** (55% reduction), while eliminating JSON parse overhead at startup.

2. **Zero-Copy File Pointers (Principle 1 Invariant Alignment)**:
   - *Problem*: Code text is currently duplicated three times:
     1. Raw source files on disk (`src/**/*.rs`, `pkg/**/*.go`).
     2. Stored uncompressed in SQLite `chunks.text` (consuming ~60% of `meta.db`).
     3. Stored compressed in Tantivy doc store (`body: TEXT | STORED`).
   - *Solution*: Align strictly with Principle 1 (*"Markdown/source is authoritative ground truth"*).
     - Store only `(doc_path, start_byte, end_byte, start_line, end_line)` in SQLite `chunks`.
     - Configure Tantivy `body` as `TEXT` only (indexed in inverted index, but NOT `STORED` in `.store` files).
     - All snippet fetches (`search`, `get_snippet`, `read_file`) read direct byte-range slices from the authoritative source file on disk. Modern OS page cache keeps hot working files in memory (<50µs read latency).
   - *Impact*: Slashes SQLite `meta.db` from 1.36 GB down to **~350 MB** and shrinks Tantivy `.store` files by **~60%**, dropping total index footprint on 14k files from 2.15 GB down to **<700 MB** (reducing overall expansion from 7.2x to ~2.3x).

---

### 10.6 Token-Optimal Agent Responses: Lean Multiline Text Emission (Delivered)
*Authoritative ADR*: [[docs/architecture/adr/adr-020-lean-multiline-text-emission]]  
*Authoritative RFC*: [[docs/roadmap/RFC-lean-multiline-text-emission]]  
*Implementation*: [`crates/groundcontrol-mcp/src/format/lean.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/format/lean.rs)

Identified during LLM agent context profiling and benchmark evaluation against `codebase-memory-mcp`'s `compact_out` architecture:

1. **The JSON Context Tax**:
   - *Problem*: In the MCP specification, tool results are delivered as raw text (`content[0].text`). Serializing complex AST structures to JSON forces coding agents to ingest repetitive schema keys (`"node":`, `"rel":`, `"file":`, `"line":`, `"hop":`, `"branches":`), quotes, and closing delimiter cascades (`}]}}`). On deep multi-hop traversals, **50% to 70% of response tokens are purely structural boilerplate**.
   - *Solution*: Emit indented, human-and-LLM-readable ASCII Cypher trees for `graph_match`, metadata-headed Markdown code blocks for `search` and `get_snippet`, and line-numbered text blocks for `read_file`. Default to lean multiline text across stdio MCP transport with `format="json"` opt-in.
   - *Impact*: Reduces context consumption from **~420 tokens down to ~130 tokens per 14-node tree (~69% reduction)**, slashing attention noise and maximizing available reasoning context.

2. **Unified Progressive Disclosure Text Formats Across Turns 1, 2a, 2b, and 3**:
   - *Turn 1 (`search`)*: Partitioned Markdown sections with hit ranking, non-zero score breakdowns, affordance degree counts, inlined Turn 1 source snippets, and next-turn `get_snippet` / `graph_match` scents.
   - *Turn 2a (`get_snippet`)*: Bounded source blocks with 1-based prefixed line numbers (`L<num>: `), docstrings in markdown blockquotes, grammar-driven incoming/outgoing relationships, and outbound navigation hints (`-> [T2b callers]`, `-> [T3 full file]`).
   - *Turn 2b (`graph_match`)*: 2-space indented Cypher ASCII trees with quantified blast radius summary header (`[direct: D, transitive: T, files: F, depth: H]`), cycle detection markers, and hub suppression annotations (`... (+N more)`).
   - *Turn 3 (`read_file`)*: Line-numbered markdown blocks for single files or batch arrays (`paths: [...]`) with zero JSON quote/newline escaping overhead.

---

### 10.7 Dedicated Data Science Benchmarking & Resource Profiling Harness (Delivered)
*Authoritative Concept Docs*: [[docs/concepts/search/benchmarking-harness]], [[docs/concepts/search/evaluation-methodology]]  
*Implementation*: [`crates/groundcontrol-bench/`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-bench) (`gc-bench`)

Engineered for empirical evaluation and ablation of all retrieval modes against ground-truth corpora without MCP JSON-RPC protocol overhead:

1. **Indexing Pipeline & Resource Profiling**:
   - Measures wall-clock stage timings: AST tree-sitter parsing, Tantivy BM25 postings, static SIF projections, 256-bit binary fingerprints, Petgraph AST edge resolution, and optional dense ONNX re-embedding.
   - Measures indexing throughput (documents/second, files/second) and tracks process memory via [`MemoryTracker`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-bench/src/profile/memory.rs) (peak RSS, memory delta).
   - Profiles storage footprint via [`DiskProfiler`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-bench/src/profile/disk.rs): SQLite `meta.db`, Tantivy `tantivy/`, binary `fingerprints.bin`, Petgraph `graph.bin`, vectors `vectors.bin`, text projections `projections/`, and index expansion ratios.
2. **Retrieval Algorithm Quality & Latency Ablation**:
   - Supports isolated and hybrid evaluations across `bm25`, `binary` (SIF+Hamming), `ppr` (HippoRAG diffusion), `fast` (3-way RRF), `semantic` (dense ONNX), and `full` (BM25+ONNX+Graph).
   - Computes standard IR metrics: Recall@K, Precision@K, MRR@K, NDCG@K (with graded relevance), score separation, and latency percentiles (p50, p90, p95, p99, QPS).
   - Evaluates Turn-1 topological orientation metrics ([`IrEvaluator::evaluate_with_orientation`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-bench/src/metrics/ir.rs)): **Cluster Recall@K** and **Mean Hop Distance** ($\bar{H}_d$) to ground truth on Petgraph.
3. **Public Benchmark Ingestion & External Ground Truth**:
   - Native adapters ([`PublicBenchmarkAdapter`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-bench/src/dataset/adapters.rs)) and CLI command `gc-bench import` for:
     - **CodeSearchNet / AdvTest** (polyglot Go, Java, JS, Python function docstrings).
     - **RepoBench-R** (cross-file repository retrieval context).
     - **SWE-bench Lite** (git diff patch parsing for bug localization).
4. **Statistical Significance Testing**:
   - Hypothesis testing via [`SignificanceEvaluator`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-bench/src/metrics/significance.rs): paired Student's $t$-test and Wilcoxon signed-rank test across metric distributions.
5. **Multi-Format Publication Exporters**:
   - Exports GitHub markdown comparison tables (`report.md`), publication-ready LaTeX `booktabs` tables (`report.tex` via [`LatexReporter`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-bench/src/report/latex.rs)), machine-readable JSON (`report.json`), and tabular CSV (`report.csv`).

---

## 11. Upcoming Engineering Milestones

### 11.1 SOTA Code Retrieval & High-Throughput Semantic Bridging (Completed / Delivered)
*Authoritative RFC*: [[docs/roadmap/RFC-sota-code-retrieval-and-semantic-bridging]]  
*Status*: **Completed / Delivered**  
*Scope*: `groundcontrol-common`, `groundcontrol-core`, `groundcontrol-mcp`, `groundcontrol-cli`

Eliminated the 35–55 minute ONNX CPU embedding bottleneck on 100K+ file repositories without dedicated GPUs via a 4-pillar sub-minute retrieval engine:

1. **Sub-Minute CPU Semantic Bridging**:
   - **Static SIF Projections**: Smooth Inverse Frequency weighted embeddings over Tree-sitter code tokens and document text ([`SifEngine`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/search/sif.rs)), executing in **~10 seconds for 500,000 symbols** entirely on CPU with power-iteration 1st principal component removal.
   - **256-Bit Matryoshka Binary Embeddings (MRL)**: Sign-quantized binary fingerprints ([`BinaryFingerprint`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/types.rs), [`BinarySearchIndex`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/search/binary.rs)) requiring only **16 MB of total RAM** for 500k symbols, serialized via `postcard` to `.index/fingerprints.bin` and evaluated via single-cycle AVX2/AVX-512 `count_ones()` POPCOUNT (<1ms SIMD candidate scoring).
   - **AST Pattern Injection**: Pre-tokenized syntactic pattern tokens ([`extract_semantic_tokens`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/code/patterns.rs)) injected directly into Tantivy BM25 postings, bridging lexical-semantic synonym gaps at zero marginal CPU cost.
2. **Query-Time Personalized PageRank (HippoRAG Diffusion)**:
   - Eliminates graph edge bloat and pre-computation by executing 2-hop PPR random walks on demand across Petgraph at query time ([`personalized_pagerank`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/graph/diffusion.rs)).
   - Leaves Petgraph's topology completely clean (zero artificial `[:semantically_related]` edges), achieving sub-2ms diffusion without combinatorial path explosion.
3. **Fast Hybrid Search Mode**:
   - Exposed as `mode="fast"` across [`SearchService`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/search_service.rs), [`search_fast`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/search/mod.rs), [`search_explain_fast`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/search/mod.rs), and the MCP `search` tool in [`crates/groundcontrol-mcp/src/tools/mod.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/tools/mod.rs).

---

### 11.2 Pluggable Document Extractors & Derived Text Projections (Word, PDF, HTML) (Completed)
*Authoritative RFC*: [[docs/roadmap/RFC-document-extractors-and-projections]]  
*Status*: **Completed**  
*Scope*: `groundcontrol-common`, `groundcontrol-core`, `groundcontrol-mcp`, `groundcontrol-cli`

Expands `groundcontrol` beyond Markdown notes into polyglot document vaults while strictly preserving Non-Negotiable Invariant #1 (disk as authoritative ground truth) and the Zero-Copy File-Offset architecture:

1. **Corpus Modality Disambiguation**:
   - Deterministically separates code UI templates (`.html` in React/Vue/Go projects) from documentation articles (Sphinx/Doxygen/Confluence HTML exports).
   - Implemented via [`FileClassifier`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/index/classifier.rs) with `CorpusType` (`code_repo`, `doc_vault`, `mixed`), explicit `doc_patterns` (`docs/**`, `specs/**`, `wiki/**`), and content-based text-to-tag density heuristics.
   - Filters binary fixtures (`.pdf`, `.docx` in `tests/fixtures/`) from indexing unless explicitly opted into document roots.
2. **Derived Text Projections (DTP)**:
   - Stores disposable, deterministic, line-numbered text projections under `.index/projections/<path>.txt`.
   - The authoritative `.docx`, `.pdf`, or `.html` file on disk remains the sole source of truth; projections are 100% rebuildable upon index refresh.
   - Slices byte offsets (`start_byte..end_byte`) in [`fetch_chunk_text`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/engine.rs) and line ranges directly from the projected file for sub-millisecond [`get_snippet`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/tools/mod.rs) and [`read_file`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/tools/mod.rs) performance.
3. **100% Pure-Rust Ingestion Adapters**:
   - Managed via [`DocumentExtractorRegistry`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/document/mod.rs):
     - **Word (`.docx`)**: [`DocxExtractor`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/document/docx.rs) using `quick-xml` + `zip` streaming OpenXML parser with heading-style mapping and GFM table generation (<2ms per document, zero C runtime).
     - **PDF (`.pdf`)**: [`PdfExtractor`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/document/pdf.rs) using `lopdf` text-and-vector parser with `<!-- Page N -->` line anchors, annotation hyperlinks, and reading-order reconstruction.
     - **HTML (`.html`)**: [`HtmlDocExtractor`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/document/html.rs) using `scraper` with automatic chrome stripping (`<nav>`, `<header>`, `<footer>`, `<script>`, `<style>`) and semantic Markdown synthesis.
4. **Strict Read-Only Ingestion Boundary**:
   - [`write_note`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/tools/mod.rs) strictly rejects non-markdown document formats with `NotPermitted`. Principle 3 knowledge crystallization authors canonical Markdown notes linking to extracted documents via `derived_from: ["specs/architecture.docx"]`.

---

### 11.3 cAST Structural Signal Boosting & Partitioned Multi-Channel Hyperplanes (Proposed)
*Authoritative RFC*: [[docs/roadmap/RFC-cast-signal-boosting-and-partitioned-hyperplanes]]  
*Status*: **Proposed**  
*Scope*: `groundcontrol-core`, `groundcontrol-common`

Eliminates the 2D $\rightarrow$ 1D Bag-of-Words reduction loss and 64-bit quantization resolution blur in algorithmic binary embeddings via cAST concrete syntax tree metadata:

1. **Syntactic Role-Decorated Tokens**:
   - Tags extracted tokens with AST syntactic slots (`def:`, `param:`, `callee:`, `ret:`) to distinguish between caller/callee and subject/object roles without transformer attention.
2. **Tree-Depth Attenuation ($1/\sqrt{1 + \text{depth}}$)**:
   - Weights tokens by inverse square root of tree nesting depth, prioritizing top-level interface definitions and suppressing inner loop boilerplate.
3. **Data-Flow Edge Synthesis**:
   - Extracts definition-use chains (`flow:param->callee->return`) to capture behavioral execution paths across functions with disjoint vocabularies.
4. **Partitioned Multi-Channel Hyperplanes (24b / 24b / 16b)**:
   - Allocates the 64-bit signature across three orthogonal channels: Channel A (Lexical/Interface, 24 bits), Channel B (Data Flow/Calls, 24 bits), and Channel C (Control AST Shape, 16 bits). Enables channel-masked bitwise sweeps for behavioral clone detection.
5. **Deterministic Idiom Injection**:
   - Unifies cross-language patterns (`try/catch`, `if err != nil`, `match Err`) into canonical synthetic semantic tokens (`$sem:error_handler`).
