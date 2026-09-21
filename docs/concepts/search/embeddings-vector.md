---
title: "Dense ONNX Embeddings & Vector Space"
description: "Local 768-dimensional Jina Code v2 embeddings, DirectML acceleration, and HNSW vector search."
category: "search"
status: "active"
tags: ["embeddings", "onnx", "jina", "directml", "hnsw", "vector-search"]
related:
  - "[[docs/concepts/search/index]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
  - "[[docs/architecture/implementation/gpu-and-directml]]"
  - "[[docs/architecture/adr/adr-002-jina-code-768d-selection]]"
---

# Dense ONNX Embeddings & Vector Space

To capture abstract technical intentions and conceptual queries, `groundcontrol` embeds a local 768-dimensional dense vector model.

* **DirectML Provider**: [`crates/groundcontrol-core/src/embedding/directml.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/embedding/directml.rs)
* **HNSW Vector Store**: [`crates/groundcontrol-core/src/vector/hnsw.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/vector/hnsw.rs)
* **EmbeddingProvider Port**: [`crates/groundcontrol-common/src/ports.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/ports.rs)

---

## Model Selection: `jina-embeddings-v2-base-code`

* **Architecture**: 768-dimensional asymmetric bi-encoder optimized for code and technical documentation.
* **Context Length**: Up to 8,192 tokens per chunk.
* **License**: Apache-2.0.
* **Format**: Pure ONNX runtime execution (`ort`), bundled directly as a sidecar.

---

## Hardware Acceleration: DirectML & SIMD

Rather than requiring proprietary NVIDIA CUDA toolchains, `groundcontrol` leverages **DirectX 12 DirectML** on Windows and native SIMD/CoreML on macOS/Linux:
* **Vendor-Neutral GPU Compute**: Runs natively on AMD Radeon, Intel Arc, NVIDIA GeForce, and Qualcomm Snapdragon GPUs.
* **Adaptive AIMD Governor**: Dynamically monitors GPU VRAM to maintain a safe 70% memory ceiling, preventing driver watchdog timeouts (TDR `0x887A0006`).
* **Instant Fallback**: If no compatible GPU adapter is detected, the runtime gracefully falls back to AVX-512 / NEON CPU SIMD inference.

---

## Approximate Nearest Neighbor (HNSW)

Dense vectors are indexed in an in-memory Hierarchical Navigable Small World (HNSW) graph:
* **Distance Metric**: Cosine similarity.
* **Graph Parameters**: $M = 16$, $efConstruction = 200$, $efSearch = 64$.
* **Query Latency**: < 5ms for top-50 vector candidate retrieval.
