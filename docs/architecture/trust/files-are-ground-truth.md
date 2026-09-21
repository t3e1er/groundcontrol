---
title: "Files are Ground Truth"
description: "Why plain text markdown and polyglot source files remain the only authoritative state."
category: "trust"
status: "active"
tags: ["ground-truth", "markdown", "git", "disposable-index", "invariants"]
related:
  - "[[docs/architecture/trust/index]]"
  - "[[docs/architecture/trust/deterministic-graph]]"
  - "[[docs/architecture/implementation/hexagonal-architecture]]"
---

# Files are Ground Truth

In `groundcontrol`, **files on disk are king**.

All internal indices—Tantivy Okapi BM25 indices, HNSW vector graphs, SQLite metadata tables, and Petgraph relational graphs—are strictly **derived, disposable, and 100% rebuildable artifacts**.

---

## The Risk of Canonical Vector Databases

Many AI knowledge bases treat a vector database (such as Pinecone, Chroma, or Milvus) or a graph database (such as Neo4j) as the canonical system of record. This leads to catastrophic failure modes in software engineering:
1. **Divergence from Git**: When source code or docs change via branch switching, rebase, or PR merge, external databases fall out of sync, leading agents to hallucinate non-existent code or outdated specifications.
2. **Vendor Lock-in & Corruption**: If an index format corrupts or the database server crashes, historical knowledge is lost unless cumbersome backups are restored.
3. **Loss of Human Auditability**: Engineers cannot review diffs of binary vector stores or opaque graph nodes in a pull request.

---

## The `groundcontrol` Ground-Truth Contract

1. **Pure Markdown & Source**: Notes, architectural decision records (ADRs), specifications, and code are stored as plain files within the repository.
2. **Disposable `.index/`**: The entire `.index/` directory can be deleted at any time with `rm -rf .index`. Running `groundcontrol sync` or restarting the server completely reconstructs BM25 postings, vector embeddings, and AST graphs from scratch.
3. **Zero Synchronization Drift**: File system watchers (`notify`) detect file modification timestamps (`mtime`) and trigger immediate atomic incremental updates in memory.
4. **Git-Native Collaboration**: Teams collaborate using standard Git pull requests, code reviews, and blame histories. `groundcontrol` simply indexes whatever is checked out.
