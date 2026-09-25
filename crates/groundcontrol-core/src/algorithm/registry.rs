//! Algorithm registry managing active retrieval components.

use std::collections::HashMap;
use std::path::Path;

use groundcontrol_common::ports::RetrievalAlgorithm;
use groundcontrol_common::types::{Modality, ParsedArtifact, SearchResult};
use groundcontrol_common::{Error, Result};

use super::composite::{search_fast_composite, search_hybrid_composite, CompositeConfig};

/// Central registry managing all registered retrieval algorithm components.
pub struct AlgorithmRegistry {
    algorithms: HashMap<&'static str, Box<dyn RetrievalAlgorithm>>,
    composite_config: CompositeConfig,
}

impl Default for AlgorithmRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AlgorithmRegistry {
    /// Create an empty algorithm registry.
    pub fn new() -> Self {
        Self { algorithms: HashMap::new(), composite_config: CompositeConfig::default() }
    }

    /// Register a retrieval algorithm component.
    pub fn register(&mut self, algo: Box<dyn RetrievalAlgorithm>) {
        self.algorithms.insert(algo.name(), algo);
    }

    /// Check if an algorithm with the given name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.algorithms.contains_key(name)
    }

    /// Get an immutable reference to an algorithm by name.
    pub fn get(&self, name: &str) -> Option<&dyn RetrievalAlgorithm> {
        self.algorithms.get(name).map(|b| b.as_ref())
    }

    /// Get a mutable reference to an algorithm by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut (dyn RetrievalAlgorithm + 'static)> {
        self.algorithms.get_mut(name).map(|b| b.as_mut())
    }

    /// Open and initialize all registered algorithms in `index_dir`.
    pub fn open_all(&mut self, index_dir: &Path) -> Result<()> {
        for algo in self.algorithms.values_mut() {
            algo.open(index_dir)?;
        }
        Ok(())
    }

    /// Broadcast a parsed artifact to all registered algorithms.
    pub fn index_document(&mut self, doc: &ParsedArtifact) -> Result<()> {
        for algo in self.algorithms.values_mut() {
            algo.index_document(doc)?;
        }
        Ok(())
    }

    /// Broadcast a document removal to all registered algorithms.
    pub fn remove_document(&mut self, path: &str) -> Result<()> {
        for algo in self.algorithms.values_mut() {
            algo.remove_document(path)?;
        }
        Ok(())
    }

    /// Commit pending updates across all registered algorithms.
    pub fn commit(&mut self) -> Result<()> {
        for algo in self.algorithms.values_mut() {
            algo.commit()?;
        }
        Ok(())
    }

    /// Clear all index state across all registered algorithms.
    pub fn clear(&mut self) -> Result<()> {
        for algo in self.algorithms.values_mut() {
            algo.clear()?;
        }
        Ok(())
    }

    /// Search using a specific named algorithm.
    pub fn search(
        &self,
        name: &str,
        query: &str,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<SearchResult>> {
        let algo = self.algorithms.get(name).ok_or_else(|| {
            Error::Index(format!("Algorithm '{name}' is not registered in the active index mode"))
        })?;
        algo.search(query, limit, modality)
    }

    /// Composite fast search (3-way RRF across BM25, Binary, and Graph/PPR).
    pub fn search_fast(
        &self,
        query: &str,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<SearchResult>> {
        let bm25 = self
            .get("bm25")
            .ok_or_else(|| Error::Index("bm25 algorithm required for fast search".into()))?;
        let binary = self
            .get("binaryv3")
            .or_else(|| self.get("binary"))
            .ok_or_else(|| Error::Index("binary algorithm required for fast search".into()))?;
        let ppr = self
            .get("ppr")
            .ok_or_else(|| Error::Index("ppr algorithm required for fast search".into()))?;

        search_fast_composite(bm25, binary, ppr, query, limit, modality, &self.composite_config)
    }

    /// Composite hybrid search (3-way RRF across BM25, Dense, and Graph).
    pub fn search_hybrid(
        &self,
        query: &str,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<SearchResult>> {
        let bm25 = self
            .get("bm25")
            .ok_or_else(|| Error::Index("bm25 algorithm required for hybrid search".into()))?;
        let dense = self.get("dense").ok_or_else(|| {
            Error::Index(
                "dense algorithm required for hybrid search (unavailable in fast mode)".into(),
            )
        })?;
        let ppr = self
            .get("ppr")
            .ok_or_else(|| Error::Index("ppr algorithm required for hybrid search".into()))?;

        search_hybrid_composite(bm25, dense, ppr, query, limit, modality, &self.composite_config)
    }
}
