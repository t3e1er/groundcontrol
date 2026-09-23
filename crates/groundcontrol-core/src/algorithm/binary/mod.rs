//! 256-Bit binary fingerprint retrieval algorithm component.

use std::path::{Path, PathBuf};

use groundcontrol_common::ports::RetrievalAlgorithm;
use groundcontrol_common::types::{
    EntityKind, FingerprintRecord, Modality, ParsedArtifact, SearchResult,
};
use groundcontrol_common::Result;

use crate::search::binary::BinarySearchIndex;

#[cfg(test)]
pub mod tests;
pub mod types;

pub use types::BinaryConfig;

/// Binary retrieval algorithm backed by 256-bit Hamming scan.
pub struct BinaryAlgorithm {
    index: BinarySearchIndex,
    config: BinaryConfig,
    index_path: Option<PathBuf>,
}

impl BinaryAlgorithm {
    /// Create a new binary algorithm wrapping an existing `BinarySearchIndex`.
    pub fn new(index: BinarySearchIndex) -> Self {
        Self { index, config: BinaryConfig::default(), index_path: None }
    }

    /// Access the underlying `BinarySearchIndex`.
    pub fn index(&self) -> &BinarySearchIndex {
        &self.index
    }

    /// Access the mutable underlying `BinarySearchIndex`.
    pub fn index_mut(&mut self) -> &mut BinarySearchIndex {
        &mut self.index
    }

    /// Set the binary projection kind.
    pub fn set_projection_kind(&mut self, kind: groundcontrol_common::types::BinaryProjectionKind) {
        self.config.projection_kind = kind;
        self.index.set_projection_kind(kind);
    }

    /// Clear all binary fingerprints from the index.
    pub fn clear(&mut self) -> Result<()> {
        RetrievalAlgorithm::clear(self)
    }

    /// Commit the binary fingerprints to disk.
    pub fn commit(&mut self) -> Result<()> {
        RetrievalAlgorithm::commit(self)
    }
}

impl std::ops::Deref for BinaryAlgorithm {
    type Target = BinarySearchIndex;

    fn deref(&self) -> &Self::Target {
        &self.index
    }
}

impl std::ops::DerefMut for BinaryAlgorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.index
    }
}

impl RetrievalAlgorithm for BinaryAlgorithm {
    fn name(&self) -> &'static str {
        "binary"
    }

    fn open(&mut self, index_dir: &Path) -> Result<()> {
        let bin_path = index_dir.join("fingerprints.bin");
        self.index_path = Some(bin_path.clone());
        if bin_path.exists() {
            self.index = BinarySearchIndex::load_from_path(&bin_path)?;
        }
        Ok(())
    }

    fn index_document(&mut self, doc: &ParsedArtifact) -> Result<()> {
        let mut fps = Vec::new();
        let modality = if doc.is_code { Modality::Code } else { Modality::Docs };

        // File-level fingerprint
        if let Some(ref content) = doc.raw_content {
            let fp = self.index.project_query(content).unwrap_or_default();
            fps.push(FingerprintRecord { id: doc.path.clone(), fingerprint: fp, modality });
        }

        if doc.is_code {
            for (i, sym) in doc.symbols.iter().enumerate() {
                let fp = if let Some(sem) = doc.grammar_semantics.get(i) {
                    self.index.project_semantics(sem)
                } else {
                    let fp_text = format!(
                        "{} {} {}",
                        sym.name,
                        sym.signature,
                        sym.docstring.as_deref().unwrap_or("")
                    );
                    self.index.project_query(&fp_text).unwrap_or_default()
                };
                fps.push(FingerprintRecord {
                    id: format!("{}#{}", doc.path, sym.scope_path),
                    fingerprint: fp,
                    modality: Modality::Code,
                });
            }

            for chunk in &doc.chunks {
                let fp = self.index.project_query(&chunk.text).unwrap_or_default();
                fps.push(FingerprintRecord {
                    id: format!("{}:chunk:{}", doc.path, chunk.chunk_index),
                    fingerprint: fp,
                    modality: Modality::Code,
                });
            }
        } else {
            for chunk in &doc.chunks {
                let fp = self.index.project_query(&chunk.text).unwrap_or_default();
                fps.push(FingerprintRecord {
                    id: if chunk.chunk_index == 0 {
                        doc.path.clone()
                    } else {
                        format!("{}:chunk:{}", doc.path, chunk.chunk_index)
                    },
                    fingerprint: fp,
                    modality: Modality::Docs,
                });
            }
        }

        if !fps.is_empty() {
            self.index.index_fingerprints(&fps)?;
        }
        Ok(())
    }

    fn remove_document(&mut self, path: &str) -> Result<()> {
        self.index.remove_document(path);
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        if let Some(ref path) = self.index_path {
            if !self.index.is_empty() {
                self.index.save_to_path(path)?;
            }
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        self.index.clear();
        Ok(())
    }

    fn search(&self, query: &str, limit: usize, modality: Modality) -> Result<Vec<SearchResult>> {
        let q_fp = self.index.project_query_with_kind(query, self.config.projection_kind)?;
        let pool_size = (limit * self.config.pool_multiplier).max(500);
        let hits = self.index.search_hamming(&q_fp, pool_size, modality)?;

        let mut results = Vec::with_capacity(hits.len().min(limit));
        for (id, dist) in hits.into_iter().take(limit) {
            let sim = 1.0 - (dist as f32 / 256.0);
            let mut path = id.as_str();
            let mut symbol = None;
            let mut chunk_index = None;

            if let Some(idx) = path.find(":chunk:") {
                let chunk_str = &path[idx + 7..];
                chunk_index = chunk_str.parse::<usize>().ok();
                path = &path[..idx];
            }
            if let Some(idx) = path.find('#') {
                symbol = Some(path[idx + 1..].to_string());
                path = &path[..idx];
            }

            let entity_kind = if symbol.is_some() {
                EntityKind::CodeSymbol {
                    language: String::new(),
                    symbol_type: groundcontrol_common::types::CodeSymbolType::Function,
                    scope_path: symbol.clone().unwrap(),
                    signature: String::new(),
                }
            } else if modality == Modality::Code {
                EntityKind::CodeFile { language: String::new() }
            } else {
                EntityKind::Documentation
            };
            let mut item = SearchResult::new(path, sim as f64)
                .with_entity_kind(entity_kind)
                .with_symbol(symbol);
            if let Some(c) = chunk_index {
                item = item.with_chunk_index(Some(c));
            }
            results.push(item);
        }

        Ok(results)
    }
}
