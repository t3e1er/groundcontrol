//! Dense ONNX neural embeddings retrieval algorithm component.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use groundcontrol_common::ports::RetrievalAlgorithm;
use groundcontrol_common::types::{ChunkEmbedPolicy, Modality, ParsedArtifact, SearchResult};
use groundcontrol_common::{Error, Result};

use crate::embedding::Embedder;
use crate::vector_index::VectorIndex;

#[cfg(test)]
pub mod tests;
pub mod types;

pub use types::DenseConfig;

/// Dense neural retrieval algorithm backed by an HNSW vector index and ONNX embedder.
pub struct DenseAlgorithm {
    vector_index: VectorIndex,
    embedder: Option<Arc<Embedder>>,
    config: DenseConfig,
    index_path: Option<PathBuf>,
}

impl DenseAlgorithm {
    /// Create a new dense algorithm wrapping a `VectorIndex` and optional `Embedder`.
    pub fn new(vector_index: VectorIndex, embedder: Option<Arc<Embedder>>) -> Self {
        let dims = vector_index.dimensions();
        Self {
            vector_index,
            embedder,
            config: DenseConfig { dimensions: dims, ..Default::default() },
            index_path: None,
        }
    }

    /// Access the underlying `VectorIndex`.
    pub fn vector_index(&self) -> &VectorIndex {
        &self.vector_index
    }

    /// Access the mutable underlying `VectorIndex`.
    pub fn vector_index_mut(&mut self) -> &mut VectorIndex {
        &mut self.vector_index
    }

    /// Attach an active `Embedder`.
    pub fn set_embedder(&mut self, embedder: Option<Arc<Embedder>>) {
        self.embedder = embedder;
    }

    /// Access dense configuration.
    pub fn config(&self) -> &DenseConfig {
        &self.config
    }

    /// Clear all vectors from the index.
    pub fn clear(&mut self) -> Result<()> {
        RetrievalAlgorithm::clear(self)
    }

    /// Commit pending vector changes to disk.
    pub fn commit(&mut self) -> Result<()> {
        RetrievalAlgorithm::commit(self)
    }
}

impl std::ops::Deref for DenseAlgorithm {
    type Target = VectorIndex;

    fn deref(&self) -> &Self::Target {
        &self.vector_index
    }
}

impl std::ops::DerefMut for DenseAlgorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.vector_index
    }
}

impl RetrievalAlgorithm for DenseAlgorithm {
    fn name(&self) -> &'static str {
        "dense"
    }

    fn open(&mut self, index_dir: &Path) -> Result<()> {
        let bin_path = index_dir.join("vectors.bin");
        self.index_path = Some(bin_path.clone());
        if bin_path.exists() {
            self.vector_index = VectorIndex::load(&bin_path)?;
        }
        Ok(())
    }

    fn index_document(&mut self, doc: &ParsedArtifact) -> Result<()> {
        // Clear previous vectors for this file
        self.vector_index.remove_document(&doc.path);

        if doc.is_code {
            // Source code is not dense embedded (uses BM25 and 256-bit binary hamming)
            return Ok(());
        }

        let embedder = match self.embedder.as_ref() {
            Some(e) => e,
            None => return Ok(()),
        };

        let anchor_chunks: Vec<_> =
            doc.chunks.iter().filter(|c| c.embed_policy == ChunkEmbedPolicy::Anchor).collect();

        if anchor_chunks.is_empty() {
            return Ok(());
        }

        let texts: Vec<&str> = anchor_chunks.iter().map(|c| c.text.as_str()).collect();
        if let Ok(embeddings) = embedder.embed_batch(&texts) {
            for (chunk, emb) in anchor_chunks.into_iter().zip(embeddings) {
                self.vector_index.add(&emb, &doc.path, Some(chunk.chunk_index), false, "docs")?;
            }
        }

        Ok(())
    }

    fn remove_document(&mut self, path: &str) -> Result<()> {
        self.vector_index.remove_document(path);
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        if let Some(ref path) = self.index_path {
            if self.vector_index.is_dirty() && !self.vector_index.is_empty() {
                self.vector_index.save_binary(path)?;
            }
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        let dims = self.vector_index.dimensions();
        self.vector_index = VectorIndex::new_default(dims);
        Ok(())
    }

    fn search(&self, query: &str, limit: usize, modality: Modality) -> Result<Vec<SearchResult>> {
        let embedder = self.embedder.as_ref().ok_or_else(|| {
            Error::Index("Dense embeddings unavailable: embedder not initialized".to_string())
        })?;

        let query_embedding = embedder.embed(query)?;
        let hits = self.vector_index.search(&query_embedding, limit, false, modality)?;
        let results = hits
            .into_iter()
            .map(|r| {
                let mut item = SearchResult::new(r.doc_path, r.score as f64);
                if let Some(c) = r.chunk_index {
                    item = item.with_chunk_index(Some(c));
                }
                item
            })
            .collect();
        Ok(results)
    }
}
