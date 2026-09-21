---
title: "DirectML Acceleration & AIMD GPU Governor"
description: "Vendor-neutral DirectX 12 GPU compute, VRAM protection ceilings, double-buffered dispatch, and TDR resilience."
category: "implementation"
status: "active"
tags: ["directml", "gpu", "aimd", "vram", "tdr", "double-buffered"]
related:
  - "[[docs/architecture/implementation/index]]"
  - "[[docs/concepts/search/embeddings-vector]]"
  - "[[docs/architecture/adr/adr-013-directml-vendor-neutral-acceleration]]"
  - "[[docs/architecture/adr/adr-015-dynamic-token-budgeting-tdr-safety]]"
---

# DirectML Acceleration & AIMD GPU Governor

To avoid locking users into proprietary CUDA runtimes, `groundcontrol` uses **DirectX 12 DirectML** on Windows. This enables hardware acceleration across all modern discrete and integrated GPUs (AMD, Intel, NVIDIA, Qualcomm).

---

## 1. Dynamic AIMD Memory Governor

Batching large token sequences into a GPU can cause Out-Of-Memory (OOM) crashes if another application (e.g. an IDE or browser) consumes VRAM.

`groundcontrol` implements an **Additive Increase / Multiplicative Decrease (AIMD)** hardware governor:
* **VRAM Ceiling**: Caps total process VRAM allocation at **70%** of available adapter memory.
* **Dynamic Batch Scaling**:
  * If batch latency < 150ms and VRAM < 70%: increases token batch size additively ($+64$ tokens).
  * If batch latency spikes or VRAM > 70%: cuts batch size multiplicatively ($\times 0.5$).

---

## 2. Double-Buffered Pipeline Dispatch

Indexing large codebases requires reading files from disk, tokenizing via HuggingFace tokenizers, and running ONNX tensor passes.

`groundcontrol` uses a double-buffered staging pipeline:
```
Stage 1 (CPU Threadpool):   [Tokenize Batch N+1] ──┐
                                                    ▼
Stage 2 (GPU DirectML):     [Compute Tensor Batch N]
```
While the GPU processes Batch $N$, CPU worker threads tokenize and pack tensors for Batch $N+1$. This keeps GPU compute saturation above 85% during full-corpus indexing.

---

## 3. Windows TDR Watchdog Safety

On Windows, the GPU scheduler triggers a **Timeout Detection and Recovery (TDR)** reset (error `0x887A0006`) if any single compute dispatch occupies the GPU for more than 2 seconds.

`groundcontrol` enforces a strict **400ms per-dispatch execution ceiling**. Long document sequences are automatically chunked and staged across multiple micro-dispatches, guaranteeing zero Windows desktop freezes or driver crashes.
