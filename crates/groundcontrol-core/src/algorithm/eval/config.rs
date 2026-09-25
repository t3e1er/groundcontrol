//! Algorithm configuration types for retrieval ablation and evaluation.

use groundcontrol_common::types::BinaryProjectionKind;
use serde::{Deserialize, Serialize};

/// Runtime configuration for algorithm selection and tuning during evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgoConfig {
    /// Binary fingerprint projection variant (selects between flat SIF and 4-channel
    /// partitioned hyperplane projections at index + query time).
    pub binary_projection: BinaryProjectionKind,
    /// Personalized PageRank damping factor α. Default: 0.85.
    pub ppr_alpha: f64,
    /// PPR iteration count. Higher = more accurate diffusion but slower. Default: 10.
    pub ppr_iterations: usize,
    /// BM25 candidate multiplier for PPR seeding (`limit * ppr_bm25_multiplier`). Default: 5.
    pub ppr_bm25_multiplier: usize,
    /// Binary candidate pool size (`limit * binary_pool_multiplier` or 500, whichever is larger).
    /// Larger pools improve recall at cost of linear scan time. Default: 50.
    pub binary_pool_multiplier: usize,
    /// Apply Stage 2 Bayesian structural prior in binaryv3. Default: true.
    pub binaryv3_prior: bool,
    /// Enable Word 0 Matryoshka early-exit filter in binaryv3. Default: false.
    pub binaryv3_early_exit: bool,
}

impl Default for AlgoConfig {
    fn default() -> Self {
        Self {
            binary_projection: BinaryProjectionKind::PartitionedHyperplane,
            ppr_alpha: 0.85,
            ppr_iterations: 10,
            ppr_bm25_multiplier: 5,
            binary_pool_multiplier: 50,
            binaryv3_prior: true,
            binaryv3_early_exit: false,
        }
    }
}
