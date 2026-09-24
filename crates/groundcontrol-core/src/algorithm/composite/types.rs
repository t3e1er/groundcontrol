//! Types and configuration options for composite multi-algorithm retrieval.

use serde::{Deserialize, Serialize};

/// Configuration options for composite multi-modal retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeConfig {
    /// RRF k-constant (default 60.0).
    pub rrf_k: f64,
}

impl Default for CompositeConfig {
    fn default() -> Self {
        Self { rrf_k: 60.0 }
    }
}
