//! Symmetrical storage adapters implementing groundcontrol hexagonal ports:
//! - `sqlite`: [`MetadataCatalog`] adapter (`Store`)
//! - `tantivy`: [`TextIndex`] adapter (`BM25Index`)
//! - `hnsw`: [`VectorStore`] adapter (`VectorIndex`)
//! - `binary`: [`AlgorithmicSearchIndex`] adapter (`BinarySearchIndex`)

pub mod hnsw;
pub mod sqlite;
pub mod tantivy;

pub use hnsw::VectorIndex;
pub use sqlite::Store;
pub use tantivy::BM25Index;
