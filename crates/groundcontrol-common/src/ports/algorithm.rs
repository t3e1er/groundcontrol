//! Retrieval algorithm port for modular search backends.

use std::path::Path;

use crate::types::{Modality, ParsedArtifact, SearchResult};
use crate::Result;

/// Uniform port satisfied by individual search and retrieval algorithm components.
///
/// Each retrieval algorithm (`bm25`, `binary`, `ppr`, `dense`, etc.) manages its
/// own indexing lifecycle, internal storage/memory structures, and isolated query execution.
pub trait RetrievalAlgorithm: Send + Sync {
    /// Canonical algorithm identifier (e.g. `"bm25"`, `"binary"`, `"ppr"`, `"dense"`).
    fn name(&self) -> &'static str;

    /// Open or initialize storage in the corpus `.index/` directory.
    fn open(&mut self, index_dir: &Path) -> Result<()>;

    /// Index a parsed document artifact into this algorithm's index.
    fn index_document(&mut self, doc: &ParsedArtifact) -> Result<()>;

    /// Batch index document artifacts (defaults to sequentially calling `index_document`).
    fn index_batch(&mut self, docs: &[ParsedArtifact]) -> Result<()> {
        for doc in docs {
            self.index_document(doc)?;
        }
        Ok(())
    }

    /// Remove a document from the index on file deletion or delta change.
    fn remove_document(&mut self, path: &str) -> Result<()>;

    /// Commit in-memory buffers and persist state to disk.
    fn commit(&mut self) -> Result<()>;

    /// Clear and purge all index state for full reindex.
    fn clear(&mut self) -> Result<()>;

    /// Execute an isolated retrieval query.
    fn search(&self, query: &str, limit: usize, modality: Modality) -> Result<Vec<SearchResult>>;
}
