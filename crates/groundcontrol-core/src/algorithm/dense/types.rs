//! Types and configuration options for dense ONNX neural embeddings retrieval.

use serde::{Deserialize, Serialize};

/// Configuration options for the dense neural retrieval algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenseConfig {
    /// Embedding vector dimensions (default 768 for jina-embeddings-v2-base-code).
    pub dimensions: usize,
    /// Batch size for forward tensor inference passes.
    pub batch_size: usize,
}

impl Default for DenseConfig {
    fn default() -> Self {
        Self { dimensions: 768, batch_size: 64 }
    }
}
