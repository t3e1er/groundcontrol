//! Semantic gap detection: comparing BM25 and vector search divergence.

use std::collections::HashSet;

use groundcontrol_common::types::Modality;
use groundcontrol_common::Result;
use serde::{Deserialize, Serialize};

use crate::index::BM25Index;
use crate::vector_index::VectorIndex;

/// A semantic gap: a query where BM25 and vector search produce divergent results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticGap {
    /// The test query that revealed the gap.
    pub query: String,
    /// Documents found only by BM25 (not in vector top-K).
    pub bm25_only: Vec<String>,
    /// Documents found only by vector search (not in BM25 top-K).
    pub vector_only: Vec<String>,
    /// Overlap ratio (0.0 = completely disjoint, 1.0 = identical results).
    pub overlap_ratio: f64,
}

/// Find queries where BM25 and vector search disagree.
///
/// Takes a set of test queries and compares BM25 vs vector results for each.
/// Queries with low overlap indicate potential embedding blind spots.
///
/// - `bm25`: BM25 index to search.
/// - `vector_index`: Vector index to search.
/// - `queries`: Test queries to evaluate.
/// - `query_embeddings`: Pre-computed embeddings for each query (same order as queries).
/// - `top_k`: Number of results to compare per query.
pub fn find_semantic_gaps(
    bm25: &BM25Index,
    vector_index: &VectorIndex,
    queries: &[&str],
    query_embeddings: &[Vec<f32>],
    top_k: usize,
) -> Result<Vec<SemanticGap>> {
    let mut gaps = Vec::new();

    for (query, embedding) in queries.iter().zip(query_embeddings.iter()) {
        // Get BM25 results.
        let bm25_results = bm25.search(query, top_k)?;
        let bm25_paths: HashSet<String> = bm25_results.iter().map(|r| r.path.clone()).collect();

        // Get vector results (analytics spans both modalities).
        let vector_results = vector_index.search(embedding, top_k, false, Modality::Both)?;
        let vector_paths: HashSet<String> =
            vector_results.iter().map(|r| r.doc_path.clone()).collect();

        // Compute overlap.
        let overlap: HashSet<&String> = bm25_paths.intersection(&vector_paths).collect();
        let union_size = bm25_paths.len() + vector_paths.len() - overlap.len();
        let overlap_ratio =
            if union_size > 0 { overlap.len() as f64 / union_size as f64 } else { 1.0 };

        let bm25_only: Vec<String> = bm25_paths.difference(&vector_paths).cloned().collect();
        let vector_only: Vec<String> = vector_paths.difference(&bm25_paths).cloned().collect();

        gaps.push(SemanticGap { query: query.to_string(), bm25_only, vector_only, overlap_ratio });
    }

    // Sort by overlap ratio ascending (most divergent first).
    gaps.sort_by(|a, b| {
        a.overlap_ratio.partial_cmp(&b.overlap_ratio).unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(gaps)
}
