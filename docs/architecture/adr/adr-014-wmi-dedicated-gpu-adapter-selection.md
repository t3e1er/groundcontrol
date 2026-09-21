---
title: "ADR 014: Automated Dedicated GPU Adapter Selection via CIM/WMI"
category: "gpu-optimization"
status: "accepted"
tags: ["adr", "gpu", "wmi", "cim", "directml", "adapter-selection", "decision"]
related:
  - "[[docs/gpu-optimization/index]]"
  - "[[docs/gpu-optimization/directml-vendor-neutrality]]"
---

# ADR 014: Automated Dedicated GPU Adapter Selection via CIM/WMI

## Status
Accepted / Implemented

## Context
On multi-GPU Windows workstations and laptops (e.g. Intel Core CPU with integrated Intel HD Graphics alongside a dedicated NVIDIA GeForce GTX GPU), DirectML by default binds to **Adapter 0**. On many systems, Adapter 0 is the integrated Intel GPU with shared system RAM. This caused `groundcontrol` to exhaust iGPU memory and crawl at sluggish inference speeds while the high-performance dedicated GPU sat completely idle.

## Decision
We implemented **Automated Dedicated GPU Selection** in safe Rust:
1. Allow manual override via the `CTX_DEVICE_ID` environment variable if specified by the user.
2. If unset, query Windows system video controllers via CIM/WMI (`Win32_VideoController`), inspecting dedicated `AdapterRAM`.
3. Automatically bind DirectML execution to the device ID with the greatest dedicated VRAM (e.g. selecting Device ID 1 NVIDIA GTX 1070 with 8 GB VRAM over Device ID 0 Intel HD Graphics with 128 MB dedicated RAM).

## Consequences

### Positive
- Workstations and laptops automatically leverage their most powerful graphics hardware without requiring manual environment variable flags or configuration.
- Eliminates iGPU VRAM exhaustion and driver crashes on dual-adapter machines.

### Trade-offs
- Adds a lightweight CIM query (~15ms) during cold engine startup on Windows.
