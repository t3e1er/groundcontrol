//! `groundcontrol-algo` — Standalone per-algorithm retrieval library.
//!
//! Exposes each retrieval algorithm in `groundcontrol-core` as an independently
//! callable library function. Designed to be consumed by external benchmarking
//! harnesses (e.g. `groundtruth`) via a Cargo path dependency, enabling
//! zero-IPC, zero-serialization algorithm ablation.
//!
//! # Two-Tier Model
//!
//! ```text
//! groundtruth
//! ├── AlgoBackend  ── Cargo path dep ──► groundcontrol-algo (this crate, lib)
//! │   (direct Rust calls, ~0µs overhead)
//! └── McpBackend  ── stdio JSON-RPC ──► groundcontrol (MCP binary)
//!     (system-level, ~80ms/query)
//! ```
//!
//! # Usage (from groundtruth or any Rust harness)
//!
//! ```rust,no_run
//! use groundcontrol_algo::{AlgoConfig, AlgorithmicIndex, BinaryProjectionKind, Modality};
//! use std::path::Path;
//!
//! let config = AlgoConfig {
//!     binary_projection: BinaryProjectionKind::PartitionedHyperplane,
//!     ..Default::default()
//! };
//! let (index, stats) = AlgorithmicIndex::build(Path::new("/path/to/repo"), config)?;
//! let hits = index.query_binary("convert date string", 10, Modality::Code)?;
//! # Ok::<(), groundcontrol_algo::Error>(())
//! ```

pub mod config;
pub mod index;
pub mod query;

pub use config::{AlgoConfig, BinaryProjectionKind};
pub use index::{AlgorithmicIndex, IndexStats};
pub use query::AlgoHit;
pub use groundcontrol_common::types::Modality;


use thiserror::Error;

/// Errors returned by the `groundcontrol-algo` library.
#[derive(Debug, Error)]
pub enum Error {
    /// Underlying groundcontrol-core engine error.
    #[error("engine error: {0}")]
    Engine(#[from] groundcontrol_common::Error),
    /// Corpus path does not exist or is not accessible.
    #[error("corpus path error: {0}")]
    CorpusPath(String),
    /// Algorithm requires a component that is not available (e.g. ONNX embedder).
    #[error("algorithm unavailable: {0}")]
    AlgorithmUnavailable(String),
    /// I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for all `groundcontrol-algo` operations.
pub type Result<T> = std::result::Result<T, Error>;
