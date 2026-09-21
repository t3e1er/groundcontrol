---
title: "Reciprocal Rank Fusion (RRF) Mathematics"
description: "Mathematical formulation, parameter k=60 selection, and rank combination proofs in groundcontrol."
category: "search"
status: "active"
tags: ["rrf", "math", "rank-fusion", "cormack", "algorithms", "information-retrieval"]
related:
  - "[[docs/concepts/search/index]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
  - "[[docs/architecture/adr/adr-001-rrf-vs-learned-fusion]]"
---

# Reciprocal Rank Fusion (RRF) Mathematics

To merge disparate score distributions (BM25 unbounded log-odds vs cosine similarity in $[-1, 1]$ vs PageRank probabilities), linear score normalization is notoriously brittle.

`groundcontrol` utilizes **Reciprocal Rank Fusion (RRF)** (Cormack, Clarke, and Büttcher, 2009).

* **Source Implementation**: [`crates/groundcontrol-core/src/search/rrf.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/search/rrf.rs)

---

## Formal Formulation

Let $M$ be the set of retrieval modalities (e.g. $M = \{\text{BM25}, \text{Vector}, \text{Graph}\}$). For a given document $d$ and ranking model $m \in M$, let $r_m(d) \in \{1, 2, \dots, N\}$ denote the ordinal rank of document $d$ within the top-ranked results of model $m$.

If document $d$ is not present in the top candidates of model $m$, $r_m(d) = \infty$.

The RRF score $S_{\text{RRF}}(d)$ is defined as:

$$S_{\text{RRF}}(d) = \sum_{m \in M} \frac{1}{k + r_m(d)}$$

where $k$ is a constant smoothing factor.

---

## Why $k = 60$?

The smoothing parameter $k$ prevents highly ranked outliers in a single modality from dominating the ensemble:

1. **Rank 1 Advantage**:
   * For $k = 60$, a rank 1 hit contributes $\frac{1}{61} \approx 0.01639$.
   * A rank 10 hit contributes $\frac{1}{70} \approx 0.01428$.
2. **Consensus Over Outliers**:
   * If document $A$ ranks 1st in BM25 but is absent in Vector and Graph:
     $$S(A) = \frac{1}{61} + 0 + 0 \approx 0.01639$$
   * If document $B$ ranks 4th in BM25, 5th in Vector, and 6th in Graph:
     $$S(B) = \frac{1}{64} + \frac{1}{65} + \frac{1}{66} \approx 0.01562 + 0.01538 + 0.01515 = 0.04615$$
   * Consensus among multiple modalities easily outscores an isolated top-1 outlier ($0.04615 \gg 0.01639$).

This guarantees that documents confirmed by both lexical match and semantic similarity rise to the top of the agent's Turn 1 snippet list.
