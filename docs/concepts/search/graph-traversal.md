---
title: "Graph Traversal & Cypher-Lite CTEs"
description: "High-speed graph queries, Petgraph topology, recursive SQLite Common Table Expressions, and community detection."
category: "search"
status: "active"
tags: ["graph", "cypher-lite", "petgraph", "sqlite-cte", "leiden", "louvain", "communities"]
related:
  - "[[docs/concepts/search/index]]"
  - "[[docs/architecture/trust/deterministic-graph]]"
  - "[[docs/concepts/progressive-disclosure/turn-1-affordances]]"
  - "[[docs/architecture/adr/adr-003-leiden-louvain-graph-clustering]]"
---

# Graph Traversal & Cypher-Lite CTEs

While BM25 and vector search operate on isolated text chunks, software systems are fundamentally **graphs of dependencies and concepts**.

`groundcontrol` integrates two complementary graph representations:
1. **In-Memory Petgraph**: For sub-millisecond Personalized PageRank (`search_related`) and shortest-path reachability.
2. **Recursive SQLite CTE Engine**: Compiling linear **Cypher-Lite** patterns into recursive SQL queries with cycle guards for multi-hop path extraction (`graph_match`).

* **Petgraph KnowledgeGraph**: [`crates/groundcontrol-core/src/graph/mod.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/graph/mod.rs)
* **SQLite Graph Traversal Backend**: [`crates/groundcontrol-core/src/catalog/sqlite.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/catalog/sqlite.rs)
* **GraphStore Port**: [`crates/groundcontrol-common/src/ports.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/ports.rs)

---

## 1. Cypher-Lite Query Language (`graph_match`)

Agents execute multi-hop queries using clean ASCII pattern syntax:

```text
(:CodeSymbol {name: "Engine"})-[:calls*1..2]->(target)
(:DocNode {path: "adrs/001-architecture.md"})-[:derived_from*1..3]->(target)
(source)-[:implements]->(:CodeSymbol {name: "TextIndex"})
```

### The 5 Typed Edge Classes
Traversals can filter across dedicated graph layers:
* `code`: AST relationships (`defines`, `imports`, `calls`, `implements`).
* `semantic`: Markdown links (`wikilink`, `derived_from`, `shared_tag`).
* `structural`: Document hierarchy (`parent_child`, `section`).
* `crossmodal`: Links between code and documentation (`documents`, `implements_spec`).
* `hybrid`: Blended multi-layer traversals.

### Hierarchical Branching Tree & Hub Suppression
Rather than returning flat Cartesian path lists that duplicate prefixes and waste context tokens, [`graph_match`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/tools/mod.rs) returns a hierarchical branching tree modeled by [`GraphMatchResult`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/types.rs):

```json
{
  "root": "detect_bundle",
  "file": "crates/groundcontrol-core/src/bundle.rs:32",
  "summary": {
    "direct": 2,
    "transitive": 3,
    "files": 2,
    "max_depth": 2
  },
  "tree": [
    {
      "node": "build_pipeline",
      "rel": "calls",
      "file": "crates/groundcontrol-core/src/pipeline.rs",
      "line": 45,
      "hop": 1,
      "branches": [
        {
          "node": "main",
          "rel": "calls",
          "file": "crates/groundcontrol-cli/src/main.rs",
          "line": 110,
          "hop": 2
        }
      ]
    }
  ],
  "total_matches": 3
}
```

#### Key Capabilities:
* **Quantified Blast Radius (`summary`)**: Instant structural signal informing the agent of `direct` (1-hop), `transitive` (multi-hop ripple), `files` affected, and `max_depth` traversed.
* **Hub Suppression**: High-degree utilities (e.g. logging/formatting/error helpers with >10 callers) are automatically capped at 10 branches, flagged with `"hub": true`, and annotated with `"suppressed": <count>` to prevent combinatorial explosion.
* **Cycle Guarded**: Path ancestry sets prevent cycles and infinite loops across mutual call chains.
* **Zero Semantic Duplication**: Redundant properties and syntax wrappers are omitted; file and line references provide direct jump targets.

Execution by [`QueryEngine`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/graph/query.rs) finishes in **under 1.8ms**.

---

## 2. Architectural Community Detection (`graph_communities`)

To understand the macro-architecture of an unfamiliar codebase without reading every file, agents run `graph_communities`:
* **Algorithms**: Leiden (connectivity-refined) and Louvain (modularity maximization).
* **View Modes**:
  * `view="architecture"`: Groups files into functional subsystems (e.g. "Storage Engine", "Transport Layer", "Indexing Pipeline").
  * `view="raw"`: Detailed symbol-level graph clusters.
