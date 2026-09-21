---
name: groundcontrol-search
description: >-
  Retrieve knowledge and code, execute Cypher-Lite graph queries, inspect bounded snippets,
  and trace entity paths using groundcontrol MCP tools. Use this skill when the user
  asks questions about codebase architecture, documentation, past decisions, or multi-hop entity connections.
---

# GroundControl Hybrid Search & Graph Navigation

This skill teaches agents how to effectively query knowledge bases and codebases using `groundcontrol`'s hybrid 3-Way Reciprocal Rank Fusion (RRF), BM25 lexical search, dense ONNX vector search, and Cypher-Lite typed graph queries.

---

## 1. Fast Hybrid Retrieval Flow & Turn 1 Snippets

When answering questions or researching topics:

1. **Perform Unified Hybrid Search**:
   Call `search` with `mode="hybrid"` and requested snippet count:
   ```json
   {
     "query": "How does vector index compaction work?",
     "mode": "hybrid",
     "limit": 5,
     "snippets": 3
   }
   ```
2. **Review Partitioned Results & Turn 1 Snippets**:
   Examine returned `docs` and `code` hits.
   - For top hits, `groundcontrol` inlines text/code snippets directly in Turn 1.
   - Ground your answer on these Turn 1 snippets whenever possible without secondary tool calls.
3. **Inspect Graph Affordances**:
   Review `graph_affordances` (`calls_in`, `calls_out`, `implements`, `imports`, `wikilinks_in`) to identify central hubs or dependencies.
4. **Targeted Symbol or Chunk Fetching (Tier 2)**:
   If a snippet needs complete signature or chunk details, call `get_snippet`:
   ```json
   {
     "symbol": "NewMainKubelet"
   }
   ```
5. **Line-Bounded Reading (Tier 3)**:
   Only if full context across multiple symbols or notes is required, call `read_file` with explicit line bounds:
   ```json
   {
     "path": "pkg/kubelet/kubelet.go",
     "start_line": 1,
     "end_line": 150
   }
   ```

---

## 2. Cypher-Lite Graph Exploration (`graph_match`)

When the query requires understanding connections between distinct components, decisions, or code symbols:

1. **Trace Call Graphs & AST Relationships**:
   To find callee functions called by a specific symbol:
   ```json
   {
     "pattern": "(:CodeSymbol {name: \"NewMainKubelet\"})-[:calls*1..2]->(target)",
     "edge_class": "code"
   }
   ```
2. **Trace Interface Implementations**:
   To discover which structs implement a given trait or interface:
   ```json
   {
     "pattern": "(source)-[:implements]->(target {name: \"Store\"})"
   }
   ```
3. **Trace Documentation Ancestry & Decisions**:
   To discover decisions or incidents that an ADR or concept note is derived from:
   ```json
   {
     "pattern": "(:DocNode {path: \"decisions/adr-002-bm25.md\"})-[:derived_from*1..3]->(target)"
   }
   ```
4. **Inspect High-Level Architectural Communities**:
   To understand subsystem clustering and community topology:
   ```json
   {
     "view": "architecture"
   }
   ```
   (using the `graph_communities` tool).

---

## 3. Lexical Keyword & Symbol Precision

When looking for exact error codes, struct names, CLI flags, or function signatures:

1. Call `search` with `mode="bm25"`:
   ```json
   {
     "query": "ReciprocalRankFusion min_chunk_tokens",
     "mode": "bm25",
     "limit": 10
   }
   ```
2. To diagnose how BM25, vector, and graph components contributed to scores, call `search` with `mode="explain"`:
   ```json
   {
     "query": "ReciprocalRankFusion",
     "mode": "explain",
     "limit": 5
   }
   ```

---

## 4. Verification & Best Practices

- **Never guess note or file paths**: Always retrieve candidates via `search` first.
- **Avoid Context Flooding**: Work from Turn 1 snippets and bounded `get_snippet` calls rather than reading entire vaults into LLM context.
- **Always Cite Sources**: State file paths and line ranges in answers (e.g., `[concepts/hybrid-retrieval.md#3-way-rrf]` or `[pkg/kubelet/kubelet.go:120-145]`).
