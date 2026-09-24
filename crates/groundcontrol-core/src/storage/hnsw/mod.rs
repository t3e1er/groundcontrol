//! HNSW vector store adapter wrapping hnsw_rs.

pub mod store;

#[cfg(test)]
mod tests;

pub use store::{VectorIndex, DEFAULT_DIMENSIONS};
