//! Tantivy-backed full-text BM25 index adapter.

pub mod lockfile;
pub mod retry_directory;
pub mod schema;
pub mod store;

#[cfg(test)]
mod tests;

pub use lockfile::heal_stale_lockfiles;
pub use retry_directory::RetryDirectory;
pub use schema::build_schema;
pub use store::BM25Index;
