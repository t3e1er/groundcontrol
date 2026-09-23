//! Isolated binary fingerprint projection + Hamming scan retrieval.

use groundcontrol_common::types::Modality;
use groundcontrol_core::engine::Engine;

use crate::config::AlgoConfig;
use crate::query::{deduplicate_hits, sanitize_lucene_query, AlgoHit};
use crate::Result;

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
