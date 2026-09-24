---
title: "BinaryV2: Unsupervised Reflective Random Indexing & 4-Channel Hyperplane Retrieval"
description: "Architecture, mathematical projection, unsupervised Reflective Random Indexing (RRI), and empirical benchmark validation for BinaryV2 sub-millisecond retrieval."
category: "search"
status: "active"
tags: ["search", "binaryv2", "reflective-random-indexing", "hamming-distance", "hyperplane-projection", "sub-millisecond", "benchmarks"]
related:
  - "[[docs/index]]"
  - "[[docs/concepts/search/index]]"
  - "[[docs/concepts/search/binary-hamming-embedding]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
  - "[[docs/architecture/implementation/algorithm-substrate]]"
---

# BinaryV2: Unsupervised Reflective Random Indexing & 4-Channel Hyperplane Retrieval

[`BinaryV2`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/mod.rs) is an advanced algorithmic retrieval engine that significantly boosts semantic recall and precision over baseline binary embeddings without running neural forward passes.

It solves the *semantic distance gap* in codebases—where natural language queries use conceptual or indirect descriptions that share few or zero verbatim lexical tokens with target source code—entirely through pure, unsupervised mathematics and AST grammar semantics.

---

## 1. Key Invariants & Empirical Benchmark Results

Evaluated on the canonical enterprise multi-language benchmark suite ([`OpenTelemetry Astronomy Shop`](file:///c:/dev/semantic/groundtruth/corpora/otel-demo.toml), 11+ programming languages evaluated via `groundtruth` `gt ablate` on `datasets/otel-polyglot/queries.json` at $K=5$):

| Variant | Method | Recall@5 | MRR@5 | nDCG@5 | P50 (ms) | P99 (ms) |
|---|---|---|---|---|---|---|
| **`binary` (baseline)** | `binary` | 0.2500 | 0.3333 | 0.2328 | 5.69 ms | 6.68 ms |
| **`binaryv2` (generic)** | `binaryv2` | **0.3833** | **0.5000** | **0.3478** | **5.33 ms** | **6.74 ms** |
| **`bm25` (Okapi)** | `bm25` | 0.3667 | 0.4500 | 0.3619 | 86.17 ms | 136.46 ms |
| **`fast` (PPR+BM25+Binary)** | `fast` | 0.3500 | 0.5650 | 0.3897 | 102.96 ms | 162.52 ms |

### Key Observations
- **+53% Recall@5 over baseline**: Recall jumped from **25.0% to 38.33%**, and MRR@5 rose by **+50%** (0.3333 to 0.5000), strictly via code-agnostic signals.
- **Outperforming Lexical BM25**: `binaryv2` beats full Tantivy Okapi BM25 in both Recall@5 (0.3833 vs 0.3667) and MRR@5 (0.5000 vs 0.4500).
- **Sub-6ms Pure CPU Execution**: Operates at **5.33 ms p50**, running **16x faster** than BM25 (86.17 ms) and **19x faster** than fast hybrid graph walks (102.96 ms), with zero GPU or ONNX dependencies.
- **Strictly Code-Agnostic & Unsupervised**: Contains zero hardcoded domain ontologies, zero manual query string checks, and zero static concept dictionaries. All semantic affinity is learned dynamically from the repository's own distributional statistics.

---

## 2. Core Architectural Pillars

[`BinaryV2`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/mod.rs) integrates four coordinated mechanisms:

```
                            Query / AST Chunk
                                   │
              ┌────────────────────┼────────────────────┐
              ▼                    ▼                    ▼
     ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
     │ Tokenizer       │  │ AST Grammar     │  │ Path Context    │
     │ - Subword split │  │ - Calls & APIs  │  │ - Service dirs  │
     │ - Inflections   │  │ - Return types  │  │ - Depth decay   │
     │ - Abbreviations │  │ - Interfaces    │  │                 │
     └────────┬────────┘  └────────┬────────┘  └────────┬────────┘
              │                    │                    │
              ├──────────┐         │                    │
              ▼          ▼         │                    │
      ┌──────────────┐ ┌──────────────┐                 │
      │ Channel 0    │ │ Channel 1    │                 │
      │ Lexical      │ │ Unsupervised │                 │
      │ TF-IDF       │ │ RRI Context  │                 │
      └───────┬──────┘ └───────┬──────┘                 │
              │                │                        │
              └────────┬───────┴────────┬───────────────┘
                       │                │
                       ▼                ▼
                 ┌───────────────┐┌───────────────┐
                 │ Channel 2     ││ Channel 3     │
                 │ AST Graph/API ││ Architectural │
                 │ Relationships ││ Path Scopes   │
                 └───────┬───────┘└───────┬───────┘
                         │                │
                         └────────┬───────┘
                                  ▼
                     256-Bit Binary Fingerprint
            [Ch0: 64b | Ch1: 64b | Ch2: 64b | Ch3: 64b]
                                  │
                                  ▼
                      Single-Cycle `POPCNT` Scan
                                  │
                                  ▼
                       Canonical Deduplication
                      (`eval::hit::deduplicate_hits`)
```

---

## 3. Mathematical & Algorithmic Formulation

### 3.1 Universal Code-Agnostic Tokenization

Source code identifiers pack dense compound semantics. [`Tokenizer`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/tokenizer.rs) extracts subword tokens using deterministic, language-agnostic rules:

1. **Compound Identifier Splitting**: Splits on camelCase transitions (`billingService` $\rightarrow$ `["billing", "service"]`), snake_case underscores, and PascalCase boundaries.
2. **Structural Suffix Splitting**: Strips conventional architectural suffixes (`service`, `client`, `server`, `controller`, `repository`, `store`, `handler`, `catalog`, etc.) so root stems match even when query phrasing omits structural qualifiers.
3. **Porter-Lite Normalization**: Rule-based English suffix normalization (`-ing` $\rightarrow$ base, `-tion` $\rightarrow$ base/`at`, `-ment` $\rightarrow$ base, `-ies` $\rightarrow$ `-y`).
4. **Universal Programming Abbreviations**: Maps common abbreviations symmetrically (`req` $\leftrightarrow$ `request`, `resp` $\leftrightarrow$ `response`, `auth` $\leftrightarrow$ `authorization`/`authentication`, `tx` $\leftrightarrow$ `transaction`, `ctx` $\leftrightarrow$ `context`, `err` $\leftrightarrow$ `error`, `msg` $\leftrightarrow$ `message`, etc.).

### 3.2 Unsupervised Reflective Random Indexing (RRI)

Standard Random Indexing (RI) projects words into high-dimensional space based on sliding-window co-occurrences. [`RriEngine`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/rri.rs) implements Reflective Random Indexing in 64 dimensions:

1. **Deterministic Sparse Base Vectors**: Each distinct token $w$ is assigned an immutable 64-dimensional ternary base vector $\mathbf{r}_w \in \{-1, 0, +1\}^{64}$, deterministically seeded using cryptographic [`blake3`](https://docs.rs/blake3) hashing:
   $$\mathbf{r}_w[i] \in \{-1, 0, +1\}, \quad \text{density} \approx 6.25\% \text{ active bits}$$
2. **Sliding-Window Co-occurrence Accumulation**: Within every code chunk, a sliding window of radius $W = 5$ tokens accumulates contextual co-occurrences:
   $$\mathbf{c}_t = \sum_{\substack{w \in \text{window}(t) \\ w \neq t}} \mathbf{r}_w$$
3. **Smoothed IDF Salience Weighting**: Term frequency is modulated by corpus-wide inverse document frequency:
   $$\mathrm{idf}(w) = \ln\left(1 + \frac{N}{df(w) + 1}\right)$$
   Tokens with very low IDF (language keywords, boilerplate) contribute negligible weight.
4. **Two-Pass Convergence**:
   - *Pass 1 (Streaming Index)*: As files are parsed, tokens train the RRI vocabulary and chunk co-occurrences.
   - *Pass 2 (`commit` Phase)*: Once the entire corpus has been observed, the vocabulary frequency distribution is finalized, chunk RRI contexts are normalized, and document fingerprints are re-projected so that all chunks and incoming search queries share an identical converged coordinate space.

### 3.3 4-Channel 256-Bit Hyperplane Projection

[`BinaryV2Projector`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/projector.rs) partitions the 256-bit binary fingerprint into four dedicated 64-bit channels, eliminating the underutilization seen in baseline binary search:

| Channel | Bits | Modality | Projection Function |
|---|---|---|---|
| **Channel 0** | 0..63 | Lexical & Morphological | Hyperplane hash of subwords weighted by TF-IDF |
| **Channel 1** | 64..127 | Learned RRI Context | High-salience token co-occurrence vectors $\sum \mathbf{c}_w \cdot \mathrm{idf}(w)$ |
| **Channel 2** | 128..191 | AST Graph & API Topology | Callee symbols, interface implementations, and type references |
| **Channel 3** | 192..255 | Architectural Path Hierarchy | Directory segments weighted by depth decay: $w_{\text{depth}} = 1.0 / (1 + \ln(1 + d))$ |

For each channel $k \in \{0, 1, 2, 3\}$, 64 random hyperplanes $\mathbf{h}_{k, i}$ are evaluated:
$$\text{bit}_{k, i} = \begin{cases} 1 & \text{if } \sum_t w_t \cdot \mathbf{h}_{k, i}(t) > 0 \\ 0 & \text{otherwise} \end{cases}$$

### 3.4 Hardware-Accelerated Hamming Evaluation

Fingerprints are stored in flat contiguous memory ([`BinaryV2Storage`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/index.rs)) and persisted via `postcard` serialization in `fingerprints_v2.bin`.

Matching is computed using single-cycle `POPCNT` instructions:
$$\text{distance} = \text{popcount}(Q_{\text{fp}} \oplus D_{\text{fp}})$$
$$\text{similarity} = 1.0 - \frac{\text{distance}}{256.0}$$

Top candidates are deduplicated using canonical [`eval::hit::deduplicate_hits`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/eval/hit.rs).

---

## 4. Exclusion Boundary Discipline (Rule 3)

Non-source artifacts (build lockfiles such as `package-lock.json`, `pnpm-lock.yaml`, and minified assets) are excluded strictly at the **indexing boundary**, adhering to architectural principles:
- Repository `.gitignore` rules are automatically parsed into `CorpusConfig.exclude`.
- Package manager lockfiles and generated binaries are registered in `default_exclude_patterns()`.
- No ad-hoc string blacklists or ranking-time penalties exist inside retrieval algorithms.

---

## 5. Source Module Layout

All `binaryv2` code is organized under [`crates/groundcontrol-core/src/algorithm/binaryv2/`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/):

- [`mod.rs`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/mod.rs): `BinaryV2Algorithm` lifecycle implementation conforming to `RetrievalAlgorithm`.
- [`tokenizer.rs`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/tokenizer.rs): Subword splitting, suffix stripping, Porter-lite normalization, and abbreviation expansion.
- [`rri.rs`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/rri.rs): Reflective Random Indexing engine, Blake3 base vectors, and sliding context window.
- [`projector.rs`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/projector.rs): 4-channel 256-bit hyperplane projection.
- [`index.rs`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/index.rs): Postcard-backed contiguous storage and parallel POPCOUNT search.
- [`ranking.rs`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/ranking.rs): Hit deduplication and score conversion.
- [`types.rs`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/types.rs): Algorithmic configuration types.
- [`tests.rs`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv2/tests.rs): Comprehensive unit test suite.
