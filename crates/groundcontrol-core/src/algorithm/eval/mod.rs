//! In-process algorithm retrieval evaluation and ablation substrate.
//!
//! Exposes each retrieval algorithm in `groundcontrol-core` as directly callable,
//! in-process functions with zero serialization or IPC overhead.

pub mod config;
pub mod hit;
pub mod index;
pub mod query;
pub mod sanitizer;
#[cfg(test)]
mod tests;

pub use config::AlgoConfig;
pub use hit::{clean_path, deduplicate_hits, is_file_path, is_non_code_path, AlgoHit};
pub use index::{AlgorithmicIndex, IndexStats};
pub use query::{
    execute_binary_query, execute_bm25_query, execute_fast_query, execute_hybrid_query,
    execute_ppr_query, execute_semantic_query,
};
pub use sanitizer::sanitize_lucene_query;
