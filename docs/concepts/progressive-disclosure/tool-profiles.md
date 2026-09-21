---
title: "Role-Based Tool Profiles"
description: "Gating the 17 authoritative MCP tools via --profile scout, analysis, and all."
category: "progressive-disclosure"
status: "active"
tags: ["profiles", "mcp-tools", "scout", "analysis", "all", "security"]
related:
  - "[[docs/concepts/progressive-disclosure/index]]"
  - "[[docs/concepts/progressive-disclosure/swarm-topologies]]"
  - "[[docs/architecture/implementation/mcp-transport]]"
  - "[[docs/architecture/adr/adr-006-role-based-tool-profiling]]"
---

# Role-Based Tool Profiles

Exposing 17 tools to every agent can cause cognitive distraction. An exploratory scout agent might mistakenly attempt to write files, or an analytical reader might trigger re-indexing.

`groundcontrol` solves this using **Role-Based Tool Profiles** configured via the `--profile` CLI argument.

---

## Profile Tiers & Tool Inventories

| Profile | Tool Count | Tools Included | Typical Use Case |
|---|---|---|---|
| **`scout`** | 6 | `search`, `search_related`, `get_snippet`, `read_file`, `list_notes`, `status` | Read-only discovery, fast information sweeps, junior assistants. |
| **`analysis`** | 11 | `scout` tools + `graph_match`, `graph_communities`, `validate`, `list_templates`, `list_corpora` | Architectural reviewers, dependency audits, read-only graph traversal. |
| **`all`** *(default)* | 17 | Full suite including mutating writes: `write_note`, `delete_note`, `move_note`, `sync_corpus`, `index_corpus`, `unload_corpus` | Senior pair programmers, autonomous coding agents, crystallizers. |

---

## Authoritative Tool Inventory (17 Tools)

```
1. read_file           [Read]      Polymorphic batch reader with line slicing [start, end]
2. get_snippet         [Read]      Bounded AST code symbol or doc chunk fetcher
3. list_notes          [Read]      Catalog listing & note frontmatter inspector
4. search              [Search]    Tier 1 hybrid retrieval (BM25 + Vector + Graph + Snippets)
5. search_related      [Search]    Personalized PageRank & entity association
6. graph_match         [Graph]     Linear Cypher-Lite ASCII query compiled to SQLite CTE
7. graph_communities   [Graph]     Leiden & Louvain architectural community detection
8. write_note          [Write]     Schema-validated authoring (create, overwrite, append, prepend)
9. delete_note         [Write]     Permanent note removal and index cleanup
10. move_note          [Write]     Refactor note paths with automatic wikilink updating
11. validate           [Validate]  Frontmatter schema, broken link & taxonomy validator
12. list_templates     [Validate]  Discover active templates in .templates/
13. status             [System]    Unified corpus statistics, graph density, coverage
14. list_corpora       [System]    List active mounted corpora in CorpusManager
15. sync_corpus        [System]    Incremental delta, full re-index, or re-embed
16. index_corpus       [System]    Mount and initialize a new corpus root dynamically
17. unload_corpus      [System]    Unmount a corpus root from the runtime engine
```

---

## Configuration Example

```bash
# Launch read-only scout server
groundcontrol --corpus /path/to/repo --profile scout

# Launch read-only architecture analyzer
groundcontrol --corpus /path/to/repo --profile analysis
```
