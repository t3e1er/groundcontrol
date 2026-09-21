---
title: "Architectural Decision Records (ADR Catalog)"
description: "Authoritative catalog of architectural decisions governing groundcontrol design and invariants."
category: "adr"
status: "active"
tags: ["adr", "decisions", "architecture", "invariants", "history"]
related:
  - "[[docs/index]]"
---

# Architectural Decision Records (ADR Catalog)

All architectural design decisions in `groundcontrol` are formally recorded as **Architectural Decision Records (ADRs)** with context, alternatives considered, decision rationale, and verified consequences.

---

## Authoritative ADR Registry

| ADR # | Title | Topic Domain |
|---|---|---|
| **[[docs/architecture/adr/adr-001-rrf-vs-learned-fusion]]** | Reciprocal Rank Fusion vs Learned Rankers | Search & Ranking Theory |
| **[[docs/architecture/adr/adr-002-jina-code-768d-selection]]** | Selection of Jina Code v2 768d ONNX Embedding Model | Data Science & Vector Spaces |
| **[[docs/architecture/adr/adr-003-leiden-louvain-graph-clustering]]** | Leiden & Louvain Community Detection Algorithms | Graph Analytics & Modularity |
| **[[docs/architecture/adr/adr-004-progressive-disclosure-token-contract]]** | 3-Tier Progressive Disclosure Token Contract | Agentic Strategy & Context Budgets |
| **[[docs/architecture/adr/adr-005-deterministic-vs-llm-graph-extraction]]** | Deterministic Grammar Extraction vs LLM Entity Extractors | Trust & Knowledge Graph Fidelity |
| **[[docs/architecture/adr/adr-006-role-based-tool-profiling]]** | Role-Based Tool Profiling (`--profile scout\|analysis\|all`) | Agent Surface Isolation |
| **[[docs/architecture/adr/adr-007-hexagonal-ports-adapters-isolation]]** | Hexagonal Ports & Adapters Architecture | Systems Engineering & Encapsulation |
| **[[docs/architecture/adr/adr-008-anchor-embedding-paradigm]]** | Anchor Embedding Paradigm for Sub-second Search | Embedding Acceleration |
| **[[docs/architecture/adr/adr-009-greenfield-no-backwards-compat]]** | Greenfield Discipline: Zero Backwards Compatibility Shims | Codebase Hygiene & Zero Dead Code |
| **[[docs/architecture/adr/adr-010-unified-modal-search-tool]]** | Unified Modal Search Tool Ergonomics | MCP Tool Design |
| **[[docs/architecture/adr/adr-011-readonly-readwrite-handler-model]]** | ReadOnly vs ReadWrite Handler Concurrency Model | MCP Concurrency & Engine Locks |
| **[[docs/architecture/adr/adr-012-in-process-multi-corpus-manager]]** | In-Process Multi-Corpus Manager (`CorpusManager`) | Multi-Repository Serving |
| **[[docs/architecture/adr/adr-013-directml-vendor-neutral-acceleration]]** | Vendor-Neutral GPU Acceleration via DirectML | Hardware Acceleration & DirectX 12 |
| **[[docs/architecture/adr/adr-014-wmi-dedicated-gpu-adapter-selection]]** | Dedicated Discrete GPU Adapter Selection | Hardware Discovery & DXGI |
| **[[docs/architecture/adr/adr-015-dynamic-token-budgeting-tdr-safety]]** | Dynamic Token Budgeting & Windows TDR Watchdog Safety | GPU Stability & Driver Safety |
| **[[docs/architecture/adr/adr-016-generic-normalized-scope-resolution]]** | Generic Normalized Scope Resolution in AST Trees | Tree-sitter cAST Chunking |
| **[[docs/architecture/adr/adr-017-docs-embed-intermediate-indexing-mode]]** | Intermediate Docs-Embed Indexing Mode | Pipeline Optimization |
| **[[docs/architecture/adr/adr-018-file-skeleton-map-chunking]]** | File Skeleton Map Chunking for Skeleton Mode | Embedding Volume & Throughput |
| **[[docs/architecture/adr/adr-019-file-level-rrf-fusion]]** | File-Level RRF Fusion Key for Code Search | Search Correctness & Signal Fusion |
| **[[docs/architecture/adr/adr-020-lean-multiline-text-emission]]** | Lean Multiline Text Emission Protocol Across Progressive Disclosure Turns | Token Efficiency & MCP Wire Protocol |
