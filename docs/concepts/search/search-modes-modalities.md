---
title: "Search Modes & Modality Filters"
description: "How to parameterize the unified search tool across modes (hybrid, bm25, semantic, graph, explain) and modalities (code, docs, both)."
category: "search"
status: "active"
tags: ["search-tool", "modes", "modalities", "bm25", "semantic", "hybrid", "explain"]
related:
  - "[[docs/concepts/search/index]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
  - "[[docs/concepts/progressive-disclosure/three-tier-model]]"
  - "[[docs/architecture/adr/adr-010-unified-modal-search-tool]]"
---

# Search Modes & Modality Filters

The authoritative `search` tool in `groundcontrol` exposes a unified interface controlled by two parameters: `mode` and `modality`.

---

## 1. Search Modes (`mode`)

| Mode | Underlying Engines | When to Use |
|---|---|---|
| **`hybrid`** *(default)* | Tantivy BM25 + ONNX Dense Vector + Petgraph | General exploratory queries, balancing exact identifiers and conceptual semantics. |
| **`bm25`** | Tantivy Okapi BM25 only | Exact struct names, function names, error strings, verbatim tokens. Runs in < 2.2ms. |
| **`semantic`** | Local 768-dim ONNX Vector space only | Abstract technical intentions, algorithmic concepts, and architectural queries. |
| **`graph`** | Petgraph & recursive SQLite CTE traversal | Graph neighborhood traversals; filtered by `edge_types` or `edge_class`. |
| **`explain`** | Full hybrid execution with telemetry | Diagnostic mode returning individual BM25, vector, and graph rank breakdowns. |

---

## 2. Modality Filtering (`modality`)

Code and documentation are indexed independently to prevent one modality from drowning out the other:
* **`modality="code"`**: Restricts search results strictly to AST code symbols and polyglot source chunks.
* **`modality="docs"`**: Restricts search results strictly to markdown notes, ADRs, RFCs, and documentation.
* **`modality="both"`** *(default)*: Executes independent ranking across code and docs, returning partitioned `docs` and `code` result lists.

---

## 3. Inline Turn 1 Snippets (`snippets`)

The `snippets` parameter dictates how many inline source snippets are bundled into the Turn 1 response:
```json
{
  "query": "CorpusManager routing",
  "mode": "hybrid",
  "modality": "code",
  "snippets": 3
}
```
Setting `snippets: 3` (default) inlines the top 3 symbol implementations directly into Turn 1, answering 70%+ of agent inquiries in a single round-trip.
Setting `snippets: 0` performs a fast handle sweep when the agent only needs file paths.
