//! Binary projection substrate for mapping text queries and AST grammar semantics
//! into 256-bit Hamming fingerprints.

pub mod hyperplanes;
pub mod sif;

use std::sync::Arc;

use groundcontrol_common::types::{
    BinaryFingerprint, BinaryProjectionKind, ExtractedGrammarSemantics,
};

pub use hyperplanes::PartitionedHyperplaneProjector;
pub use sif::SifEngine;

/// Unified trait for projecting text queries and AST grammar semantics into 256-bit binary fingerprints.
pub trait BinaryProjector: std::fmt::Debug + Send + Sync {
    /// Project a text query into a 256-bit binary fingerprint.
    fn project_query(&self, text: &str) -> BinaryFingerprint;

    /// Project extracted AST grammar semantics into a 256-bit binary fingerprint.
    fn project_semantics(&self, sem: &ExtractedGrammarSemantics) -> BinaryFingerprint;
}

/// Factory function to construct a binary projector matching the specified projection strategy.
pub fn create_projector(kind: BinaryProjectionKind) -> Arc<dyn BinaryProjector> {
    match kind {
        BinaryProjectionKind::FlatSif => Arc::new(SifEngine::default()),
        BinaryProjectionKind::PartitionedHyperplane => {
            Arc::new(PartitionedHyperplaneProjector::default())
        }
    }
}
