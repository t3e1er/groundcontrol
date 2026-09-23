//! Types and configuration options for structural graph and PPR retrieval.

use serde::{Deserialize, Serialize};

/// Configuration options for the graph / PPR retrieval algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphConfig {
    /// Personalized PageRank damping factor alpha (default 0.85).
    pub alpha: f64,
    /// Number of power iterations for diffusion (default 10).
    pub iterations: usize,
    /// Max traversal depth for BFS.
    pub max_depth: usize,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self { alpha: 0.85, iterations: 10, max_depth: 2 }
    }
}
