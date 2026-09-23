//! Pure Tantivy BM25 lexical retrieval.

use groundcontrol_common::ports::{SearchQuery, SearchService};
use groundcontrol_common::types::Modality;
use groundcontrol_core::engine::Engine;

use crate::query::{deduplicate_hits, sanitize_lucene_query, AlgoHit};
use crate::Result;

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
