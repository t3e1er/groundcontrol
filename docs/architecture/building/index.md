---
title: "Building & Deployment Hub"
description: "Installation guides, native binary setup, Cargo builds, IDE integration, and multi-agent server deployment."
category: "building"
status: "active"
tags: ["installation", "cli", "build", "mcp", "cargo", "directml", "deployment"]
related:
  - "[[docs/index]]"
  - "[[docs/architecture/building/installation]]"
  - "[[docs/architecture/building/build-from-source]]"
  - "[[docs/architecture/building/client-setup]]"
  - "[[docs/architecture/building/daemon-and-server]]"
---

# Building & Deployment Hub

`groundcontrol` (`gc`) is delivered as a **single standalone native binary** with zero external runtime dependencies. This hub covers installing prebuilt release binaries, compiling from source with hardware acceleration, configuring coding agents, and deploying multi-corpus daemons.

---

## Navigation & Guides

* **[[docs/architecture/building/installation]]**: One-command platform installers for Windows, macOS, and Linux, with pre-bundled ONNX sidecar models.
* **[[docs/architecture/building/build-from-source]]**: Compiling from source via Cargo, MSRV 1.80 verification, DirectML/ONNX hardware acceleration, and fast indexing mode.
* **[[docs/architecture/building/client-setup]]**: Drop-in MCP configuration for Cursor, Claude Desktop, Antigravity IDE, Gemini CLI, Windsurf, and Zed via `groundcontrol install -y`.
* **[[docs/architecture/building/daemon-and-server]]**: Running the shared auto-daemon, hosting multi-corpus servers over HTTP SSE, and CLI automation.

---

## Platform Support Matrix

| Platform | Architecture | Hardware Acceleration | Binary Distribution |
|---|---|---|---|
| **Windows** | `x86_64` | DirectML (DirectX 12 Compute, AMD/NVIDIA/Intel) | Precompiled `.zip` & `install.ps1` |
| **macOS** | `aarch64` (Apple Silicon) | CoreML / Metal / Accelerate SIMD | Precompiled `.tar.gz` & `install.sh` |
| **Linux** | `x86_64` | OpenVINO / CPU AVX-512 / CUDA | Precompiled `.tar.gz` & `install.sh` |
| **Linux** | `aarch64` | NEON SIMD | Precompiled `.tar.gz` & `install.sh` |

---

## Quick Reference Commands

```bash
# 1. One-command installer (Windows)
irm https://raw.githubusercontent.com/t3e1er/groundcontrol/master/install.ps1 | iex

# 2. One-command installer (macOS / Linux)
curl -fsSL https://raw.githubusercontent.com/t3e1er/groundcontrol/master/install.sh | sh

# 3. Auto-configure coding agents
groundcontrol install -y

# 4. Verify installation & health
groundcontrol status --scope all
```
