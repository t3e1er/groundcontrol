# groundcontrol — Gemini & Antigravity Steering Guide

> **Authoritative Source of Truth**: This steering document governs AI pair programming, architectural discipline, and Model Context Protocol (MCP) interactions for `groundcontrol`.

---

## 1. Product Context & Core Invariants

`groundcontrol` (`gc`) is an enterprise semantic **Model Context Protocol (MCP) server** for markdown knowledge bases and polyglot codebases. It provides AI agents with fast, minimal, high-signal context without file dumping or non-deterministic LLM entity extraction.

Written in 100% pure Rust (`unsafe_code = "forbid"`) for memory safety, zero C-runtime dependencies, and sub-millisecond graph and lexical retrieval.

### Non-Negotiable Invariants
1. **Markdown/source is authoritative ground truth**: Files on disk are king. All indices (Tantivy BM25, HNSW vectors, SQLite catalog, Petgraph) are derived, disposable, and 100% rebuildable. Never treat an index as canonical.
2. **Explicit graph topology, not LLM extraction**: Edges are generated deterministically from typed frontmatter fields, `#tags`, `[[wikilinks]]`, and AST code relations (`calls`, `defines`, `imports`, `implements`) — never from stochastic extraction pipelines.
3. **Pure Rust sub-millisecond speed**: Multi-hop graph traversal and hybrid ranking operate in real time (lexical p50 ~2.2ms, graph BFS ~1.8ms) with no perceptible agent lag.
4. **Multi-agent memory substrate**: A shared in-memory + on-disk semantic plane for specialized agent swarms (Scouts, Readers, Writers, Analysts).
5. **Never git push**: AI agents must NEVER run `git push` under any circumstances. Staging, branching, and committing locally are permitted when requested, but pushing to remote repositories is strictly reserved for the human developer.
6. **Exclusively use `.agents\mcp_config.json` for MCP install config**: AI agents must NEVER edit, rewrite, or populate global/machine MCP configuration files (e.g., `~/.gemini/antigravity-ide/mcp_config.json`, `~/.gemini/config/mcp_config.json`, or external IDE global settings). All MCP server configurations, daemon endpoints, arguments, or environment variables in this workspace must strictly and exclusively reside in `.agents\mcp_config.json`.

### Retrieval, Configuration & Multi-Corpus Architecture
- **Central vs Local Configuration Separation**:
  - *Central Machine Config* (`${GROUNDCONTROL_CACHE_DIR}/config.toml`): Daemon host/port, client authentication registry, GraphView telemetry relay, and persistent corpus registry. Lazily generated on first run with cryptographic keys via [`ensure_global_config`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/config.rs).
  - *Local Repository Config* (`<repo_root>/groundcontrol.toml`): Authoritative per-repo rules (`[docs.patterns]`, `[exclude.patterns]`, `[templates]`, `[chunking]`, `[graph]`). Initialized via `groundcontrol init` with automatic `.gitignore` importing.
  - *Zero-Config Repository Indexing*: Unconfigured repositories dynamically import local `.gitignore` rules in memory and index directly into central storage (`${GROUNDCONTROL_CACHE_DIR}/corpora/<name>/`) without polluting git working trees.
  - *Workspace MCP Configuration*: All agent MCP server configurations for this workspace strictly and exclusively live in `<repo_root>/.agents/mcp_config.json`. Never edit global user-profile configurations (`~/.gemini/antigravity-ide/mcp_config.json`, `~/.gemini/config/mcp_config.json`).
  - *Automated Agent Configuration*: `install.ps1`, `install.sh`, and `groundcontrol install -y` auto-detect installed coding agents and configure zero-arg MCP entries with optional auth tokens via [`run_install`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-cli/src/installer/mod.rs). For this repository, install/benchmark server entries (including head-to-head servers like `codebase-memory`) must always be configured in `.agents\mcp_config.json`.
