---
title: "ADR 003: Leiden Connectivity-Refined Community Detection over Louvain"
category: "data-science"
status: "accepted"
tags: ["adr", "graph", "leiden", "louvain", "modularity", "decision"]
related:
  - "[[docs/data-science/index]]"
  - "[[docs/data-science/community-detection-modularity]]"
---

# ADR 003: Leiden Connectivity-Refined Community Detection over Louvain

## Status
Accepted / Implemented

## Context
To provide architectural intelligence via `get_architecture` and `graph_communities`, `groundcontrol` clusters cross-modal Petgraph graphs (AST code symbols + markdown documentation).

The classical Louvain algorithm suffers from a known pathological flaw: it can produce **internally disconnected communities**. Two completely unrelated components that have no path between them may be assigned the same community ID if both share weak indirect ties to common central hub nodes.

## Decision
We adopted the **Leiden Community Detection Algorithm** as the default algorithm in `graph/community.rs` and `get_architecture`, while retaining raw Louvain as an opt-in parameter (`algorithm="louvain"`).

### Rationale:
1. **Guaranteed Connected Components**: Leiden splits Louvain candidate communities into internally connected components before re-aggregating, ensuring every detected architectural community is topologically cohesive.
2. **Deterministic Partitioning**: Recomputing modularity deterministically eliminates non-deterministic community boundary jitter across index updates.
3. **Pure Rust Execution**: Implemented directly over Petgraph without external C-runtime or Python dependencies.

## Consequences

### Positive
- Subsystem clustering reported by `get_architecture` accurately reflects true software architecture boundaries.
- Blast radius calculations for `detect_changes` are bounded to genuinely connected modules.

### Trade-offs
- Leiden requires an additional node refinement step, increasing graph partitioning time by ~15% relative to raw Louvain (still executing in under 50ms on 20,000 node graphs).
