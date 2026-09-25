//! Symmetrical storage adapters implementing groundcontrol hexagonal ports:
//! - `sqlite`: [`MetadataCatalog`](groundcontrol_common::ports::MetadataCatalog) adapter (`Store`)
//! - `tantivy`: [`TextIndex`](groundcontrol_common::ports::TextIndex) adapter (`BM25Index`)
//! - `hnsw`: [`VectorStore`](groundcontrol_common::ports::VectorStore) adapter (`VectorIndex`)
//! - `binary`: [`AlgorithmicSearchIndex`](groundcontrol_common::ports::AlgorithmicSearchIndex) adapter (`BinarySearchIndex`)

pub mod hnsw;
pub mod sqlite;
pub mod tantivy;

pub use hnsw::VectorIndex;
pub use sqlite::Store;
pub use tantivy::BM25Index;
