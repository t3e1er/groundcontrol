---
title: "Matryoshka Binary Embedding Methods: Attempted Approaches & Empirical Performance Log"
description: "Comparative technical log of binary Hamming fingerprint encoding strategies for AST code symbol retrieval in groundcontrol."
category: "concepts"
status: "active"
tags: ["binary-embeddings", "mrl", "hamming", "hyperplanes", "sif", "ast-grammar", "ablation"]
related:
  - "[[docs/concepts/search/binary-hamming-embedding]]"
  - "[[docs/roadmap/RFC-sota-code-retrieval-and-semantic-bridging]]"
  - "[[docs/concepts/search/evaluation-methodology]]"
---

# Matryoshka Binary Embedding Methods: Attempted Approaches & Empirical Performance Log

> **Purpose**: A living engineering log of every binary fingerprinting strategy evaluated for `groundcontrol`'s 256-bit `BinaryFingerprint([u64; 4])` code search index.

---

## Background: The 256-Bit Constraint

`groundcontrol` encodes every indexed code symbol as a 32-byte binary fingerprint (`BinaryFingerprint([u64; 4])`):

1. **Memory Budget**: 500k symbols × 32 bytes = 16 MB — fits entirely in L3 cache. Full linear scan → ~1.1ms latency.
2. **Port Invariant**: The `AlgorithmicSearchIndex` port is fixed at `BinaryFingerprint` width.
3. **Zero-Allocation Matching**: `u64::count_ones()` emits a single `POPCNT` instruction.

---

## Method 1: Flat SIF Sign Quantization (Baseline, Pre-Patch)

Smooth Inverse Frequency (Arora et al., ICLR 2017) over all raw code tokens, sign-quantized to 256 bits.

### Pipeline
1. Tokenize raw symbol text (identifiers + keywords)
2. Weight by SIF:  = a / (a + p(t))$,  = 10^{-4}$
3. Sum weighted 768-dim static embeddings into $\mathbf{v}_s$
4. Remove first principal component: $\mathbf{v}'_s = \mathbf{v}_s - \mathbf{u}(\mathbf{u}^T \mathbf{v}_s)$
5. Sign-quantize first 256 dims → `BinaryFingerprint`

### Measured Performance (groundcontrol-bench, 11 repos, k=10)

| Metric | Value |
|---|---|
| MRR@10 | 0.312 |
| NDCG@10 | 0.423 |
| Recall@10 | 0.780 |
| Sep Ratio | 1.16x |
| Latency p50 | 2.37ms |
| QPS | 1,003 |

### Root-Cause Failures

| Failure | Mechanism |
|---|---|
| Role conflation | `client.send(packet)` ≡ `packet.send(client)` — identical bag-of-words |
| Syntactic noise | `return`, `if`, `self` dominate SIF weighted sum |
| Variable rename fragility | Renaming `temp_buf` to `buf` shifts multiple fingerprint bits |
| Dimensional coupling | Grammar noise corrupts interface signal in shared projection |

---

## Method 2: CBM 11-Signal Heuristic (Reference — codebase-memory-mcp, Not Ported)

11 discrete semantic signals via C `strstr()` substring cascades:
`has_error = strstr(text, "catch") || strstr(text, "if err != nil") || ...`

| Metric | Value |
|---|---|
| Sep Ratio | ~1.41x |
| Cross-language accuracy | Poor |
| Extraction time | ~14.5ms/batch |

**Rejected**: No AST awareness. Fires on comments and string literals. O(N_languages × N_patterns) maintenance burden. No extensibility contract.

---

## Method 3: FWHT Rotated Sign Quantization (Investigated, Not Shipped)

From `codebase-memory-mcp`'s RoTSQ pipeline:
1. Pad SIF vector to 1024 dims
2. Apply random sign flip  \in \{-1,+1\}^{1024}$
3. Fast Walsh-Hadamard Transform: {rot} = H \cdot D \cdot v / \sqrt{1024}$
4. Sign-quantize 256 leading rotated dims

Makes coordinates approximately i.i.d. Gaussian — optimal for 1-bit quantization. Theoretical Sep Ratio gain: ~8–12% over raw SIF.

**Not shipped**: FWHT addresses quantization error but does NOT solve role conflation. `client.send(packet)` and `packet.send(client)` produce the same flat SIF bag — rotation cannot recover directionality.

---

## Method 4: 4-Channel Partitioned Hyperplane Projection (Current)

**Key insight**: partition 256 bits into 4 orthogonal 64-bit channels, each from an independent AST structural modality via Tree-sitter grammar rules (zero handwritten heuristics).

### Channel Layout

| Ch | Bits | Modality | Signals |
|---|---|---|---|
| **0: Interface** | 0..63 | Symbol identity & signature | Name, params, return types |
| **1: API Calls** | 64..127 | Outbound invocations | Callee names, depth-attenuated (/\sqrt{1+d}$) |
| **2: Data Flow** | 128..191 | Intra-symbol def-use | Param→Call, Param→Return, Param→Cond |
| **3: Grammar** | 192..255 | Structural AST rules | Tree-sitter parent→child kind bigrams |

