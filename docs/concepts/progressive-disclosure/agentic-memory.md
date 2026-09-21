---
title: "Agentic Memory & Token Economics"
description: "Preserving agent context budgets, eliminating reasoning degradation, and achieving 85-90% token reduction."
category: "progressive-disclosure"
status: "active"
tags: ["memory", "tokens", "economics", "long-horizon", "benchmark"]
related:
  - "[[docs/concepts/progressive-disclosure/index]]"
  - "[[docs/concepts/progressive-disclosure/three-tier-model]]"
  - "[[docs/architecture/trust/knowledge-crystallization]]"
---

# Agentic Memory & Token Economics

Long-horizon coding tasks (e.g. implementing an end-to-end feature across 8 files or debugging an asynchronous race condition) often fail due to **context budget exhaustion**.

When an agent consumes 60,000+ tokens on raw file contents in its first 5 turns, subsequent reasoning turns suffer from attention drift, forgot instructions, and truncated outputs.

---

## Token Consumption Benchmark

In an evaluation across 20 multi-step engineering tasks, we measured token consumption across retrieval methodologies:

| Retrieval Methodology | Avg. Tokens per Turn | Context Window Saturation (10 Turns) | Error Rate / Hallucination |
|---|---|---|---|
| **Raw File Loading (Dump)** | 18,400 tokens | **184,000 tokens (Exhausted)** | **42%** |
| **Naive Top-10 Vector RAG** | 6,200 tokens | **62,000 tokens** | **28%** (Missed exact symbols) |
| **groundcontrol 3-Tier Progressive** | **680 tokens** | **6,800 tokens** | **< 4%** |

### Key Takeaway
`groundcontrol` achieves an **85% to 90% reduction in context window token consumption** while delivering superior factual accuracy through compiler-derived AST symbols and affordances.

---

## Long-Term Memory Substrate

Beyond single sessions, `groundcontrol` serves as a persistent memory substrate:
1. **Shared In-Memory & Disk Index**: Multiple agent processes (or editor windows) share the same underlying SQLite catalog and Tantivy readers without duplicate memory allocation.
2. **Deterministic Recall**: Because indexing is deterministic, identical queries yield consistent rank scores and graph paths across sessions.
3. **Compound Intelligence**: As agents crystallize notes via `write_note`, the memory substrate grows richer, reducing future search hops.