- **4-Modality Hybrid Retrieval**: Fused via 3-way Reciprocal Rank Fusion (RRF) across Tantivy Okapi BM25, dense ONNX embeddings (`jina-embeddings-v2-base-code`, 768-dim), and Petgraph typed graph traversal.
- **Cross-Modal Linking**: Unifies documentation and polyglot source code (Rust, TS/JS, Python, Go, Java, C/C++) in a single graph.
- **Multi-Corpus Serving**: A central MCP process serves $N$ index roots via `CorpusManager`. Tools accept optional `corpus` or fan-out `corpora` (`["a", "b"]` or `"all"`).
- **Progressive Disclosure (3 Tiers)**:
  - *Tier 1*: `search` returns partitioned `docs` and `code` hits enriched with Turn 1 source snippets (`snippets: usize`, default 3) and graph affordances (`calls_in`, `calls_out`, `implements`, `imports`, `wikilinks_in`).
  - *Tier 2*: `get_snippet` fetches exactly one code symbol or doc chunk, bounded, with optional neighbor expansion.
  - *Tier 3*: `read_file` reads full file contents or line slices (`[start_line, end_line]`) only when exhaustive context is required.

---

## 2. Greenfield Engineering Principles

groundcontrol has no legacy external consumers to protect. Optimize for a clean, minimal, cohesive codebase.

### Simplification & Generalization Strategy (Prompt First)
- **Proactive Simplification**: Continuously seek to simplify architectures, collapse overlapping modes, prune bloated parameter surfaces, and generalize bespoke abstractions into clean, robust primitives.
- **User Consent Required**: Always present proposed simplifications and architectural reductions to the user and obtain explicit consent before performing major refactoring.
- **Clean Greenfield Pruning**: Once user consent is granted, eliminate obsolete code, flags, and types outright. Never leave dead code, deprecated aliases, or compatibility shims behind.

### No Backwards Compatibility
- **Never add compatibility shims**, deprecated tool names, aliased handlers, or "fallback to legacy behavior" logic.
- When changing a type, tool signature, argument, or on-disk index layout, **replace the old shape outright**.
- Indices are rebuildable and APIs are unversioned greenfield. Defaults exist for ergonomics, never for legacy emulation.

### No Dead Code, No Tech Debt
- Every function, struct, field, enum variant, and branch must be reachable and used. Delete unused code in the same change.
- Clippy runs with `-D warnings`. Never silence unused code warnings with blanket `#[allow(dead_code)]`.
- Do not leave TODO stubs, commented-out code, or duplicate code paths. Collapse duplicate paths immediately.

### Hexagonal Architecture (Ports & Adapters)
Every major concern is defined as a trait (**port**) in `groundcontrol-common::ports` or `groundcontrol-core`; concrete backends (**adapters**) implement them:
- **Major Ports**: `MetadataCatalog` (SQLite), `TextIndex` (Tantivy BM25), `VectorStore` (HNSW), `GraphStore` (Petgraph), `EmbeddingProvider` (ONNX), `SearchService` (multi-modal dispatch + RRF).
- **Encapsulation Barrier**: Adapters never leak backend types (`rusqlite::Connection`, `tantivy::*`, `hnsw_rs::*`, `petgraph::*`, `ort::*`) across ports. Port signatures use domain types from `groundcontrol-common` only.
- **Domain Decoupling**: `Engine` holds ports; it does not own concrete backends and does not expose concrete accessors. `groundcontrol-mcp` depends on ports, `SearchService`, and domain types, never core internals.
- **Composition Root**: `crates/groundcontrol-cli/src/main.rs` is the *only* place adapters are named, constructed, and injected via `CorpusManager` / engine builders.
- **Rust DI Policy**: Prefer generics with trait bounds on hot paths (zero-cost monomorphization). Use `Arc<dyn Trait>` only for runtime pluggable boundaries.

---

## 3. MCP Tool Surface (17 Tools) & Usage Directives

Authoritative tool registry: `crates/groundcontrol-mcp/src/tools/mod.rs`. Handlers are `ReadOnly(fn(&Engine, Value))` or `ReadWrite(fn(&mut Engine, Value))`.

### Registered Tool Inventory (17 Tools)

