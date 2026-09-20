//! Multi-corpus support: manages multiple independent Engine instances.
//!
//! Each corpus is an independent unit with its own BM25 index, vector index,
//! knowledge graph, and SQLite store. The [`CorpusManager`] provides a unified
//! interface for routing operations to the correct engine.

pub(crate) mod federation;
pub(crate) mod manager;
pub(crate) mod routing;
pub(crate) mod types;

#[cfg(test)]
mod tests;

pub use manager::CorpusManager;
pub use types::{CorpusHop, CorpusInfo, FederatedNode, FederatedTraversal, ResolverKind};
