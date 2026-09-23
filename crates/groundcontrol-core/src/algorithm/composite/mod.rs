//! Composite multi-algorithm retrieval orchestrators.

use groundcontrol_common::ports::RetrievalAlgorithm;
use groundcontrol_common::types::{Modality, SearchResult};
use groundcontrol_common::Result;

pub mod rrf;
#[cfg(test)]
pub mod tests;
pub mod types;

pub use rrf::rrf_merge;
pub use types::CompositeConfig;

/// Search fast mode: 3-way RRF across BM25, Binary, and Graph/PPR.
pub fn search_fast_composite(
    bm25: &dyn RetrievalAlgorithm,
    binary: &dyn RetrievalAlgorithm,
    graph: &dyn RetrievalAlgorithm,
    query: &str,
    limit: usize,
    modality: Modality,
    config: &CompositeConfig,
) -> Result<Vec<SearchResult>> {
    let bm25_hits = bm25.search(query, limit * 3, modality).unwrap_or_default();
    let binary_hits = binary.search(query, limit * 3, modality).unwrap_or_default();
    let graph_hits = graph.search(query, limit * 3, modality).unwrap_or_default();

    Ok(rrf_merge(&[&bm25_hits, &binary_hits, &graph_hits], limit, config.rrf_k))
}

/// Search hybrid mode: 3-way RRF across BM25, Dense, and Graph.
pub fn search_hybrid_composite(
    bm25: &dyn RetrievalAlgorithm,
    dense: &dyn RetrievalAlgorithm,
    graph: &dyn RetrievalAlgorithm,
    query: &str,
    limit: usize,
    modality: Modality,
    config: &CompositeConfig,
) -> Result<Vec<SearchResult>> {
    let bm25_hits = bm25.search(query, limit * 3, modality).unwrap_or_default();
    let dense_hits = dense.search(query, limit * 3, modality).unwrap_or_default();
    let graph_hits = graph.search(query, limit * 3, modality).unwrap_or_default();

    Ok(rrf_merge(&[&bm25_hits, &dense_hits, &graph_hits], limit, config.rrf_k))
}
