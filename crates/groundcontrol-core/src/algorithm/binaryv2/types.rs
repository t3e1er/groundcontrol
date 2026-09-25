//! Types and configuration options for binaryv2 retrieval.

use serde::{Deserialize, Serialize};

/// Configuration options for the binaryv2 retrieval algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryV2Config {
    /// Multiplier on `limit` for candidate pool size in Hamming search.
    pub pool_multiplier: usize,
    /// Lexical and path match boost weight.
    pub path_bonus: f64,
}

impl Default for BinaryV2Config {
    fn default() -> Self {
        Self { pool_multiplier: 50, path_bonus: 0.05 }
    }
}
