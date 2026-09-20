//! Full-text index port.

use crate::types::{Chunk, Modality, SearchResult};
use crate::Result;

/// Full-text index port: the BM25 lexical-retrieval contract for a corpus.
///
/// This is the domain-facing contract for the Tantivy-backed full-text index.
/// It covers document ingestion (add/remove), commit/writer-lifecycle, and
/// ranked lexical search — optionally restricted to a [`Modality`]. Every
/// signature speaks only [`crate::types`] domain types (`Chunk`,
/// `SearchResult`, `Modality`) and standard-library types — no backend type
/// (`tantivy::*`, schemas, writers, readers) ever crosses this boundary, so
/// consumers depend on the contract rather than on Tantivy.
///
/// Construction (opening or creating the underlying index) and lockfile
/// healing are deliberately **not** part of this port: they are
/// adapter/composition-root concerns. The port describes only the runtime
/// behaviour a full-text index must provide.
pub trait TextIndex {
    /// Release the underlying writer, dropping any exclusive index lock.
    ///
    /// Call after a commit to allow other processes to access the index.
    fn release_writer(&mut self);

    /// Add all chunks for a document to the index. Does NOT auto-commit.
    fn add_document(
        &mut self,
        doc_path: &str,
        title: Option<&str>,
        tags: &[String],
        chunks: &[Chunk],
    ) -> Result<()>;

    /// Remove all indexed chunks for a given document path. Does NOT auto-commit.
    fn remove_document(&mut self, doc_path: &str) -> Result<()>;

    /// Commit pending changes to disk.
    fn commit(&mut self) -> Result<()>;

    /// Search the index with a text query (no modality restriction).
    ///
    /// Thin wrapper over [`TextIndex::search_with_modality`] with
    /// [`Modality::Both`]. Returns ranked results with scores and snippets.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>>;

    /// Search the index, restricting results to the requested [`Modality`].
    fn search_with_modality(
        &self,
        query: &str,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<SearchResult>>;
}
