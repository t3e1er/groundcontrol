---
title: "Trust, Safety & Determinism Hub"
description: "Why files on disk are ground truth, deterministic AST graphs, pure safe Rust invariants, and knowledge crystallization."
category: "trust"
status: "active"
tags: ["trust", "ground-truth", "determinism", "safety", "schema", "provenance"]
related:
  - "[[docs/index]]"
  - "[[docs/architecture/trust/files-are-ground-truth]]"
  - "[[docs/architecture/trust/deterministic-graph]]"
  - "[[docs/architecture/trust/pure-rust-invariants]]"
  - "[[docs/architecture/trust/schema-validation]]"
  - "[[docs/architecture/trust/knowledge-crystallization]]"
---

# Trust, Safety & Determinism Hub

In modern AI-assisted engineering, **trust is the primary bottleneck**. When an AI tool hallucinates code relationships, corrupts knowledge files, or relies on proprietary opaque vector stores, developer velocity collapses.

`groundcontrol` is architected as an **uncompromising trust layer** between AI models and your filesystem.

---

## Core Invariants

* **[[docs/architecture/trust/files-are-ground-truth]]**: Why markdown and source files on disk remain authoritative, while all indices are disposable and 100% rebuildable in seconds.
* **[[docs/architecture/trust/deterministic-graph]]**: Why AST relations (`calls`, `defines`, `imports`, `implements`) and wikilinks are extracted via deterministic grammars rather than stochastic LLMs.
* **[[docs/architecture/trust/pure-rust-invariants]]**: `#![forbid(unsafe_code)]`, zero C-runtime dependencies, and sub-millisecond execution budgets.
* **[[docs/architecture/trust/schema-validation]]**: Enforcing formal YAML frontmatter templates, taxonomy checks, and whole-corpus health audits.
* **[[docs/architecture/trust/knowledge-crystallization]]**: Principle 3 — transforming ephemeral conversational exhaust into permanent, verified semantic notes with lineage (`derived_from`).

---

## Summary Matrix

| Principle | Traditional AI Tools | `groundcontrol` Architecture |
|---|---|---|
| **Authoritative State** | Hidden Vector Database / Cloud Cache | **Files on Disk (Git-Tracked)** |
| **Relationship Extraction** | Stochastic LLM prompt pipelines | **Deterministic Tree-sitter AST & Link Parsers** |
| **Index Lifecycle** | Proprietary state, brittle to corruption | **100% Disposable & Rebuildable** |
| **Memory Safety** | Python C-extensions, unverified memory | **Pure Safe Rust (`forbid(unsafe)`)** |
| **Data Privacy** | Cloud embeddings / External API calls | **100% Local Inference & DirectML** |
