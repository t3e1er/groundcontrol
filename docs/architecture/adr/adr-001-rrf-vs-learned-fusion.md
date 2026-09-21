---
title: "ADR 001: Reciprocal Rank Fusion vs Learned Score Combination"
category: "data-science"
status: "accepted"
tags: ["adr", "rrf", "ranking", "data-science", "decision"]
related:
  - "[[docs/data-science/index]]"
  - "[[docs/data-science/rrf-mathematics]]"
  - "[[docs/data-science/hybrid-retrieval-theory]]"
---

# ADR 001: Reciprocal Rank Fusion vs Learned Score Combination

## Status
Accepted / Implemented

## Context
In building a 4-modality hybrid retrieval pipeline (Tantivy BM25, HNSW dense vectors, Petgraph graph hops), `groundcontrol` needed an algorithm to merge heterogeneous candidate score distributions into a single, authoritative ranked list.

Two primary paradigms were evaluated:
1. **Parametric Linear / Learned Score Normalization**:
   $$S(d) = \alpha \cdot \text{MinMax}(S_{\text{BM25}}) + \beta \cdot S_{\text{cosine}} + \gamma \cdot S_{\text{graph}}$$
2. **Non-Parametric Reciprocal Rank Fusion (RRF)**:
   $$R(d) = \sum_{m} \frac{w_m}{k + r_m(d)}$$

## Decision
We adopted **Reciprocal Rank Fusion (RRF)** with canonical smoothing constant $k = 60$ and rejected learned/parametric score combinations.

### Rationale:
1. **Zero Hyperparameter Calibration**: In polyglot software environments, BM25 scores vary from $0.5$ to $45.0+$ depending on corpus size and term rarity. Learned weights $\alpha, \beta, \gamma$ trained on one corpus (e.g. Rust documentation) systematically fail on another (e.g. Kubernetes Go code). RRF requires zero training and zero score calibration.
2. **Robustness to Extreme Outliers**: A single anomalous cosine score or huge BM25 spike cannot skew the final ranking. Candidates that appear consistently in the top ranks across multiple modalities dominate the result set.
3. **Sub-millisecond Compute**: Evaluating $R(d)$ requires simple ordinal addition over candidate rank lists, executing in $<0.2\text{ms}$ in safe Rust.

## Consequences

### Positive
- Cross-corpus multi-root fan-out queries merge seamlessly with no domain-specific score normalization.
- The retrieval pipeline behaves deterministically regardless of underlying corpus size.
- Implementation is clean, minimal, and dependency-free.

### Trade-offs
- RRF discards raw score margins (the difference between rank 1 and rank 2 is identical whether score delta was 0.001 or 0.5). In practice, this trade-off significantly improves retrieval robustness.