| Category | Count | Tools |
|---|---|---|
| **Read** | 3 | `read_file` (Tier 3 polymorphic path/paths batch with line slicing), `get_snippet` (Tier 2 symbol/chunk fetch + grammar-driven relationship handles + symbol definition lookup), `list_notes` (note catalog & single note frontmatter inspection) |
| **Search** | 2 | `search` (Tier 1 retrieval with Turn 1 hybrid snippets across docs & code via `snippets: usize`; `mode` = `hybrid` \| `bm25` \| `semantic` \| `graph` \| `explain`), `search_related` |
| **Graph** | 2 | `graph_match` (linear Cypher-Lite ASCII path query executed via pure in-memory Petgraph traversal with cycle guards), `graph_communities` (`algorithm` = `leiden` \| `louvain`, `view` = `architecture` \| `raw`) |
| **Write** | 3 | `write_note` (`mode` = `create` \| `overwrite` \| `append` \| `prepend`), `delete_note`, `move_note` (wikilink refactoring) |
| **Template / Validation** | 2 | `validate` (unified single note template check, corpus scan, and taxonomy check via `check_taxonomy`), `list_templates` |
| **System / Corpus** | 5 | `status` (unified multi-corpus overview or per-corpus stats, indexing, graph density, coverage via `scope`), `list_corpora`, `sync_corpus` (`mode` = `delta` \| `full` \| `reembed`), `index_corpus`, `unload_corpus` |

### Tool Profiles (`--profile`)
- **`scout`** (6 tools): Minimal retrieve/navigate set (`search`, `search_related`, `get_snippet`, `read_file`, `list_notes`, `status`).
- **`analysis`** (11 tools): `scout` + read-only graph (`graph_match`, `graph_communities`), validation (`validate`, `list_templates`), and `list_corpora`.
- **`all`** (17 tools): Full suite including mutating writes (`write_note`, `delete_note`, `move_note`, `sync_corpus`, `index_corpus`, `unload_corpus`).

### Agent Directives
1. **MCP Retrieval-First Invariant (No Direct File Dumps)**:
   - **Never** begin code/docs exploration, search, or architectural discovery with raw file reads (`view_file`), full file dumps, or directory-wide grep searches.
   - **Always** use `groundcontrol` MCP tools (`search`, `get_snippet`, `graph_match`, `search_related`) as the primary intake mechanism for high-signal, token-efficient context.
   - Direct file reads (`read_file` or native `view_file`) are strictly a **Tier 3 last resort**, permitted only when actively preparing a code edit or when exhaustive contiguous context is proven necessary after Tier 1 & 2 elaboration. Files on disk remain authoritative for applying modifications, but discovery must be mediated via MCP.
2. **Select optimal `search` mode & leverage Turn 1 snippets**:
   - `mode="hybrid"`: Default for broad exploratory queries. Respects full vs fast bimodality: fuses BM25 + dense ONNX embeddings + Petgraph for `docs`, and BM25 + 256-bit binary Hamming scan + Petgraph for `code` (3-way RRF).
   - `mode="fast"`: Pure CPU algorithmic search across both docs and code (Tantivy BM25 + 256-bit Hamming scan + HippoRAG PPR). Zero ONNX inference in <2ms.
   - `mode="bm25"`: Exact symbols, identifiers, struct names, error strings, verbatim tokens.
   - `mode="semantic"`: Conceptual similarity and abstract technical intentions across documentation.
   - `mode="graph"`: Typed graph traversal; filter by `edge_types` or `edge_class` (`code`, `structural`, `semantic`, `crossmodal`, `hybrid`).
   - `mode="explain"`: Introspect scoring breakdowns (BM25 vs vector vs graph).

   - `snippets=K`: Search automatically inlines source snippets for the top $K$ results (default 3) directly in Turn 1 across docs and code. Set `snippets=0` for pure handle sweeps.
   - `detail="ids"`: Strips Turn 1 snippets, graph affordances, and zero score breakdowns for minimal token consumption (<250 tokens) during wide identifier sweeps.
3. **Turn 1 Affordance Grounding, Path Expansion & Structural Census**:
   - Census: Use `status(scope="census" | "architecture")` for instant (<2ms) whole-repository structural inventory (symbol counts, edge counts, language breakdown, and total file counts).
   - Turn 1: `search` returns partitioned results (`docs` and `code`) enriched with `graph_affordances` (degree counts: `calls_in`, `calls_out`, `implements`, `imports`, `wikilinks_in`, etc.) and `schema_envelope` (active node labels and edge types).
   - Turn 2: Follow information scents with `graph_match` using linear Cypher-Lite ASCII patterns, e.g. `(:CodeSymbol {name: "foo"})-[:calls*1..2]->(target)` or `(:DocNode {path: "adrs/002.md"})-[:supersedes]->(target)`. Use `graph_communities(view="architecture")` for high-level architectural component mapping (summarized key nodes, no full member dump).
