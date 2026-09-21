---
title: "Progressive Disclosure & Agentic Strategy Hub"
description: "Eliminating context rot and token waste through strict 3-tier retrieval, Turn 1 affordances, and swarm memory."
category: "progressive-disclosure"
status: "active"
tags: ["progressive-disclosure", "agents", "tokens", "affordances", "swarms", "memory"]
related:
  - "[[docs/index]]"
  - "[[docs/concepts/progressive-disclosure/three-tier-model]]"
  - "[[docs/concepts/progressive-disclosure/turn-1-affordances]]"
  - "[[docs/concepts/progressive-disclosure/agentic-memory]]"
  - "[[docs/concepts/progressive-disclosure/swarm-topologies]]"
  - "[[docs/concepts/progressive-disclosure/tool-profiles]]"
  - "[[docs/concepts/progressive-disclosure/graphview-lod]]"
---

# Progressive Disclosure & Agentic Strategy Hub

In agentic software development, **context window pollution is the leading cause of reasoning degradation**. When an agent receives hundreds of lines of irrelevant source code, its ability to reason accurately drops exponentially.

`groundcontrol` eliminates context rot through **Progressive Disclosure**—a formal contract that bounds token usage at every step of an agent's reasoning loop.

---

## Navigation & Core Topics

* **[[docs/concepts/progressive-disclosure/three-tier-model]]**: The foundational 3-tier contract (Tier 1 search handles $\to$ Tier 2 bounded symbols $\to$ Tier 3 line slices).
* **[[docs/concepts/progressive-disclosure/turn-1-affordances]]**: How Turn 1 search returns inline snippets and graph degree counts (`calls_in`, `calls_out`, `implements`) for immediate answers.
* **[[docs/concepts/progressive-disclosure/agentic-memory]]**: Preserving token budgets across long-running tasks, achieving 85–90% token reduction.
* **[[docs/concepts/progressive-disclosure/swarm-topologies]]**: Orchestrating specialized agent roles (Scouts, Readers, Writers, Crystallizers).
* **[[docs/concepts/progressive-disclosure/tool-profiles]]**: Gating tool exposure via `--profile scout|analysis|all` (17 authoritative tools).
* **[[docs/concepts/progressive-disclosure/graphview-lod]]**: Visual progressive disclosure: 4-tier 3D Level-of-Detail (LOD) for 1M+ node knowledge graphs.

---

## Token Efficiency Comparison

```
Traditional RAG (Full File / Large Chunk Dumping):
┌──────────────────────────────────────────────────────────┐
│ 15,000 - 45,000 tokens dumped into Turn 1 context window │  --> High Cost, Reasoning Degradation
└──────────────────────────────────────────────────────────┘

groundcontrol 3-Tier Progressive Disclosure:
┌──────────────────────────────────────────────────────────┐
│ Turn 1: 300 - 800 tokens (Snippets + Graph Affordances)  │  --> 85-90% Token Savings
├──────────────────────────────────────────────────────────┤
│ Turn 2 (Optional): 200 - 600 tokens (Exact Symbol AST)   │  --> High Precision
└──────────────────────────────────────────────────────────┘
```
