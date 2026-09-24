//! Reciprocal Rank Fusion (RRF) for composite multi-algorithm retrieval.
//!
//! Unifies multi-list ranked fusion by re-exporting the canonical implementation from `search::fusion`.

pub use crate::search::fusion::rrf_fuse_with_k as rrf_merge;
