---
title: "Project Roadmap & RFC Archive"
description: "High-level technical roadmap, feature RFCs, and evolutionary specifications for groundcontrol."
category: "roadmap"
status: "active"
tags: ["roadmap", "rfc", "planning", "evolution", "architecture"]
related:
  - "[[docs/index]]"
  - "[[docs/architecture/adr/index]]"
  - "[[docs/architecture/implementation/index]]"
---

# Project Roadmap & RFC Archive

This section tracks the technical roadmap and Request for Comments (RFC) engineering specifications for `groundcontrol`.

---

## Technical Roadmap

* **[[docs/roadmap/coderoadmap]]**: The comprehensive engineering roadmap, covering completed phases and upcoming milestones for graph scaling, distributed swarms, and multi-modal expansion.

---

## Architectural RFCs

| RFC | Title | Status | Primary Focus |
|---|---|---|---|
| **[[docs/roadmap/RFC-cross-corpus-graph-federation]]** | Cross-Corpus Graph Federation | Implemented | Multi-repo routing, external symbol resolution, federated BFS traversal. |
| **[[docs/roadmap/RFC-adaptive-graph-expansion]]** | Adaptive Graph Expansion & SQL Backend | Implemented | Recursive SQLite CTEs, bounded depths, and cycle protection for sub-2ms queries. |
| **[[docs/roadmap/RFC-zero-copy-file-offsets-and-binary-vectors]]** | Zero-Copy File Offsets & Packed Vectors | Implemented | Eliminating memory duplication with aligned binary vectors and disk byte offsets. |
| **[[docs/roadmap/RFC-treesitter-expansion-and-lsp-analysis]]** | Tree-sitter Polyglot AST & Language Expansion | Implemented | cAST chunking across 16+ languages with parent scope breadcrumb injection. |
| **[[docs/roadmap/RFC-docs-embed-intermediate-indexing-mode]]** | Intermediate Docs-Embed Indexing Mode | Implemented | Fast indexing mode prioritizing markdown doc embeddings over raw code vectors. |
| **[[docs/roadmap/RFC-markdown-templates-and-frontmatter-edge-schema]]** | Markdown Templates & Frontmatter Edge Schema | Proposed | Native .templates/*.md standard with frontmatter schema, edge synthesis, and scaffolding. |
| **[[docs/roadmap/RFC-lean-multiline-text-emission]]** | Lean Multiline Text Emission Protocol | Implemented | Eliminating JSON context overhead via indented Cypher ASCII trees and markdown blocks (~69% token savings). |
| **[[docs/roadmap/RFC-graphview]]** | Standalone 3D GraphView & Multi-Agent Activation Substrate | Implementing | Zero-overhead sidecar 3D visualization, 1M+ node binary protocol, and real-time SSE agent telemetry. |
| **[[docs/roadmap/RFC-algorithmic-semantic-bridging]]** | Algorithmic Semantic Bridging & Code Graph Synthesis | Proposed | Sub-minute CPU-only semantic graph bridging (11 signals, RoTSQ 4-bit, RRI) for 100K+ file codebases. |
| **[[docs/roadmap/RFC-sota-code-retrieval-and-semantic-bridging]]** | SOTA Code Retrieval & High-Throughput Semantic Bridging | Proposed (Next Up) | Sub-second CPU semantic bridging (SIF, 256-bit MRL binary embeddings, AST pattern injection, Query-Time PPR) for 100K+ files. |
| **[[docs/roadmap/RFC-document-extractors-and-projections]]** | Pluggable Document Extractors & Derived Projections | Proposed | Pure-Rust ingestion for Word (.docx), PDF (.pdf), and HTML (.html) via disposable Derived Text Projections and modality disambiguation. |
| **[[docs/roadmap/RFC-cast-signal-boosting-and-partitioned-hyperplanes]]** | cAST Structural Signal Boosting & Partitioned Hyperplanes | Proposed | Eliminating Bag-of-Words loss via AST role tagging, tree-depth attenuation, def-use flow synthesis, and 3-channel hyperplanes. |

