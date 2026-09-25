//! 256-Bit binaryv3 algorithmic retrieval component.
//!
//! Unifies:
//! 1. Pure Interface-Semantic 256-bit Vector Projection (SIF over interface intent only)
//! 2. Zero-Cost Bayesian Structural Prior (1-byte packed EntityPriorFlags post-Hamming multiplier)
//! 3. True Matryoshka Representation Learning (MRL) prefix slicing (V_64 ⊂ V_128 ⊂ V_256)

use std::path::{Path, PathBuf};

use groundcontrol_common::ports::RetrievalAlgorithm;
use groundcontrol_common::types::{EntityKind, Modality, ParsedArtifact, SearchResult};
use groundcontrol_common::Result;

pub mod index;
pub mod projector;
#[cfg(test)]
mod tests;
pub mod tokenizer;
pub mod types;

pub use index::BinaryV3SearchIndex;
pub use projector::BinaryV3Projector;
pub use tokenizer::{expand_tokens_morphology, tokenize_code_text, ExtractedToken, TokenKind};
pub use types::{BinaryV3Config, EntityPriorFlags, FingerprintV3Record};

/// BinaryV3 retrieval algorithm backed by 256-bit Matryoshka Hamming scan and Bayesian structural prior.
pub struct BinaryV3Algorithm {
    index: BinaryV3SearchIndex,
    config: BinaryV3Config,
    index_path: Option<PathBuf>,
}

impl Default for BinaryV3Algorithm {
    fn default() -> Self {
        Self::new()
    }
}

impl BinaryV3Algorithm {
    /// Create a new binaryv3 algorithm with an empty index and default configuration.
    pub fn new() -> Self {
        Self {
            index: BinaryV3SearchIndex::new(),
            config: BinaryV3Config::default(),
            index_path: None,
        }
    }

    /// Access the underlying `BinaryV3SearchIndex`.
    pub fn index(&self) -> &BinaryV3SearchIndex {
        &self.index
    }

    /// Access the mutable underlying `BinaryV3SearchIndex`.
    pub fn index_mut(&mut self) -> &mut BinaryV3SearchIndex {
        &mut self.index
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

impl std::ops::Deref for BinaryV3Algorithm {
    type Target = BinaryV3SearchIndex;

    fn deref(&self) -> &Self::Target {
        &self.index
    }
}

impl std::ops::DerefMut for BinaryV3Algorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.index
    }
}

impl RetrievalAlgorithm for BinaryV3Algorithm {
    fn name(&self) -> &'static str {
        "binaryv3"
    }

    fn open(&mut self, index_dir: &Path) -> Result<()> {
        let bin_path = index_dir.join("fingerprints_v3.bin");
        self.index_path = Some(bin_path.clone());
        if bin_path.exists() {
            self.index = BinaryV3SearchIndex::load_from_path(&bin_path)?;
        }
        Ok(())
    }

    fn index_document(&mut self, doc: &ParsedArtifact) -> Result<()> {
        let mut fps = Vec::new();
        let modality = if doc.is_code { Modality::Code } else { Modality::Docs };

        // 1. File-level fingerprint
        if let Some(ref content) = doc.raw_content {
            let fp = self.index.projector().project_document(&doc.path, content);
            let flags = EntityPriorFlags::from_path(&doc.path);
            fps.push(FingerprintV3Record {
                id: doc.path.clone(),
                fingerprint: fp,
                modality,
                flags,
            });
        }

        // 2. Symbols or chunks
        if doc.is_code {
            for sym in &doc.symbols {
                let fp = self.index.projector().project_symbol(sym, &doc.path);
                let flags = EntityPriorFlags::from_symbol(sym, &doc.path);
                fps.push(FingerprintV3Record {
                    id: format!("{}#{}", doc.path, sym.scope_path),
                    fingerprint: fp,
                    modality: Modality::Code,
                    flags,
                });
            }

            for chunk in &doc.chunks {
                let fp = self.index.projector().project_chunk(&doc.path, &chunk.text);
                let flags = EntityPriorFlags::from_chunk(&doc.path);
                fps.push(FingerprintV3Record {
                    id: format!("{}:chunk:{}", doc.path, chunk.chunk_index),
                    fingerprint: fp,
                    modality: Modality::Code,
                    flags,
                });
            }
        } else {
            for chunk in &doc.chunks {
                let fp = self.index.projector().project_chunk(&doc.path, &chunk.text);
                let flags = EntityPriorFlags::from_chunk(&doc.path);
                fps.push(FingerprintV3Record {
                    id: if chunk.chunk_index == 0 {
                        doc.path.clone()
                    } else {
                        format!("{}:chunk:{}", doc.path, chunk.chunk_index)
                    },
                    fingerprint: fp,
                    modality: Modality::Docs,
                    flags,
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
        let q_fp = self.index.project_query(query);
        let pool_size = (limit * self.config.pool_multiplier).max(500);
        let candidates = self.index.search_candidates(&q_fp, pool_size, modality)?;

        let mut results = Vec::with_capacity(candidates.len().min(limit));
        for (id, _dist, score) in candidates.into_iter().take(limit) {
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
            let mut item =
                SearchResult::new(path, score).with_entity_kind(entity_kind).with_symbol(symbol);
            if let Some(c) = chunk_index {
                item = item.with_chunk_index(Some(c));
            }
            results.push(item);
        }

        Ok(results)
    }
}
