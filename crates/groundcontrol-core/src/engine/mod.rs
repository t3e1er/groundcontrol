//! Engine: coordinates persistence (SQLite), BM25 index (Tantivy), knowledge graph (petgraph),
//! and vector index (HNSW) with optional embedding support.
//!
//! The [`Engine`] is the top-level orchestrator for a single corpus. It manages
//! indexing, delta scanning, and provides unified access to all subsystems.

pub(crate) mod analytics;
pub(crate) mod indexer;
pub(crate) mod state;
pub(crate) mod types;

#[cfg(test)]
mod tests;

pub use state::Engine;
pub use types::{
    DeltaScanResult, IndexingProgress, IndexingStage, IndexingStatusResponse, PendingChunk,
    ProgressCallback,
};
