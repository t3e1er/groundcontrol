//! Semantic (dense vector) search strategies.

use ctxvault_common::ports::{EmbeddingProvider, VectorStore};
use ctxvault_common::types::{Modality, ScoreBreakdown, SearchDepth, SearchResult};
use ctxvault_common::Result;

use super::fusion::rrf_fuse;

/// Semantic vector search using embedding similarity.
///
/// Embeds the query text, then searches the vector index for nearest neighbors.
/// Returns results ranked by cosine similarity.
///
/// - `vector_index`: The HNSW vector index to search.
/// - `embedder`: The embedding model to encode the query.
/// - `query`: The natural language query to embed and search.
/// - `limit`: Maximum results to return.
/// - `doc_level_only`: If true, only search document-level embeddings (broad mode).
pub fn search_semantic(
    vector_index: &impl VectorStore,
    embedder: &impl EmbeddingProvider,
    query: &str,
    limit: usize,
    doc_level_only: bool,
    modality: Modality,
) -> Result<Vec<SearchResult>> {
    // 1. Embed the query.
    let query_embedding = embedder.embed_query(query)?;

    // 2. Search vector index.
    let vector_results = vector_index.search(&query_embedding, limit, doc_level_only, modality)?;

    // 3. Convert to SearchResult.
    let results: Vec<SearchResult> = vector_results
        .into_iter()
        .map(|vr| {
            SearchResult::new(vr.doc_path, vr.score)
                .with_chunk_index(vr.chunk_index)
                .with_score_components(ScoreBreakdown {
                    bm25: 0.0,
                    vector: vr.score,
                    graph_boost: 0.0,
                    graph_hops: None,
                })
        })
        .collect();

    Ok(results)
}

/// Semantic vector search using a pre-computed query embedding.
///
/// Use this when you already have the query embedding (avoids redundant embedding).
pub fn search_semantic_with_embedding(
    vector_index: &impl VectorStore,
    query_embedding: &[f32],
    limit: usize,
    doc_level_only: bool,
    modality: Modality,
) -> Result<Vec<SearchResult>> {
    let vector_results = vector_index.search(query_embedding, limit, doc_level_only, modality)?;

    let results: Vec<SearchResult> = vector_results
        .into_iter()
        .map(|vr| {
            SearchResult::new(vr.doc_path, vr.score)
                .with_chunk_index(vr.chunk_index)
                .with_score_components(ScoreBreakdown {
                    bm25: 0.0,
                    vector: vr.score,
                    graph_boost: 0.0,
                    graph_hops: None,
                })
        })
        .collect();

    Ok(results)
}

/// Dual-level semantic search with depth parameter.
///
/// Implements the LightRAG-inspired dual-level retrieval:
/// - **Precise**: chunk-level vectors only (specific passages)
/// - **Broad**: document-level vectors only (thematically connected docs)
/// - **Adaptive**: both levels merged with Reciprocal Rank Fusion (default)
///
/// The `depth` parameter controls which level(s) to search.
pub fn search_semantic_dual(
    vector_index: &impl VectorStore,
    embedder: &impl EmbeddingProvider,
    query: &str,
    limit: usize,
    depth: SearchDepth,
    modality: Modality,
) -> Result<Vec<SearchResult>> {
    // Embed the query.
    let query_embedding = embedder.embed_query(query)?;

    match depth {
        SearchDepth::Precise => {
            // Chunk-level only.
            search_semantic_with_embedding(vector_index, &query_embedding, limit, false, modality)
        }
        SearchDepth::Broad => {
            // Document-level only.
            search_semantic_with_embedding(vector_index, &query_embedding, limit, true, modality)
        }
        SearchDepth::Adaptive => {
            // Both levels, merged with RRF.
            let chunk_results = search_semantic_with_embedding(
                vector_index,
                &query_embedding,
                limit * 2,
                false,
                modality,
            )?;
            let doc_results = search_semantic_with_embedding(
                vector_index,
                &query_embedding,
                limit * 2,
                true,
                modality,
            )?;

            // RRF fusion of both result sets.
            let fused = rrf_fuse(&[&chunk_results, &doc_results], limit);
            Ok(fused)
        }
    }
}
