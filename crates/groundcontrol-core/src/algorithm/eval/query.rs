//! Isolated query execution across retrieval algorithms for evaluation.

use groundcontrol_common::ports::{SearchQuery, SearchService};
use groundcontrol_common::types::Modality;
use groundcontrol_common::{Error, Result};

use super::config::AlgoConfig;
use super::hit::{deduplicate_hits, AlgoHit};
use super::sanitizer::sanitize_lucene_query;
use crate::engine::Engine;

/// Execute isolated binary Hamming search.
pub fn execute_binary_query(
    engine: &Engine,
    config: &AlgoConfig,
    query: &str,
    k: usize,
    modality: Modality,
) -> Result<Vec<AlgoHit>> {
    let sanitized = sanitize_lucene_query(query);
    let search_text = if sanitized.is_empty() { query } else { &sanitized };

    let binary = engine.binary_index();
    let q_fp = binary.project_query_with_kind(search_text, config.binary_projection)?;
    let pool_size = (k * config.binary_pool_multiplier).max(500);
    let hits = binary.search_hamming(&q_fp, pool_size, modality)?;

    let raw = hits.into_iter().map(|(id, dist)| {
        let sim = 1.0 - (dist as f32 / 256.0);
        (id, sim as f64, None)
    });

    Ok(deduplicate_hits(raw, k, modality, search_text))
}

/// Execute isolated BM25 lexical search.
pub fn execute_bm25_query(
    engine: &Engine,
    query: &str,
    k: usize,
    modality: Modality,
) -> Result<Vec<AlgoHit>> {
    let sanitized = sanitize_lucene_query(query);
    let search_text = if sanitized.is_empty() { query } else { &sanitized };

    let sq = SearchQuery {
        query: search_text.to_string(),
        mode: Some("bm25".to_string()),
        limit: Some(k * 5),
        modality,
        ..Default::default()
    };
    let raw_hits = engine.search_service().search(&sq)?;
    let raw = raw_hits.into_iter().map(|r| (r.path, r.score, r.symbol));

    Ok(deduplicate_hits(raw, k, modality, search_text))
}

/// Execute isolated PPR diffusion with BM25 seeding.
pub fn execute_ppr_query(
    engine: &Engine,
    config: &AlgoConfig,
    query: &str,
    k: usize,
    modality: Modality,
) -> Result<Vec<AlgoHit>> {
    let sanitized = sanitize_lucene_query(query);
    let search_text = if sanitized.is_empty() { query } else { &sanitized };

    let sq = SearchQuery {
        query: search_text.to_string(),
        mode: Some("bm25".to_string()),
        limit: Some(k * config.ppr_bm25_multiplier),
        modality,
        ..Default::default()
    };
    let bm25_hits = engine.search_service().search(&sq)?;
    let seeds: Vec<(String, f64)> = bm25_hits.into_iter().map(|r| (r.path, r.score)).collect();

    let ppr_scores = crate::graph::diffusion::personalized_pagerank(
        engine.knowledge_graph(),
        &seeds,
        config.ppr_alpha,
        config.ppr_iterations,
        None,
    );

    let raw = ppr_scores.into_iter().map(|p| (p.path, p.score, None));
    Ok(deduplicate_hits(raw, k, modality, search_text))
}

/// Execute fast algorithmic hybrid search (BM25 + Binary + PPR via 3-way RRF, zero ONNX inference).
pub fn execute_fast_query(
    engine: &Engine,
    query: &str,
    k: usize,
    modality: Modality,
) -> Result<Vec<AlgoHit>> {
    let sanitized = sanitize_lucene_query(query);
    let search_text = if sanitized.is_empty() { query } else { &sanitized };

    let sq = SearchQuery {
        query: search_text.to_string(),
        mode: Some("fast".to_string()),
        limit: Some(k * 3),
        modality,
        ..Default::default()
    };
    let raw_hits = engine.search_service().search(&sq)?;
    let raw = raw_hits.into_iter().map(|r| (r.path, r.score, r.symbol));

    Ok(deduplicate_hits(raw, k, modality, search_text))
}

/// Execute pure dense neural embedding search.
pub fn execute_semantic_query(
    engine: &Engine,
    query: &str,
    k: usize,
    modality: Modality,
) -> Result<Vec<AlgoHit>> {
    if !engine.ensure_embedder()? {
        return Err(Error::Index(
            "Dense ONNX embedder could not be initialized or corpus is in Fast mode".into(),
        ));
    }

    let sanitized = sanitize_lucene_query(query);
    let search_text = if sanitized.is_empty() { query } else { &sanitized };

    let sq = SearchQuery {
        query: search_text.to_string(),
        mode: Some("semantic".to_string()),
        limit: Some(k * 3),
        modality,
        ..Default::default()
    };
    let raw_hits = engine.search_service().search(&sq)?;
    let raw = raw_hits.into_iter().map(|r| (r.path, r.score, r.symbol));

    Ok(deduplicate_hits(raw, k, modality, search_text))
}

/// Execute full 3-signal hybrid search.
pub fn execute_hybrid_query(
    engine: &Engine,
    query: &str,
    k: usize,
    modality: Modality,
) -> Result<Vec<AlgoHit>> {
    let sanitized = sanitize_lucene_query(query);
    let search_text = if sanitized.is_empty() { query } else { &sanitized };

    let sq = SearchQuery {
        query: search_text.to_string(),
        mode: Some("hybrid".to_string()),
        limit: Some(k * 3),
        modality,
        ..Default::default()
    };
    let raw_hits = engine.search_service().search(&sq)?;
    let raw = raw_hits.into_iter().map(|r| (r.path, r.score, r.symbol));

    Ok(deduplicate_hits(raw, k, modality, search_text))
}
