# RFC: Cross-Corpus Graph Federation & Multi-Repo Symbol Resolution

**Status**: Implemented
**Author**: Architecture Team
**Scope**: `groundcontrol-core`, `groundcontrol-common`, `groundcontrol-mcp`, `groundcontrol-cli`
**Date**: September 2026
**Target Version**: `0.1.0`+
**Inspiration / Benchmark**: `codebase-memory-mcp` (the comparative benchmark this RFC seeks to match and surpass)
**Related Documents**: [RFC-treesitter-expansion-and-lsp-analysis.md](file:///c:/dev/ctx/groundcontrol/docs/RFC-treesitter-expansion-and-lsp-analysis.md), [RFC-adaptive-graph-expansion.md](file:///c:/dev/ctx/groundcontrol/docs/RFC-adaptive-graph-expansion.md), [ARCHITECTURE.md](file:///c:/dev/ctx/groundcontrol/docs/ARCHITECTURE.md)

---

## 1. Executive Summary

`groundcontrol` indexes **one repository at a time** into a self-contained *corpus* (its own tantivy BM25 index, HNSW vector index, petgraph knowledge graph `graph.bin`, and SQLite metadata store), routed by `CorpusManager`. This "one repo at a time" isolation is a deliberate invariant and must be preserved.

The comparative benchmark, `codebase-memory-mcp`, keeps the same physical isolation (one SQLite DB per project; its SQLite authorizer *denies* `ATTACH`/`DETACH`, so a single query physically cannot join across databases). Its flagship differentiator is a batch **cross-repo-intelligence** pass that matches service boundaries (HTTP routes, async topics, pub/sub channels, gRPC/GraphQL/tRPC endpoints) across projects and materializes bidirectional `CROSS_*` edges into both project databases, carrying the remote endpoint identity (`target_project`/`target_function`/`target_file`) in edge properties. Its `trace_path` `cross_service` mode simply includes those `CROSS_*` edge types; the walk stays inside one database and surfaces the remote endpoint via edge properties, requiring the user to re-invoke against the target project to continue.

This RFC documents the current state of both systems and proposes a plan that:

1. Makes **all search modes** (`bm25`, `semantic`, `hybrid`, `graph`) scopable to **N or N+1 target corpora** (single, explicit set, or `"all"`).
2. Adds **cross-corpus graph federation**: traversal that indicates *where* a corpus hop occurs, *which* corpus it hops to, and — going beyond the benchmark — **continues the traversal live into the target corpus's graph** within the daemon (gateway → middleware → database host/table → back, with each service also linking to the infra repo that deploys it).
3. Adds **export/commit of per-corpus indexes** as single loadable artifacts, importable into the central daemon.
4. Replaces best-guess ingest-root heuristics with a **deferred, confidence-banded, post-processing symbol-resolution pass** (SCIP > LSP > qualified-name) that produces accurate multi-repo graph edges **without abandoning single-repo indexing**.

---

## 2. Invariant Constraints

Any implementation must conform to `groundcontrol`'s architectural invariants (see [`GEMINI.md`](file:///c:/dev/ctx/groundcontrol/GEMINI.md)):

1. **Source on disk is authoritative ground truth.** All indices are disposable and rebuildable.
2. **Single-repo indexing is preserved.** Cross-repo resolution is a *post-processing reconciliation step* over independently built corpora, never a change to how a single corpus is indexed.
3. **Sub-millisecond query latency.** Cross-corpus traversal must stay bounded; live continuation must be depth- and budget-limited.
4. **Pure Rust safety** (`#![forbid(unsafe_code)]`). No mandatory external daemons or C toolchains at query time.
5. **Zero external agent dependencies at index time.** SCIP/LSP resolvers are *optional accelerators*, never required to index a repo.
6. **Multi-language and cross-corpus by construction.** Every mechanism must be language-agnostic.

---

## 3. Current State (Verified Against Source)

### 3.1 groundcontrol

| Concern | Current behaviour | Source |
| :--- | :--- | :--- |
| Scoping unit | One *corpus* = one indexed root with own BM25/vector/graph/SQLite. `CorpusManager` holds `HashMap<String, Engine>` routed by name. | `crates/groundcontrol-core/src/corpus_manager.rs` |
| Storage | Repo-local `.index/` when `.index` or `groundcontrol.toml` exists, else central `${GROUNDCONTROL_CACHE_DIR}/corpora/<name>/`. | `corpus_manager.rs::ensure_corpus` |
| Cross-corpus **search** | Read tools accept `corpus` (single) or `corpora` (array / `"all"`); `resolve_corpus_target` resolves the target **before** the tool runs (mode-agnostic); `fan_out_read` runs per corpus and merges with `rrf_fuse_cross_corpus` (RRF K=60, keyed by `(corpus, path)`, tagged with origin corpus). Write tools never fan out. | `crates/groundcontrol-mcp/src/tools/mod.rs`, `crates/groundcontrol-core/src/search/mod.rs` |
| Modalities | BM25 (tantivy), semantic (Jina v2 base-code, 768-dim, INT8; the only live model — `bge` is a config-accepted string that falls back to Jina), plus `hybrid` (3-signal), `graph`, `explain`, `related`, `multihop`. | `search_service.rs`, `search/mod.rs`, `embedding.rs` |
| Graph traversal | `KnowledgeGraph` = one `petgraph::DiGraph` per corpus; `traverse_bfs` walks **only that graph**. | `crates/groundcontrol-core/src/graph/mod.rs` |
| Cross-corpus edges | Modeled as a **proxy node** `"<corpus>::<scope_path>"` + `GraphEdge.target_corpus` + `confidence`. Created **only** by `link_cross_corpus_symbols` for **document frontmatter** targets resolving to **exactly one** symbol in another corpus. Traversal following such an edge **lands on the stub and stops**. | `corpus_manager.rs::link_cross_corpus_symbols`, `graph/mod.rs::add_edge_full` |
| Doc↔code (cross-modality) | First-class: `EdgeClass::CrossModal`, `EdgeProvenance::DocumentsCode` / `ImplementsAdr`. Intra-corpus frontmatter target hitting a code `scope_path` lands on the real symbol node. | `graph/mod.rs`, `groundcontrol-common/src/types.rs` |
| Code extraction | `graph/code.rs` produces `scope_path` qualified names and resolves callees with a **confidence band** (`High` unique / `Medium` same-dir / `Speculative`). Unresolved callees are currently **dropped**. `hybrid_lsp` and `scip` modules already exist. | `crates/groundcontrol-core/src/graph/code.rs`, `graph/hybrid_lsp.rs`, `graph/scip.rs` |

### 3.2 codebase-memory-mcp (benchmark)

| Concern | Behaviour | Source |
| :--- | :--- | :--- |
| Scoping | One SQLite DB per project (`<name>.db`); SQLite authorizer **denies ATTACH/DETACH**. One project per query — no fan-out. | `src/store/store.c`, `src/mcp/mcp.c` |
| Cross-repo | Batch `cross-repo-intelligence` pass (`pass_cross_repo.c`) opens each target DB, matches **Route/Channel/RPC** by URL-path template + method (fuzzy `{param}` segment match), broker + topic, channel name + transport, gRPC service/method, GraphQL op, tRPC procedure. Writes **bidirectional** `CROSS_HTTP_CALLS` / `CROSS_ASYNC_CALLS` / `CROSS_CHANNEL` / `CROSS_GRPC_CALLS` / `CROSS_GRAPHQL_CALLS` / `CROSS_TRPC_CALLS` edges into **both** DBs; `properties_json` carries `target_project`/`target_function`/`target_file`. | `src/pipeline/pass_cross_repo.c` |
| Cross-service trace | `trace_path` `cross_service` mode = edge-type set including `CROSS_*`. BFS stays in **one** DB, hops to the boundary node, surfaces the remote endpoint via edge props; user re-invokes against `target_project` to continue. | `src/mcp/mcp.c::resolve_trace_edge_types`, `handle_trace_call_path` |
| Similarity | `SIMILAR_TO` (MinHash LSH) + `SEMANTICALLY_RELATED` (11-signal *algorithmic* embedding, 768-dim, no external model, threshold 0.75). Code-to-code only. | `src/pipeline/pass_semantic_edges.c`, `pass_similarity.c` |
| Doc↔code | **No** explicit prose-node→code-symbol edge. Docstrings fold into the FTS `body` column + embedding tokens. ADRs stored as unlinked free text. | `src/store/store.c` |

### 3.3 Gap Analysis Against the Three Objectives

- **Objective 1 (all modes scopable to N/N+1 corpora):** ~90% present. `resolve_corpus_target` is mode-agnostic, so `bm25`/`semantic`/`hybrid`/`graph` are already scopable via `corpus`/`corpora`/`"all"`. **Gap:** `graph` mode under fan-out runs BFS *independently per corpus* and RRF-merges result rows — it does not cross a corpus boundary mid-walk (that is Objective 2).
- **Objective 2 (federated traversal with hop indication + live continuation):** Largest gap. Cross-corpus edges (a) exist only for doc-frontmatter targets, (b) carry only `target_corpus` (no rich `target_path`/`target_symbol`/`target_kind` payload), and (c) dead-end at the proxy node. There is **no** service-boundary (route/RPC/channel) or infra-resource matching, and **no** live multi-graph continuation.
- **Objective 3 (export/commit loadable per-corpus indexes):** Pieces exist (`ensure_corpus` hybrid storage; postcard `graph.bin`; SQLite; `vectors.bin`; `tar`/`zstd` already in workspace deps). **Gap:** no single-artifact bundle export + import/bootstrap + version/compat stamp.

**groundcontrol already wins** on: cross-corpus RRF search fan-out (the benchmark has none) and first-class doc↔code/ADR edges (the benchmark has none).

---

## 4. Recommended Approach

### 4.1 Design principle: deferred resolution as daemon-level reconciliation

Preserve single-repo indexing exactly as-is. Add a **two-phase** model:

- **Phase A (per repo, minimally changed):** during code extraction, when a call/import target cannot be resolved locally, **persist it as an external reference** (raw qualified name / import path + caller `scope_path` + confidence band) instead of dropping it. The confidence machinery in `graph/code.rs::resolve_callee` already exists.
- **Phase B (daemon reconciliation, additive):** for each external reference, resolve across all mounted corpora via `resolve_symbol_across_corpora`, and on a **unique** cross-corpus match emit a **real, richly-tagged, bidirectional, traversable** cross-corpus edge. This generalizes today's `link_cross_corpus_symbols` from "doc frontmatter targets" to "unresolved code call/import targets" and "service-boundary/resource identifiers".

Because Phase B only *adds edges* to already-built per-corpus graphs, "index one repo at a time" is fully preserved and re-runnable/idempotent (the graph already de-duplicates same-type edges).

### 4.2 Resolver trust ladder (highest first)

1. **SCIP** (`scip-clang`, `scip-typescript`, `scip-python`, `rust-analyzer` SCIP export): globally-unique symbol monikers → exact, language-uniform cross-repo linking. The `graph/scip.rs` module already anticipates ingestion.
2. **Hybrid LSP** (`graph/hybrid_lsp.rs`): in-engine static type/symbol resolution for repos lacking a SCIP index.
3. **Qualified-name matching** (current mechanism): always-available fallback, gated by confidence bands so `Speculative` matches never emit hard edges.

SCIP/LSP are **optional accelerators**. Absent them, qualified-name matching still functions.

### 4.3 Cross-corpus edge model (richer + traversable)

Extend `GraphEdge` (postcard serializes all fields, so this is a compatible addition once the on-disk graph version is bumped) with a remote-endpoint payload:

- `target_path: Option<String>` — remote node key in the target corpus graph.
- `target_symbol: Option<String>` — remote `scope_path` / qualified name.
- `target_kind: Option<String>` — `Route` | `Channel` | `RpcEndpoint` | `Resource` | `Symbol`.
- (retain existing `target_corpus`, `confidence`.)

Mirror each cross-corpus edge into the **target** corpus graph (reverse direction), matching the benchmark's bidirectionality so a trace works from either side.

### 4.4 Service-boundary & infra-resource node layer

Introduce language-agnostic boundary node kinds and extractors:

- `Route` (HTTP method + path template), `Channel` (name + transport), `RpcEndpoint` (service/method | operation | procedure).
- `Resource` for infra corpora (Terraform / Kubernetes / Compose): keyed by stable identifiers (image name, service name, hostname, DB identifier, SQL table).

A Phase B matcher (mirroring `pass_cross_repo.c` semantics) links a client call in corpus A to a handler in corpus B, and links a service's Route/Resource to the infra `Resource` that deploys it — all through the same cross-corpus resolver, no special-casing.

### 4.5 Federated traversal (beyond the benchmark)

Add a `cross_service` / federated traversal mode to graph search and a dedicated MCP tool. On reaching a cross-corpus edge during BFS, the traversal:

1. **Reports the hop** — emits the origin node, the `target_corpus`, and the rich remote payload.
2. **Optionally continues live** — resolves the `target_corpus` engine via `CorpusManager` (all engines are in one process) and continues BFS in that graph, bounded by remaining depth and a global hop budget.

This live multi-graph continuation is something the benchmark **cannot** do (its authorizer denies `ATTACH`), making it a genuine improvement rather than parity. Latency is protected by a strict `max_corpus_hops` budget and per-corpus depth caps.

### 4.6 Index bundle export/import

Add `export`/`import` (CLI + daemon tool) that bundles a corpus `.index/` (graph.bin + vectors.bin + meta.db + tantivy/) into a single `zstd`-compressed `tar` artifact with a **manifest** recording: corpus name, embedding model + dims, graph schema version, groundcontrol version, source commit. Import validates the manifest (reject on embedding-model/dim mismatch) and mounts the corpus into the running daemon. Mirrors the benchmark's `.codebase-memory/graph.db.zst` team-sharing artifact.

---

## 5. Worked Case

`api-gateway` (repo A) → `middleware-service` (repo B) → `database-host` + SQL table (repo C), each linking to `infra` (repo D) that deploys it, and traceable back:

1. Index A, B, C, D independently (Phase A). A records an unresolved HTTP call to `/v2/orders`; B records an unresolved SQL/DB reference; each service records its deployment identifiers.
2. Phase B: match A's client call to B's `Route` handler → bidirectional `CROSS_HTTP_CALLS`; match B's DB reference to C's `Resource`/table → `CROSS_DB` edge; match each service's identifiers to D's `Resource` → `DEPLOYS`/`DEPLOYED_BY` cross-corpus edges.
3. Federated `trace_path` from A's handler continues live: A → (hop to B) → B handler → (hop to C) → table, with a side-branch at each service into D's deploying resource, and reverse edges enabling the return trace.

---

## 6. Non-Goals

- Merging corpora into one physical graph object (rejected — violates single-repo isolation).
- Requiring SCIP/LSP to index (they remain optional accelerators).
- Comparing raw scores across indexes (fusion stays rank-based / RRF).

---

## 7. Risks & Mitigations

| Risk | Mitigation |
| :--- | :--- |
| False cross-repo edges from ambiguous names | Emit hard edges only on **unique** matches; gate by confidence band; `Speculative` never emits. |
| Latency blow-up from live continuation | Strict `max_corpus_hops` + per-corpus depth caps; continuation is opt-in per query. |
| Stale cross edges after re-index | Phase B is re-runnable and idempotent; delete-then-rebuild cross edges for the affected corpus (benchmark does the same). |
| Portable index incompatibility | Bundle manifest with model/dim/schema/version; import validates and refuses mismatches. |
| On-disk graph format change | Bump `graph.bin` schema version; provide load-time migration or rebuild-on-mismatch. |

---

## 8. Implementation Plan

See [`todo.txt`](file:///c:/dev/ctx/groundcontrol/todo.txt) in the repository root for the phased, subagent-executable task breakdown. Phases are ordered so each is independently verifiable (`cargo build` + `cargo test` green) before the next is dispatched.