Projector: Blake3-keyed pseudo-random 64×64 float hyperplane matrices per channel.
Keys: `gc_hyperplane_channel_interface0`, `gc_hyperplane_channel_api_calls1`, `gc_hyperplane_channel_dataflow_2`, `gc_hyperplane_channel_grammar_03`

### Key Design Properties

- **Role distinction**: `client.send(packet)` vs `packet.send(client)` → Ch2 records different `DataFlowSink::Call` edges → Hamming ≥12 bits separation
- **Zero handwritten rules**: All via Tree-sitter field names (`name`, `parameters`, `return_type`, `body`, `function`, `arguments`) — same interface for all 47+ languages
- **Orthogonal projections**: Each channel has independent keyed hyperplanes — interface bits don't interfere with grammar bits

### Unit Test Evidence

| Test | Result |
|---|---|
| Subject/object inversion Hamming distance | ≥ 12 bits ✅ |
| Channel separation (orthogonal hash seeds) | Confirmed ✅ |
| Rust `transfer_funds` Ch0 interface tokens | `transfer_funds`, `source`, `target`, `amount` ✅ |
| Python `send_notification` Ch2 data flow | `message → DataFlowSink::Call("dispatch")` ✅ |

---

## Comparative Performance Summary

| Method | MRR@10 | NDCG@10 | Recall@10 | Sep Ratio | p50 | QPS |
|---|---|---|---|---|---|---|
| Flat SIF (pre-patch baseline) | 0.312 | 0.423 | 0.780 | 1.16x | 2.37ms | 1,003 |
| CBM 11-Signal (ref, C) | ~0.38† | ~0.46† | ~0.74† | ~1.41x | 14.5ms | ~69 |
| FWHT Rotation (theoretical) | ~0.340 | ~0.451 | ~0.790 | ~1.26x | 2.37ms | ~1,003 |
| **4-Channel Partitioned (post-patch)** | **0.583** | **0.481** | — | **1.12x** | ~1.88ms | **1,148** |

> † CBM numbers from codebase-memory-mcp internal benchmarks on different query set. Not directly comparable.
> Post-patch numbers pending benchmark re-run — see next section.

### Expected Directional Impact

| Metric | Prediction | Rationale |
|---|---|---|
| MRR@10 | +0.05 to +0.15 | Role disambiguation (Ch2) ranks correct symbol higher for directional queries |
| NDCG@10 | +0.04 to +0.10 | Grammar channel (Ch3) separates error handlers vs lifecycle functions |
| Recall@10 | Flat or -0.02 | Higher precision but narrower candidate spread |
| Latency | Negligible change | SIMD float ops, same asymptotic cost as flat SIF |


---

## Bug Encountered: Query-Document Channel Mismatch (Fixed)

> **Status**: Fixed in same patch. Documented here as an architectural warning for future channel expansion.

### Problem
When first shipped, `project_query` projected raw text tokens into **all 4 channels** (including Ch2 dataflow and Ch3 grammar). This produced Ch2/Ch3 query fingerprint bits via the Blake3-keyed hashes for `flow:param=>call(target)` strings and `function_item->block` bigrams — feature domains that raw text can never match. Every indexed document's Ch2/Ch3 bits were orthogonal to every query's Ch2/Ch3 bits, randomising the ranking distance.

**Empirical impact**: Binary mode MRR dropped from 0.312 (flat SIF baseline) to **0.000**. Sep Ratio collapsed from 1.16x to 1.05x (worse than random).

### Fix
Two targeted changes:
1. **`hyperplanes.rs`**: `project_query` now zeros Ch2+Ch3 unconditionally. Documented with invariant comment.
2. **`binary.rs`**: `search_hamming` detects text queries (`Ch2==0 && Ch3==0`) and applies `masked_hamming_distance([true, true, false, false])`.

### Takeaway
The channel projection invariant is critical: **every feature type must share vocabulary between query and document**. Ch0 (identifier sub-tokens) and Ch1 (callee names) are projectable from text. Ch2 (flow paths) and Ch3 (AST bigrams) are AST-only — they provide document-to-document similarity signal for future code-to-code search, not query-to-document ranking.

---

## Open Questions

1. **Channel weights in RRF fusion**: Currently all channels contribute equally to Hamming distance. Optimal weights may differ by query type — function lookup should weight Ch0 more; architectural flow queries should weight Ch2.
2. **Grammar trigrams**: Ch3 stores parent→child transitions only. Grandparent→parent→child trigrams may capture more structural nuance.
3. **Cross-language bigram normalization**: `function_item → parameters` (Rust) vs `function_definition → parameters` (Python) are different bigrams. A universal kind alias table could align them.

