---
title: "Turn 1 Affordances & Schema Envelopes"
description: "How Turn 1 search responses ground agents with graph degree counts, node labels, and inline answers."
category: "progressive-disclosure"
status: "active"
tags: ["affordances", "graph-degrees", "schema-envelope", "turn-1", "search"]
related:
  - "[[docs/concepts/progressive-disclosure/index]]"
  - "[[docs/concepts/progressive-disclosure/three-tier-model]]"
  - "[[docs/concepts/search/graph-traversal]]"
---

# Turn 1 Affordances & Schema Envelopes

In single-turn search, standard vector databases return only chunk text and similarity floats. The agent has no idea if a returned function is a leaf utility, an entry point called by 50 modules, or an outdated duplicate.

`groundcontrol` enriches every Turn 1 search result with **Graph Affordances** and a dynamic **Schema Envelope**.

---

## 1. Cypher-Lite Graph Affordances

Every search hit includes deterministic graph relationships and degree previews formatted in compact Cypher-Lite ASCII notation directly from the AST knowledge graph:

```json
{
  "path": "crates/groundcontrol-core/src/bundle.rs",
  "score": 0.0161,
  "score_components": { "bm25": 13.05 },
  "snippet": "pub fn detect_bundle(corpus_root: &Path) -> Option<PathBuf> { ... }",
  "graph": "<-[:calls*3]-(add_corpus_with_index_dir, ensure_corpus_with_name, prompt_bundle_extraction), <-[:defines]-(bundle.rs)"
}
```

### Why Cypher-Lite Affordances Matter
* **Directional Relationship Topology**: `<-[:calls*3]-(...)` immediately shows callers without having to guess or make exploratory queries.
* **Neighborhood Density & Suppression**: High-degree hubs display preview neighbors plus explicit suppression notices (e.g. `<-[:defines*20 (suppressed 17)]-(...)`), preventing token blowouts while signaling structural centrality.
* **Zero Redundant Tokens**: Language is derived from file extension, entity kind is implicit from snippet and symbol definitions, and trailing hits (where `snippet: null`) provide `symbol` handles for Turn 2 progressive disclosure (`get_snippet(name="...")` or `graph_match(...)`).

---

## 2. Dynamic Schema Envelope

Every search response also bundles the **Schema Envelope** for the indexed corpus:

```json
{
  "schema_envelope": {
    "node_labels": ["CodeSymbol", "DocNode", "Module", "Tag"],
    "edge_types": ["calls", "defines", "imports", "implements", "wikilink", "derived_from"]
  }
}
```

This prevents hallucinated Cypher-Lite queries in Turn 2. The agent knows exactly which node labels and edge types exist before constructing patterns for `graph_match`.
