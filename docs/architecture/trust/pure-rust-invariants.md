---
title: "Pure Safe Rust Invariants"
description: "Why groundcontrol forbids unsafe code, eliminates C runtime dependencies, and pins MSRV 1.80."
category: "trust"
status: "active"
tags: ["rust", "safety", "forbid-unsafe", "msrv", "performance", "sub-millisecond"]
related:
  - "[[docs/architecture/trust/index]]"
  - "[[docs/architecture/trust/files-are-ground-truth]]"
  - "[[docs/architecture/adr/adr-009-greenfield-no-backwards-compat]]"
---

# Pure Safe Rust Invariants

`groundcontrol` is engineered for zero-crash reliability, deterministic latency, and maximum systems safety.

* **Root Safety Guard**: [`crates/groundcontrol-core/src/lib.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/lib.rs) (`#![forbid(unsafe_code)]`)
* **Toolchain Pin**: [`rust-toolchain.toml`](file:///c:/dev/ctx/groundcontrol/rust-toolchain.toml) (MSRV 1.80)

---

## 1. `#![forbid(unsafe_code)]`

Across all workspace crates (`groundcontrol-common`, `groundcontrol-core`, `groundcontrol-mcp`, `groundcontrol-cli`), unsafe code is forbidden at the compiler level:

```rust
#![forbid(unsafe_code)]
```

* **Zero Undefined Behavior**: No pointer arithmetic, no uninitialized memory, no manual memory management bugs.
* **Thread Safety**: Complete protection against data races and concurrency leaks across reader-writer worker pools.
* **Auditability**: Pull requests attempting to introduce `unsafe` blocks are rejected automatically during `cargo check`.

---

## 2. Zero External C-Runtime Dependencies

Many systems tools introduce hidden maintenance burdens by linking against dynamic C/C++ libraries (such as OpenSSL or system SQLite binaries).

`groundcontrol` eliminates this completely:
* **TLS**: Pure Rust TLS via `rustls` (no OpenSSL or LibreSSL dynamic linking).
* **SQLite**: Bundled, statically compiled SQLite via `rusqlite` with bundled source.
* **Compression**: Native Rust zstd and tar implementations.
* **Portability**: Standalone binaries run out-of-the-box on clean distributions without `apt-get install libssl-dev` or missing DLL errors.

---

## 3. Sub-Millisecond Retrieval Budgets

When coding agents execute recursive graph traversals or search queries, every 50ms of retrieval latency compounds into multi-second reasoning pauses.

`groundcontrol` operates under strict latency budgets:
* **BM25 Lexical Lookup**: **p50 < 2.2 ms** (powered by Tantivy memory-mapped segment readers in [`crates/groundcontrol-core/src/index/tantivy.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/index/tantivy.rs)).
* **Graph CTE / BFS Traversal**: **p50 < 1.8 ms** (powered by SQLite index-backed CTEs with cycle guards in [`crates/groundcontrol-core/src/catalog/sqlite.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/catalog/sqlite.rs)).
* **3-Way RRF Combination**: **p50 < 0.4 ms** (SIMD-accelerated array sorting in [`crates/groundcontrol-core/src/search/rrf.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/search/rrf.rs)).

To coding agents, retrieval feels instantaneous.
