//! Algorithm Substrate: modular, decomposed retrieval components.

pub mod binary;
pub mod binaryv2;
pub mod binaryv3;
pub mod bm25;
pub mod composite;
pub mod dense;
pub mod eval;
pub mod graph;
pub mod registry;

pub use binary::{BinaryAlgorithm, BinaryConfig};
pub use binaryv2::{BinaryV2Algorithm, BinaryV2Config};
pub use binaryv3::{BinaryV3Algorithm, BinaryV3Config};
pub use bm25::{Bm25Algorithm, Bm25Config};
pub use composite::{search_fast_composite, search_hybrid_composite, CompositeConfig};
pub use dense::{DenseAlgorithm, DenseConfig};
pub use eval::{AlgoConfig, AlgoHit, AlgorithmicIndex, IndexStats};
pub use graph::{GraphAlgorithm, GraphConfig};
pub use groundcontrol_common::types::BinaryProjectionKind;
pub use registry::AlgorithmRegistry;
