---
title: "ADR 015: Dynamic Token-Budgeting with 400ms TDR Safety Ceiling"
category: "gpu-optimization"
status: "accepted"
tags: ["adr", "gpu", "tdr", "vram", "batching", "decision"]
related:
  - "[[docs/gpu-optimization/index]]"
  - "[[docs/gpu-optimization/tdr-watchdog-resilience]]"
  - "[[docs/data-science/transformer-memory-physics]]"
---

# ADR 015: Dynamic Token-Budgeting with 400ms TDR Safety Ceiling

## Status
Accepted / Implemented

## Context
Early versions of `groundcontrol` used fixed-size batch dispatching ($B=64$). When a batch contained one or more long code files near 2,000+ tokens, the quadratic attention activation matrix ($B \times S^2$) spiked memory consumption over 12+ GB, triggering fatal DirectX 12 driver resets (`0x887A0006`) when GPU execution exceeded the Windows 2.0-second TDR threshold.

## Decision
We implemented a **Dynamic Token-Budgeting Batching Pipeline** with a strict **400ms Per-Dispatch Safety Ceiling**:
1. Sort chunks by sequence length ($S$) and dynamically pack batches such that total activation memory remains within the hardware governor's 70% available headroom.
2. Cap dispatch execution time to $\le 400\text{ms}$ (a 5x safety margin below the Windows 2.0s TDR reset window).
3. Integrate an AIMD (Additive Increase / Multiplicative Decrease) controller to dynamically adjust batch token budgets based on measured dispatch latency.

## Consequences

### Positive
- Completely eliminates Windows TDR device lost resets (`0x887A0006`).
- Eliminates GPU VRAM Out-of-Memory crashes.
- Adapts smoothly to shared desktop workloads where other applications compete for GPU time.

### Trade-offs
- Requires sorting chunks before batch construction, adding minor CPU memory staging overhead (amortized to $<2\%$ of total index runtime).