4. **Progressive disclosure (Strict 3-Tier Pipeline)**:
   - Tier 1: Query `search` (receives top $K$ source snippets + handles + affordance degree counts).
   - Tier 2: Fetch targeted symbol definitions or doc chunks via `get_snippet(symbol="...")` / `get_snippet(path="...", chunk_id=N)`.
   - Tier 3: Read whole files or bounded line slices (`read_file(path="...", line_range=[start, end])`) *only* when necessary for line-exact editing.
5. **Schema discipline on writes**: Query `list_templates` before authoring, write via `write_note`, and confirm validity with `validate(path="...")`.
6. **Destructive operations**: `delete_note` permanently removes files and index entries; confirm with user before executing.
7. **Absolute Git Push Prohibition**: Never run `git push` or attempt automated remote push commands. Remote synchronization is strictly reserved for manual human execution.
8. **Workspace MCP Configuration Exclusivity (`.agents\mcp_config.json`)**:
   - Whenever asked to install, update, reconfigure, or add MCP servers, AI agents must **exclusively edit `.agents\mcp_config.json`** in the repository root.
   - **Never touch global configuration files** such as `~/.gemini/antigravity-ide/mcp_config.json` or `~/.gemini/config/mcp_config.json`.
   - Co-locate comparative servers (e.g., `groundcontrol` and `codebase-memory`) inside `.agents\mcp_config.json` to enable side-by-side / head-to-head evaluation.

---

## 4. Workspace Structure & Module Layout

