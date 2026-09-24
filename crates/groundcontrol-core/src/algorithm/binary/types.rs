//! Types and configuration options for 256-bit binary fingerprint retrieval.

pub use groundcontrol_common::types::{BinaryFingerprint, BinaryProjectionKind, FingerprintRecord};
use serde::{Deserialize, Serialize};

/// Configuration options for the binary retrieval algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryConfig {
    /// Projection strategy (FlatSif vs PartitionedHyperplane).
    pub projection_kind: BinaryProjectionKind,
    /// Multiplier on `limit` for candidate pool size in Hamming search.
    pub pool_multiplier: usize,
}

impl Default for BinaryConfig {
    fn default() -> Self {
        Self { projection_kind: BinaryProjectionKind::default(), pool_multiplier: 5 }
    }
}
