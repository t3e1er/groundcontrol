---
title: "Building from Source"
description: "Compiling groundcontrol from source, managing Rust MSRV 1.80, DirectML dependencies, and fetching models."
category: "building"
status: "active"
tags: ["rust", "cargo", "msrv", "compilation", "directml", "onnx", "fast-mode"]
related:
  - "[[docs/architecture/building/index]]"
  - "[[docs/architecture/building/installation]]"
  - "[[docs/architecture/trust/pure-rust-invariants]]"
  - "[[docs/architecture/adr/adr-013-directml-vendor-neutral-acceleration]]"
---

# Building from Source

`groundcontrol` compiles using standard `cargo` workflows with **zero external C library dependencies**.

---

## Prerequisites

1. **Rust Toolchain**: Rust **1.80.0+** (MSRV pinned in `rust-toolchain.toml`).
2. **Just Command Runner** (Optional but recommended):
   ```bash
   cargo install just
   ```
3. **Git LFS / Model Weights**:
   The ONNX embedding sidecar is downloaded via a `just` task or script.

---

## Compilation Steps

### 1. Clone the Repository
```bash
git clone https://github.com/t3e1er/groundcontrol.git
cd groundcontrol
```

### 2. Fetch Embedding Model Sidecar
```bash
just fetch-model
# This downloads jina-embeddings-v2-base-code into ./models/
export CTX_MODELS_DIR="$(pwd)/models"
```

### 3. Build the CLI Binary
```bash
cargo build --workspace --release --locked
```
The optimized native binary is produced at:
`target/release/groundcontrol` (or `target/release/groundcontrol.exe`).

---

## Build Features & Fast Mode

### `--fast` (BM25 + Graph Only)
If you do not want to download the 768-dimensional ONNX embedding model or wish to index large source repositories instantly:
```bash
groundcontrol --corpus /path/to/repo --fast
```
Fast mode runs purely on Tantivy Okapi BM25 and Petgraph cAST syntax graphs. Indexing completes in seconds with zero vector model overhead.

### Quality Verification (`just ci`)
Run the complete workspace quality gate locally:
```bash
just check          # cargo check across all features and targets
just test           # 156+ unit, integration, and e2e tests
just clippy         # strict clippy lints with -D warnings
just fmt-check      # rustfmt verification
```
