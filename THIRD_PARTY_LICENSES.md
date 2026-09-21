# Third-Party Software & Dependency License Notices

This project (`groundcontrol` / `gc`) is licensed under the **MIT License**.

The binary distributions include static linkages of third-party open-source libraries. In compliance with open-source license agreements, this document provides the attributions and license terms for all dependencies in the software supply chain.

All dependencies in this codebase are strictly verified via automated CI policy gates (`cargo-deny`) against an enterprise-approved permissive license allow-list.

---

## 1. Supply Chain License Allow-List & Conformance

Every transitive crate in the dependency graph adheres to one or more of the following standard open-source licenses:

- **MIT License / MIT OR Apache-2.0**: (e.g., `serde`, `tokio`, `clap`, `fastembed`, `petgraph`, `postcard`, `tracing`, `blake3`, `hnsw_rs`, `pulldown-cmark`)
- **Apache License 2.0 / Apache-2.0 WITH LLVM-exception**: (e.g., `tantivy`, `axum`, `tower`, `rustls`, `tokio-rustls`, `hyper`)
- **BSD 2-Clause / BSD 3-Clause**: (e.g., `notify`)
- **ISC License**: (e.g., `rustls-webpki`)
- **Unicode-3.0 / Unicode-DFS-2016**: (e.g., `unicode-ident`, `unicode-normalization`)
- **Zlib License**: (e.g., `miniz_oxide`, `flate2`)

**Zero Copyleft / Restrictive Licenses**: No GPL, AGPL, LGPL, SSPL, or non-commercial licenses are permitted or included in any dependency.

---

## 2. Core Upstream Components & Attributions

### 2.1 Tantivy
- **Role**: Full-text inverted index engine (Okapi BM25)
- **License**: MIT / Apache-2.0
- **Copyright**: (c) 2016-2024 Paul Masurel and Tantivy Contributors

### 2.2 ONNX Runtime (`ort`) & Tokenizers
- **Role**: Local transformer embedding inference runtime + HuggingFace BPE tokenizer
- **License**: Apache-2.0 / MIT
- **Copyright**: (c) ONNX Runtime Contributors; (c) HuggingFace `tokenizers` Contributors

### 2.3 Petgraph
- **Role**: Graph data structure and BFS graph traversal
- **License**: MIT / Apache-2.0
- **Copyright**: (c) 2014-2024 Petgraph Contributors

### 2.3.1 Postcard
- **Role**: Compact `serde` binary serialization for on-disk graph persistence (`graph.bin`)
- **License**: MIT / Apache-2.0
- **Copyright**: (c) 2019-2024 James Munns and Postcard Contributors

### 2.4 HNSW-rs
- **Role**: Hierarchical Navigable Small World vector indexing
- **License**: MIT / Apache-2.0
- **Copyright**: (c) 2018-2024 HNSW-rs Contributors

### 2.5 Rusqlite & SQLite3
- **Role**: Metadata, content hash, and schema persistence (bundled C build)
- **License**: MIT (Rusqlite) / Public Domain (SQLite)
- **Copyright**: (c) 2014-2024 The Rusqlite Developers

### 2.6 Tokio & Axum
- **Role**: Async I/O runtime and HTTP MCP transport
- **License**: MIT
- **Copyright**: (c) 2019-2024 Tokio Contributors

---

## 3. Redistributed Model Weights (Bundled in Release Artifacts)

Release archives bundle a pre-trained embedding model as a sidecar next to the
binary (`<binary-dir>/models/jina-embeddings-v2-base-code/`). These weights are a
third-party work redistributed unmodified under their upstream license.

### 3.1 jina-embeddings-v2-base-code
- **Role**: Local ONNX embedding model for semantic/vector retrieval (768-dim).
- **Upstream**: [`jinaai/jina-embeddings-v2-base-code`](https://huggingface.co/jinaai/jina-embeddings-v2-base-code)
  (Hugging Face), pinned revision `516f4baf13dec4ddddda8631e019b5737c8bc250`.
- **License**: **Apache License 2.0** (declared in the model card metadata).
- **Copyright**: (c) Jina AI GmbH.
- **Bundled files** (mirrored verbatim from the upstream repo, INT8 dynamic quantization):
  - `onnx/model_quantized.onnx` — SHA256 `ed45870251c9f0cf656e78aab0d37a23489066df8a222bb1c8caf8a45f2cb16d`
  - `tokenizer.json` — SHA256 `b01c78a902aa4facb2f47f95449f48e2f7bbfea5d2472ee2f6ce92323c6f86e5`
- **Integrity**: pinned SHA256 verified at fetch time (`scripts/fetch-model.sh` /
  `.ps1`); the ONNX hash matches Hugging Face's own LFS content hash at the pinned
  revision. Each release archive also ships a `models/SHA256SUMS.txt` so the sidecar
  can be re-verified independently.
- **Attribution NOTICE**: a `models/NOTICE.md` carrying this attribution + the
  Apache-2.0 grant is generated alongside the weights and shipped in the archive.

The Apache-2.0 license text applicable to these weights is the same as that already
included for Apache-2.0 dependencies above; see <https://www.apache.org/licenses/LICENSE-2.0>.

---

## 4. Automated Continuous Verification

This repository automatically audits every commit and pull request using:
1. **`cargo-deny check`**: Prevents unauthorized licenses, wildcards, unmaintained crates, and non-crates.io sources.
2. **`cargo-audit`**: Scans the dependency tree against the official [RustSec Advisory Database](https://rustsec.org/) for CVEs.
3. **`unsafe_code = "forbid"`**: Guaranteed 100% safe Rust across all workspace crates.
