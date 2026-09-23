//! Types and configuration options for BM25 lexical retrieval.

use serde::{Deserialize, Serialize};

/// Configuration options for the BM25 retrieval algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25Config {
    /// Maximum candidates to fetch before scoring.
    pub candidate_multiplier: usize,
}

impl Default for Bm25Config {
    fn default() -> Self {
        Self { candidate_multiplier: 3 }
    }
}
