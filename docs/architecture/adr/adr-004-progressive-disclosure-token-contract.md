---
title: "ADR 004: Strict Progressive Disclosure Token Contract (<150 Token Handles)"
category: "agentic-strategy"
status: "accepted"
tags: ["adr", "progressive-disclosure", "tokens", "ergonomics", "decision"]
related:
  - "[[docs/agentic-strategy/index]]"
  - "[[docs/agentic-strategy/progressive-disclosure]]"
---

# ADR 004: Strict Progressive Disclosure Token Contract (<150 Token Handles)

## Status
Accepted / Implemented

## Context
During live empirical token audits on the `rust` compiler corpus (5,767 files), calling `search(detail="ids")` returned ~5,931 tokens. Despite specifying `detail="ids"`, the search response leaked full `lineage` graph structures and verbose multi-component score breakdowns (`bm25_raw`, `vector_cosine`, `graph_hops`), violating the progressive disclosure contract and exhausting agent prompt limits.

## Decision
We updated `apply_detail` in `crates/groundcontrol-mcp/src/tools/mod.rs` to strictly enforce the Tier-1 handle contract:
1. When `detail == Some("ids")`, explicitly strip `snippet = None` AND `lineage = None`.
2. Explicitly suppress `score_components = None` unless `mode == "explain"`.
3. Apply this stripping uniformly across `handle_search` and `handle_search_related`.

## Consequences

### Positive
- A 5-hit search query under `detail="ids"` drops from **5,931 tokens to 128 tokens** (a 97.8% reduction).
- AI agent reasoning remains focused on handle selection without reading unrequested code bodies.
- Network JSON-RPC payloads are drastically reduced.

### Trade-offs
- Agents requiring immediate context must perform a two-step retrieval (`search` $\to$ `get_snippet`), which is the deliberate design of progressive disclosure.
