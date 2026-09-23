//! Reciprocal Rank Fusion (RRF) for composite multi-algorithm retrieval.

use groundcontrol_common::types::SearchResult;
use std::collections::HashMap;

/// Reciprocal Rank Fusion combining multiple ranked result sets.
///
/// Score = sum over all lists of: `1.0 / (k + rank + 1.0)`.
pub fn rrf_merge(result_lists: &[&[SearchResult]], limit: usize, k: f64) -> Vec<SearchResult> {
    let mut scores: HashMap<String, (f64, SearchResult)> = HashMap::new();

    for list in result_lists {
        for (rank, item) in list.iter().enumerate() {
            let rrf_contribution = 1.0 / (k + rank as f64 + 1.0);
            let entry = scores.entry(item.path.clone()).or_insert_with(|| (0.0, item.clone()));
            entry.0 += rrf_contribution;
        }
    }

    let mut fused: Vec<SearchResult> = scores
        .into_iter()
        .map(|(_, (rrf_score, mut item))| {
            item.score = rrf_score;
            item
        })
        .collect();

    fused.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    fused.truncate(limit);
    fused
}
