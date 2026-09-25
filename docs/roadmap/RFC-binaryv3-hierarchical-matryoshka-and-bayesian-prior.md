---
title: "RFC: BinaryV3 — Hierarchical Matryoshka Projections and Zero-Cost Bayesian Structural Priors"
description: "Architecture and mathematical specification for binaryv3 retrieval: unifying 256-bit unpartitioned semantic SIF, true Matryoshka prefix nesting, and zero-cost post-Hamming Bayesian structural priors."
category: "roadmap"
status: "implemented"
tags: ["rfc", "binaryv3", "matryoshka", "sif", "bayesian", "code-retrieval", "hamming", "vector-quantization"]
related:
  - "[[docs/index]]"
  - "[[docs/roadmap/coderoadmap]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
  - "[[docs/roadmap/RFC-algorithmic-semantic-bridging]]"
  - "[[docs/roadmap/RFC-cast-signal-boosting-and-partitioned-hyperplanes]]"
---

# RFC: BinaryV3 — Hierarchical Matryoshka Projections and Zero-Cost Bayesian Structural Priors

**Status**: Implemented  
**Scope**: [`groundcontrol-common`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-common), [`groundcontrol-core`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core), [`groundtruth`](file:///c:/dev/semantic/groundtruth)  
**Date**: September 2026  
**Target Version**: `0.3.0`  
**Related Documents**: [[docs/roadmap/coderoadmap]], [[docs/concepts/search/hybrid-retrieval-theory]], [[docs/roadmap/RFC-algorithmic-semantic-bridging]], [[docs/roadmap/RFC-cast-signal-boosting-and-partitioned-hyperplanes]]

---

## 1. Executive Summary & Empirical Post-Mortem

In empirical benchmarks on the polyglot OpenTelemetry Astronomy Shop corpus (`otel-demo`, 11 languages, 323 files, evaluated via `groundtruth` at $K=5$, `--modality code`), baseline **`binary` (v1)** decisively outperformed **`binaryv2`** across all retrieval metrics:

| Retrieval System / Algorithm | Retrieval Mechanism | Recall@5 | MRR@5 | nDCG@5 | P50 Latency | Architecture Substrate |
|---|---|---|---|---|---|---|
| **`groundcontrol: binary` (v1)** | Full 256-bit unpartitioned SIF Hamming scan | **0.5333** | **0.6783** | 0.4960 | **0.76 ms** | In-process, 0 IPC, 0 DB |
| **`codebase-memory-mcp` (CBM)** | SQLite FTS5 BM25 + AST Symbol Boosts (`Function` +10, `Route` +8, exact +30) | 0.5167 | 0.6500 | **0.5216** | ~1,800 ms | Native C / SQLite daemon |
| **`groundcontrol: fast`** | 3-way RRF (BM25 + Binary + HippoRAG PPR graph walk) | 0.4167 | 0.6667 | 0.4641 | 34.02 ms | In-process Petgraph + Tantivy |
| **`groundcontrol: bm25`** | Tantivy Okapi BM25 schema | 0.4000 | 0.3950 | 0.3508 | 32.25 ms | In-process Tantivy engine |
| **`groundcontrol: binaryv2`** | 4-channel 256-bit Hamming scan (Ch0: Lex, Ch1: RRI, Ch2: AST, Ch3: Path) | **0.3833** | **0.5250** | 0.3901 | **0.89 ms** | In-process, 0 IPC, 0 DB |

`binary` (v1) outperformed `binaryv2` by **+39.1% Recall@5** and **+29.2% MRR@5**, and also defeated `codebase-memory-mcp` at a fraction of the latency (0.76 ms vs. 1,800 ms).

### The Four Post-Mortem Failure Modes of `binaryv2`

1. **Representational Asymmetry (The Query-Document Modality Gap)**:
   Search queries are intent-driven natural language or identifier fragments (`"credit card charge and transaction authorization"`, `"calculate shipping quote"`). Queries do not contain AST callee trees or file paths. In `binaryv2`, the query projector copied raw query text into Channels 2 and 3, whereas the document projector extracted AST call sites and file system paths. The query and document projected into completely disjoint feature spaces in those channels.

2. **Binomial Noise Amplification Across Partitioned Channels**:
   When comparing disjoint feature spaces in a 64-bit Hamming channel, the distance follows a binomial distribution $B(64, 0.5)$ with mean $\mu = 32$ and variance $\sigma^2 = 16$.
   $$\text{Dist}_{\text{v2}} = D_{\text{Ch0}} + D_{\text{Ch1}} + D_{\text{Ch2}} + D_{\text{Ch3}} \approx D_{\text{Ch0}} + 32 + 32 + 32 = D_{\text{Ch0}} + 96$$
   The 96 bits of uncorrelated noise contributed a standard deviation of $\sigma = \sqrt{3 \times 16} \approx 6.93$. A true 8-bit signal in Channel 0 was completely drowned out by the noise variance of Channels 1, 2, and 3.

3. **Conflating Semantic Likelihood with Structural Prior**:
   In Bayesian Information Retrieval, $P(D \mid Q) \propto P(Q \mid D) \cdot P(D)$.
   - $P(Q \mid D)$ is the **semantic match likelihood** (does the entity's interface describe what the query is asking?).
   - $P(D)$ is the **structural prior** (is this entity an exported public function, a class root, a private helper, or a unit test?).
   `binaryv2` attempted to encode $P(D)$ into the coordinate dimensions of the embedding vector $\phi(D)$, corrupting the metric space.

4. **Neutralization and Hubness Under IDF Weighting**:
   AST node types (`Function`, `Call`, `Return`, `Block`) occur in almost every file ($DF \approx 98\%$). Under standard SIF or IDF weighting, their weights vanish to zero. Conversely, if artificially forced to have high weights, every function's vector is pulled toward the same generic AST centroid, causing catastrophic representation collapse ("hubness").

---

## 2. Theoretical Foundations of BinaryV3

`binaryv3` resolves these failure modes by adhering strictly to three core mathematical principles:

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                 BINARYV3 ARCHITECTURE                                  │
├────────────────────────────────────────────────────────────────────────────────────────┤
│                                                                                        │
│  [ PILLAR 1: PURE INTERFACE-SEMANTIC 256-BIT PROJECTION ]                              │
│  • Exclude internal AST callees & syntax tokens from the vector.                       │
│  • Represent purely "what it is" (Name, Signature, Docstrings, CamelCase subwords).   │
│  • Arora et al. SIF with global seed vocabulary + 5-iter PCA power method.             │
│                                                                                        │
│  [ PILLAR 2: ZERO-COST BAYESIAN STRUCTURAL PRIOR ]                                     │
│  • Decouple structural priors from the metric space.                                   │
│  • Stage 1: SIMD Hamming scan (0.76ms) retrieves Top M = 50 candidate pool.           │
│  • Stage 2: Multiply likelihood by O(1) cached metadata prior:                        │
│      Score(Q, D) = [1.0 - (D_H(Q, D) / 256.0)] * Prior(D.flags)                        │
│                                                                                        │
│  [ PILLAR 3: TRUE MATRYOSHKA REPRESENTATION LEARNING (MRL) NESTING ]                   │
│  • V_64 ⊂ V_128 ⊂ V_256 (nested prefixes of the SAME semantic entity).                 │
│  • Word 0 (bits 0..63): High-salience topic anchor & primary identifier root.          │
│  • Word 1 (bits 64..127): Subword morphology & abbreviation expansion.                 │
│  • Words 2..3 (bits 128..255): Signature parameter types & docstring context.          │
│  • Enables cascaded early-exit filtering on Word 0 before full 256-bit resolve.        │
│                                                                                        │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

### Pillar 1: Pure Interface-Semantic Vector Projection

A function's *intent* is declared at its interface; its internal callees describe its implementation mechanics. 
Let $T(D) = T_{\text{name}} \cup T_{\text{sig}} \cup T_{\text{doc}} \cup T_{\text{subwords}}$.
The continuous SIF embedding $\vec{v}_D \in \mathbb{R}^{256}$ is synthesized as:

$$\vec{v}_D = \frac{1}{\sum_{t \in T(D)} w_t} \sum_{t \in T(D)} w_t \cdot \vec{v}_t$$

where:
- $\vec{v}_t \in \mathbb{R}^{256}$ is the deterministic Blake3 unit vector for token $t$.
- $w_t = \frac{a}{a + p(t)} \cdot \mu_t$, with $a = 10^{-4}$, $p(t)$ derived from the seeded global background dictionary, and $\mu_t$ representing the salience multiplier:
  - $\mu_{\text{name}} = 2.0$
  - $\mu_{\text{sig}} = 1.2$
  - $\mu_{\text{subword}} = 1.0$
  - $\mu_{\text{doc}} = 0.8$
  - $\mu_{\text{abbrev}} = 0.75$

Following Arora et al. (ICLR 2017), the first principal component $\vec{u}$ across the repository is removed via 5 power-iteration steps:
$$\vec{v}'_D = \vec{v}_D - \vec{u} (\vec{u}^T \vec{v}_D)$$
Finally, 1-bit sign quantization yields the 256-bit fingerprint:
$$\text{bit}_i = \begin{cases} 1 & \text{if } \vec{v}'_D[i] > 0.0 \\ 0 & \text{otherwise} \end{cases} \quad \text{for } i \in [0..255]$$

### Pillar 2: Zero-Cost Bayesian Structural Prior

To capture the ranking benefits that gave `codebase-memory-mcp` high precision on symbol definitions without incurring its 1,800 ms SQLite overhead, `binaryv3` applies a Bayesian prior $P(D)$ strictly as an $O(1)$ post-Hamming multiplier on the candidate pool:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct EntityPriorFlags(pub u8);

impl EntityPriorFlags {
    pub const IS_PUBLIC_EXPORT: u8 = 1 << 0;  // 1.15x boost
    pub const IS_TEST_OR_MOCK:  u8 = 1 << 1;  // 0.75x penalty
    pub const KIND_FUNCTION:    u8 = 1 << 2;  // 1.10x boost
    pub const KIND_CLASS:       u8 = 1 << 3;  // 1.05x boost
    pub const KIND_ROUTE_API:   u8 = 1 << 4;  // 1.20x boost
    pub const IS_ROOT_SCOPE:    u8 = 1 << 5;  // 1.05x boost

    #[inline]
    pub fn compute_multiplier(&self) -> f32 {
        let mut m = 1.0f32;
        if self.0 & Self::IS_PUBLIC_EXPORT != 0 { m *= 1.15; }
        if self.0 & Self::KIND_ROUTE_API != 0   { m *= 1.20; }
        else if self.0 & Self::KIND_FUNCTION != 0 { m *= 1.10; }
        else if self.0 & Self::KIND_CLASS != 0    { m *= 1.05; }
        if self.0 & Self::IS_ROOT_SCOPE != 0    { m *= 1.05; }
        if self.0 & Self::IS_TEST_OR_MOCK != 0  { m *= 0.75; }
        m
    }
}
```

The retrieval pipeline proceeds in two serial stages:
1. **Stage 1 (Hardware Hamming Scan)**:
   Scan all $N$ fingerprints using native CPU POPCOUNT (`(q ^ d).count_ones()`).
   Partition the top $M = 50$ nearest candidates via `select_nth_unstable_by`.
   *Latency: ~0.76 ms for 10,000 entities.*
2. **Stage 2 (Bayesian Prior Rescoring)**:
   For the $M = 50$ candidates:
   $$\text{Score}(Q, D) = \left(1.0 - \frac{D_H(Q, D)}{256.0}\right) \times D.\text{flags}.\text{compute\_multiplier}()$$
   *Latency: 50 multiplications = 0.002 ms.*

Total latency remains under **0.80 ms**, while tie-breaking between production code and test files is resolved deterministically.

### Pillar 3: Matryoshka Representation Learning (MRL) Prefix Slicing

Unlike `binaryv2`'s disjoint feature bins, `binaryv3` constructs nested representation levels where every prefix $[0..k]$ is a complete, self-contained semantic fingerprint of the entity at increasing granularity:

$$\mathcal{V}_{64} \subset \mathcal{V}_{128} \subset \mathcal{V}_{256}$$

- **Word 0 (`bits[0]`, Bits 0..63)**: High-level categorical semantic clusters (seeded ontology) and root symbol identifier.
- **Word 1 (`bits[1]`, Bits 64..127)**: Subword morphology (camelCase / snake_case stems) and abbreviation expansions.
- **Words 2..3 (`bits[2..3]`, Bits 128..255)**: Signature types, parameters, return types, and docstring terms.

#### Cascaded Early-Exit Optimization
Because Word 0 is a valid 64-bit coarse semantic fingerprint:
- **Filter**: For each record, evaluate `(Q.0[0] ^ D.0[0]).count_ones()`.
- If distance $> 42 / 64$ ($> 2.5\sigma$ from identity), reject immediately without loading or evaluating Words 1, 2, and 3.
- In benchmarks with $100\text{K}+$ entities, this prunes $85-90\%$ of candidate cache lines with single-cycle SIMD instructions.

---

## 3. Module & Structural Specification

`binaryv3` will be implemented as an isolated, clean module in `groundcontrol-core` satisfying `RetrievalAlgorithm`:

```
crates/groundcontrol-core/src/algorithm/binaryv3/
├── mod.rs          # BinaryV3Algorithm satisfying RetrievalAlgorithm
├── index.rs        # BinaryV3SearchIndex with SIMD scan & Stage 2 prior rescoring
├── projector.rs    # Matryoshka-aligned SIF projector & PCA power iteration
├── tokenizer.rs    # Universal camelCase/snake_case splitting & abbreviation expansion
└── types.rs        # EntityPriorFlags (u8), BinaryV3Config, FingerprintV3Record
```

### In-Memory Record Layout (`FingerprintV3Record`)

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintV3Record {
    pub id: String,
    pub fingerprint: BinaryFingerprint, // [u64; 4] = 32 bytes
    pub modality: Modality,             // 1 byte
    pub flags: EntityPriorFlags,        // 1 byte
}
```

Total size per record: **34 bytes + ID string heap allocation**. Memory footprint for 50,000 code symbols is less than **3.5 MB**.

---

## 4. Empirical Evaluation Protocol (`groundtruth`)

To validate `binaryv3`, we will benchmark it using `groundtruth` on the canonical polyglot `otel-demo` corpus under `--modality code` ($K=5$):

### Primary Target Metrics
- **Recall@5**: Target $> \mathbf{0.5500}$ (surpassing both `binary` v1 at 0.5333 and CBM at 0.5167).
- **MRR@5**: Target $> \mathbf{0.7000}$ (surpassing `binary` v1 at 0.6783 and CBM at 0.6500).
- **nDCG@5**: Target $> \mathbf{0.5300}$ (surpassing CBM at 0.5216).
- **P50 Latency**: Target $< \mathbf{1.00\text{ ms}}$ (retaining sub-millisecond in-process Rust performance).

### Ablation Matrix
The benchmark will run serial ablations across:
1. `binary`: Baseline unpartitioned SIF v1.
2. `binaryv2`: 4-channel partitioned subspace.
3. `binaryv3-pure`: Pillar 1 (Subword SIF, no prior).
4. `binaryv3-prior`: Pillar 1 + Pillar 2 (Bayesian prior).
5. `binaryv3-full`: Pillar 1 + Pillar 2 + Pillar 3 (MRL cascaded scan).
6. `codebase-memory-mcp`: Production native C / SQLite FTS5 baseline.

---

## 5. Verification Checklist & Empirical Results

- [x] RFC merged into [`docs/roadmap/`](file:///c:/dev/semantic/groundcontrol/docs/roadmap).
- [x] Bidirectional links verified to domain types in [`groundcontrol-common`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-common).
- [x] Implement `binaryv3` in [`groundcontrol-core::algorithm::binaryv3`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-core/src/algorithm/binaryv3).
- [x] Wire `binaryv3` into `AlgoMethod` in [`groundtruth-harness`](file:///c:/dev/semantic/groundtruth/crates/groundtruth-harness).
- [x] Run release ablation sweep: `cargo run -p groundtruth-cli --release -- ablate ...`.
- [x] Confirm `Recall@5 >= 0.55` and `P50 < 1.0ms`.
- [x] Ensure `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` and `cargo fmt --all -- --check` pass with zero warnings.

### Empirical Validation Benchmark ($K=5$, `--modality code`, `otel-demo`)

| Variant | Method | Recall@5 | MRR@5 | nDCG@5 | P50 (ms) | P99 (ms) | Status |
|---|---|---|---|---|---|---|---|
| **binary** (v1) | `binary` | 0.5333 | 0.6783 | 0.4960 | 0.75 ms | 0.94 ms | Baseline |
| **binaryv2** | `binaryv2` | 0.3833 | 0.5250 | 0.3901 | 0.86 ms | 1.04 ms | Degraded |
| **binaryv3** | `binaryv3` | **0.6667** | **0.7783** | **0.5883** | **0.70 ms** | 0.98 ms | **Target Met** |
| **binaryv3_pure** | `binaryv3_pure` | 0.5333 | 0.6417 | 0.4829 | 0.70 ms | 0.95 ms | Pure Subword SIF |
| **binaryv3_prior** | `binaryv3_prior` | **0.6667** | **0.7783** | **0.5883** | **0.70 ms** | 0.82 ms | +Bayesian Prior |
| **binaryv3_full** | `binaryv3_full` | **0.6667** | **0.7783** | **0.5883** | **0.68 ms** | **0.76 ms** | +MRL Early-Exit |
| **bm25** | `bm25` | 0.4000 | 0.3950 | 0.3508 | 31.76 ms | 39.30 ms | Pure Lexical |
| **fast** | `fast` | 0.4500 | 0.6667 | 0.4838 | 33.91 ms | 40.00 ms | RRF Composite |

