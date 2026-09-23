//! Isolated HippoRAG 2-hop personalized PageRank diffusion on Petgraph.

use groundcontrol_common::ports::{SearchQuery, SearchService};
use groundcontrol_common::types::Modality;
use groundcontrol_core::engine::Engine;

use crate::config::AlgoConfig;
use crate::query::{deduplicate_hits, sanitize_lucene_query, AlgoHit};
use crate::Result;

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

    let ppr_scores = groundcontrol_core::graph::diffusion::personalized_pagerank(
        engine.knowledge_graph(),
        &seeds,
        config.ppr_alpha,
        config.ppr_iterations,
        None,
    );

    let raw = ppr_scores.into_iter().map(|p| (p.path, p.score, None));
    Ok(deduplicate_hits(raw, k, modality, search_text))
}
