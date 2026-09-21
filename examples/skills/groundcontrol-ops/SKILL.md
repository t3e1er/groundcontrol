---
name: groundcontrol-ops
description: >-
  Manage corpus lifecycle, trigger incremental delta syncing, perform full re-indexing,
  and inspect graph topological metrics and community clusters using groundcontrol MCP tools.
  Use this skill when synchronizing index state after filesystem changes or analyzing vault connectivity statistics.
---

# GroundControl Operations, Index Lifecycle & Graph Topology

This skill guides agents through operational maintenance of `groundcontrol` knowledge bases and codebases, including delta synchronization, index rebuilds, embedding refreshes, and topological community analysis using the 17-tool suite.

---

## 1. Corpus State & Delta Synchronization

When files in the knowledge vault or codebase have been edited or added externally:

1. **List Configured Corpora**:
   Call `list_corpora` to inspect active corpora, their base paths, document count, and index status:
   ```json
   {}
   ```
2. **Execute Incremental Delta Scan**:
   Call `sync_corpus` with `mode="delta"` (default) to index newly added or modified files and prune deleted entries:
   ```json
   {
     "mode": "delta"
   }
   ```
3. **Full Re-Index (Cold Rebuild)**:
   When modifying tokenization rules or chunking strategies in `groundcontrol.toml`, call `sync_corpus` with `mode="full"`:
   ```json
   {
     "corpus": "knowledge-base",
     "mode": "full"
   }
   ```
   When changing embedding models in `groundcontrol.toml`, call `sync_corpus` with `mode="reembed"`:
   ```json
   {
     "mode": "reembed"
   }
   ```

---

## 2. Graph Analytics & Community Detection

To understand the macro-structure of the knowledge graph:

1. **Inspect Graph Statistics**:
   Call `status` with `scope="graph"` to review total nodes, edge density, average degree, connected components, and isolated orphan notes:
   ```json
   {
     "scope": "graph"
   }
   ```
2. **Identify Topological Communities & Architecture**:
   Call `graph_communities` with `view="architecture"` to detect subsystem clusters and modularity partitions:
   ```json
   {
     "view": "architecture",
     "algorithm": "leiden"
   }
   ```
3. **Audit Structural Health & Taxonomy**:
   Call `validate` with `check_taxonomy=true` to check for broken wikilinks, circular dependencies, and orphan ADRs:
   ```json
   {
     "check_taxonomy": true
   }
   ```
4. **Inspect Coverage & Parsing Health**:
   Call `status` with `scope="coverage"` to verify that all directories and file types are correctly indexed and parsed without empty gaps.
