//! Search strategies: BM25, semantic (vector), hybrid, graph, related, multihop, fast (SIF + Binary + PPR).

pub mod binary;
pub mod bm25;
pub mod explain;
pub mod fast;
pub mod fusion;
pub mod graph;
pub mod hybrid;
pub mod multihop;
pub mod projection;
pub mod semantic;

#[cfg(test)]
mod tests;

pub use binary::BinarySearchIndex;
pub use bm25::search_bm25;
pub use explain::search_explain;
pub use fast::{search_explain_fast, search_fast};
pub use fusion::{
    enrich_results_with_lineage, path_matches_modality, rrf_fuse, rrf_fuse_cross_corpus,
};
pub use graph::{search_graph, search_related};
pub use hybrid::{search_hybrid, search_hybrid_full};
pub use multihop::{decompose_query, search_multihop};
pub use projection::{BinaryProjector, PartitionedHyperplaneProjector, SifEngine};
pub use semantic::{search_semantic, search_semantic_dual, search_semantic_with_embedding};