groundcontrol/
├── crates/
│   ├── groundcontrol-common/  # Domain types, ports traits, config, errors
│   ├── groundcontrol-core/    # Engine, Tantivy, embeddings (DirectML/ort), Petgraph, AST chunkers
│   ├── groundcontrol-mcp/     # Stdio & HTTP transport, MCP protocol, tool registry (17 tools)
│   ├── groundcontrol-algo/    # Standalone retrieval library & CLI (AlgoBackend substrate for groundtruth)
│   ├── groundcontrol-graphview/ # Real-time 3D knowledge graph visualizer and protocol
│   └── groundcontrol-cli/     # Composition root binary, multi-corpus CLI
├── docs/                 # Authoritative architecture, concepts, and roadmap docs
└── .index/               # Derived indices: meta.db, tantivy/, vectors.json, graph.bin
```

### Key Modules in `groundcontrol-core`
- `engine.rs` / `engine_builder.rs`: Core engine orchestration and port assembly.
- `corpus_manager.rs`: Multi-corpus routing and cross-corpus symbol resolution (`link_cross_corpus_symbols`).
- `search/`: Modal search strategies (`bm25`, `semantic`, `hybrid`, `graph`, `related`, `explain`) and RRF fusion.
- `graph/code.rs`: AST-derived code edges (`defines`, `imports`, `calls`, `implements`) with confidence bands.
- `index/pipeline.rs`: Hardware-accelerated indexing pipeline with batched ONNX tensor staging.
- `parser/code/`: Tree-sitter polyglot AST chunker across 12+ languages.

---

## 5. Technology Stack, Toolchain & Quality Standards

- **Language**: 100% pure Rust, Edition 2021, pinned **MSRV 1.80** (`rust-toolchain.toml`).
- **Safety**: `unsafe_code = "forbid"` workspace-wide.
- **Linting**: `missing_docs = "warn"`, Clippy enabled for `correctness`, `suspicious`, `perf`. Runs with `-D warnings`.
- **Hardware Acceleration**: Windows DirectML, macOS CoreML, Linux CUDA, with automatic CPU SIMD fallback (`ort` 2.0.0-rc.13).

### Developer Workflow (`just`)
| Recipe | Action |
|---|---|
| `just check` | `cargo check --workspace --all-features --all-targets` |
| `just clippy` | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
| `just fmt-check` | `cargo fmt --all -- --check` (`just fmt` to format) |
| `just test` | `cargo test --workspace --all-features --locked` |
| `just build-release` | `cargo build --workspace --all-features --release --locked` |
| `just ci` | Full local CI check (fmt-check + clippy + test + deny + docs) |

---

- **Hooks**: Git hooks enabled via `just setup-hooks` (`.githooks/pre-push` enforces formatting, clippy, tests, and tag alignment with `Cargo.toml`).

---

## 7. Evergreen Documentation & Bidirectional Code Links

Documentation in `groundcontrol` is not passive prose; it is a **compiled, structured knowledge corpus** that dogfoods `groundcontrol`'s own semantic indexing and graph retrieval.

### Strict 3-Pillar Documentation Hierarchy
All documentation must conform to the 3-pillar directory layout:
* `docs/architecture/`: Systems engineering, build operations (`building/`), security/trust (`trust/`), backend internals (`implementation/`), and Architectural Decision Records (`adr/`).
* `docs/concepts/`: Theoretical paradigms, 3-tier progressive disclosure contracts (`progressive-disclosure/`), and multimodal retrieval theory (`search/`).
* `docs/roadmap/`: Long-term engineering roadmaps (`coderoadmap.md`) and technical specifications (`RFC-*.md`).

### Invariants for AI Pair Programming
1. **Never Let Documentation Rot**: Whenever modifying a port trait, tool signature, CLI argument, indexing pipeline, or core data structure, you MUST update the corresponding documentation under `docs/` in the same commit.
2. **Bidirectional Code Linking**: Technical documentation must link directly to active Rust source files and symbols using `[Symbol](file:///c:/dev/ctx/groundcontrol/crates/...)` syntax to provide ground-truth provenance.
3. **Wikilink & Frontmatter Integrity**: Every document must maintain valid YAML frontmatter (`title`, `category`, `status`, `tags`, `related`) and valid `[[wikilinks]]`. Never create broken links.

---

## 8. Sister Repository: `groundtruth` & Polyglot Benchmark Corpora

Academic benchmarking and IR evaluation are decoupled from `groundcontrol` internals and live in the sister repository [`groundtruth`](file:///c:/dev/semantic/groundtruth). All benchmarking suites, runners, and data files have been fully migrated into `groundtruth`; `groundcontrol` contains zero internal benchmarking test harnesses (`gc-bench` and `benchmarks/` have been removed).

### Two-Tier Protocol Model
1. **Tier 1 — In-Process Algorithmic Ablation (`groundcontrol-core::algorithm::eval`)**:
   - `groundtruth` imports `groundcontrol-core` as a direct Cargo path dependency.
   - Zero IPC, zero serialization, direct Rust function calls.
   - Evaluates isolated algorithms (`binary`, `bm25`, `ppr`, `fast`, `semantic`, `hybrid`) and runtime projection variants (`FlatSif` vs `PartitionedHyperplane`) via `AlgoBackend`.
   - Executed strictly in serial (`gt ablate`) for unperturbed latency percentiles (p50, p90, p99).
2. **Tier 2 — System-Level Multi-Agent Evaluation (MCP over stdio JSON-RPC)**:
   - `groundtruth` executes `gt run` against the compiled `groundcontrol` MCP server binary as a black box.
   - Measures real-world agent tool execution, transport overhead (~80ms), and Turn 1-3 progressive disclosure contracts.

### Principles of Decoupled Evaluation
- **Polyglot Benchmark Grounding**: Evaluation targets real-world, enterprise-grade multi-language architectures rather than isolated single-function snippets. The primary reference corpus is the **OpenTelemetry Astronomy Shop** (`corpora/otel-demo.toml`, covering 11+ languages across Rust, Go, TypeScript, Python, C#, Java, C++, Ruby, Kotlin, PHP, Elixir) and multi-repo architectures communicating via gRPC/Protobuf, alongside canonical CodeSearchNet, RepoBench, and SWE-bench corpora.
- **Authoritative Metrics Substrate**: Standard IR metrics (Recall@K, Precision@K, MRR@K, nDCG@K with graded relevance), query latency distribution percentiles (p50, p90, p95, p99), and statistical significance (paired Student's t-test and Wilcoxon signed-rank test) are governed by `groundtruth-judge`.



