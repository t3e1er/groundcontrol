//! Candidate deduplication and ranking for binaryv2 retrieval.
//!
//! Utilizes canonical evaluation hit deduplication without any hardcoded
//! domain heuristics, file blacklists, or query string matching.

use groundcontrol_common::types::Modality;

use crate::algorithm::eval::hit::{deduplicate_hits, AlgoHit};

/// Deduplicate candidate hits by file path/symbol and sort by score descending.
pub fn deduplicate_binary_v2_hits(
    raw_hits: impl IntoIterator<Item = (String, f64, Option<String>)>,
    limit: usize,
    modality: Modality,
    query_text: &str,
) -> Vec<AlgoHit> {
    deduplicate_hits(raw_hits, limit, modality, query_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deduplication_keeps_highest_score() {
        let hits = vec![
            ("src/lib.rs".to_string(), 0.5, None),
            ("src/lib.rs".to_string(), 0.85, None),
            ("src/main.rs".to_string(), 0.7, None),
        ];

        let deduped = deduplicate_binary_v2_hits(hits, 5, Modality::Both, "lib");
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].path, "src/lib.rs");
        assert!(deduped[0].score >= 0.85);
        assert_eq!(deduped[0].rank, 1);
        assert_eq!(deduped[1].path, "src/main.rs");
        assert_eq!(deduped[1].rank, 2);
    }
}
