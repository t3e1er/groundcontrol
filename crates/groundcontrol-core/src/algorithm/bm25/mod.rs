//! BM25 retrieval algorithm component.

use std::path::Path;

use groundcontrol_common::ports::RetrievalAlgorithm;
use groundcontrol_common::types::{Modality, ParsedArtifact, SearchResult};
use groundcontrol_common::Result;

use crate::index::BM25Index;

#[cfg(test)]
pub mod tests;
pub mod types;

pub use types::Bm25Config;

/// BM25 retrieval algorithm backed by a Tantivy full-text index.
pub struct Bm25Algorithm {
    index: BM25Index,
    config: Bm25Config,
}

impl Bm25Algorithm {
    /// Create a new BM25 algorithm wrapping an existing `BM25Index`.
    pub fn new(index: BM25Index) -> Self {
        Self { index, config: Bm25Config::default() }
    }

    /// Access the underlying `BM25Index`.
    pub fn index(&self) -> &BM25Index {
        &self.index
    }

    /// Access the underlying mutable `BM25Index`.
    pub fn index_mut(&mut self) -> &mut BM25Index {
        &mut self.index
    }

    /// Access BM25 configuration.
    pub fn config(&self) -> &Bm25Config {
        &self.config
    }

    /// Clear all documents from the BM25 index.
    pub fn clear(&mut self) -> Result<()> {
        RetrievalAlgorithm::clear(self)
    }

    /// Commit pending changes in the Tantivy index.
    pub fn commit(&mut self) -> Result<()> {
        RetrievalAlgorithm::commit(self)
    }
}

impl std::ops::Deref for Bm25Algorithm {
    type Target = BM25Index;

    fn deref(&self) -> &Self::Target {
        &self.index
    }
}

impl std::ops::DerefMut for Bm25Algorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.index
    }
}

impl RetrievalAlgorithm for Bm25Algorithm {
    fn name(&self) -> &'static str {
        "bm25"
    }

    fn open(&mut self, index_dir: &Path) -> Result<()> {
        let tantivy_dir = index_dir.join("tantivy");
        self.index = BM25Index::open(&tantivy_dir)?;
        Ok(())
    }

    fn index_document(&mut self, doc: &ParsedArtifact) -> Result<()> {
        let title = doc
            .title
            .as_deref()
            .or_else(|| doc.doc_metadata.as_ref().and_then(|d| d.title.as_deref()));

        let tags: Vec<String> =
            doc.doc_metadata.as_ref().map(|d| d.tags.clone()).unwrap_or_default();

        self.index.remove_document(&doc.path)?;
        self.index.add_document(&doc.path, title, &tags, &doc.chunks)?;
        Ok(())
    }

    fn remove_document(&mut self, path: &str) -> Result<()> {
        self.index.remove_document(path)
    }

    fn commit(&mut self) -> Result<()> {
        self.index.commit()
    }

    fn clear(&mut self) -> Result<()> {
        self.index.clear()
    }

    fn search(&self, query: &str, limit: usize, modality: Modality) -> Result<Vec<SearchResult>> {
        self.index.search_with_modality(query, limit, modality)
    }
}
