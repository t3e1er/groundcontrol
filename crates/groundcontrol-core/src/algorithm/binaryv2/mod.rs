//! 256-Bit binaryv2 semantic retrieval algorithm component.
//!
//! Provides enhanced semantic signal boosting across 4 active orthogonal channels:
//! lexical interface, unsupervised RRI co-occurrence context, AST call topology,
//! and hierarchical path context.
//!
//! Purely code-agnostic: relies strictly on AST symbols, tree-sitter grammar semantics,
//! and corpus statistics with zero domain dictionaries or hardcoded query matchers.

use std::path::{Path, PathBuf};

use groundcontrol_common::ports::RetrievalAlgorithm;
use groundcontrol_common::types::{
    CodeSymbol, EntityKind, ExtractedGrammarSemantics, FingerprintRecord, Modality, ParsedArtifact,
    SearchResult,
};
use groundcontrol_common::Result;

pub mod index;
pub mod projector;
pub mod ranking;
pub mod rri;
#[cfg(test)]
mod tests;
pub mod tokenizer;
pub mod types;

pub use index::BinaryV2SearchIndex;
pub use projector::BinaryV2Projector;
pub use ranking::deduplicate_binary_v2_hits;
pub use rri::RriEngine;
pub use tokenizer::{expand_tokens_morphology, tokenize_code_text};
pub use types::BinaryV2Config;

/// Lightweight staged document representation for two-pass global RRI convergence.
#[derive(Clone, Debug)]
struct StagedArtifact {
    path: String,
    is_code: bool,
    raw_content: Option<String>,
    symbols: Vec<CodeSymbol>,
    grammar_semantics: Vec<ExtractedGrammarSemantics>,
    chunks: Vec<(usize, String)>,
}

/// BinaryV2 retrieval algorithm backed by 256-bit multi-channel Hamming scan.
pub struct BinaryV2Algorithm {
    index: BinaryV2SearchIndex,
    config: BinaryV2Config,
    index_path: Option<PathBuf>,
    staged_artifacts: Vec<StagedArtifact>,
}

impl Default for BinaryV2Algorithm {
    fn default() -> Self {
        Self::new()
    }
}

impl BinaryV2Algorithm {
    /// Create a new binaryv2 algorithm with an empty index and default configuration.
    pub fn new() -> Self {
        Self {
            index: BinaryV2SearchIndex::new(),
            config: BinaryV2Config::default(),
            index_path: None,
            staged_artifacts: Vec::new(),
        }
    }

    /// Access the underlying `BinaryV2SearchIndex`.
    pub fn index(&self) -> &BinaryV2SearchIndex {
        &self.index
    }

    /// Access the mutable underlying `BinaryV2SearchIndex`.
    pub fn index_mut(&mut self) -> &mut BinaryV2SearchIndex {
        &mut self.index
    }

    /// Flush staged artifacts and re-project all fingerprints with globally trained RRI vocabulary.
    pub fn flush_staged(&mut self) -> Result<()> {
        if self.staged_artifacts.is_empty() {
            return Ok(());
        }

        let mut fps = Vec::new();
        for doc in self.staged_artifacts.drain(..) {
            let modality = if doc.is_code { Modality::Code } else { Modality::Docs };

            if let Some(ref content) = doc.raw_content {
                let fp = self.index.projector().project_document_or_chunk(
                    &doc.path,
                    content,
                    self.index.rri(),
                );
                fps.push(FingerprintRecord { id: doc.path.clone(), fingerprint: fp, modality });
            }

            if doc.is_code {
                for (i, sym) in doc.symbols.iter().enumerate() {
                    let sem = doc.grammar_semantics.get(i);
                    let fp = self.index.projector().project_symbol(
                        sym,
                        &doc.path,
                        sem,
                        self.index.rri(),
                    );
                    fps.push(FingerprintRecord {
                        id: format!("{}#{}", doc.path, sym.scope_path),
                        fingerprint: fp,
                        modality: Modality::Code,
                    });
                }
                for (chunk_idx, chunk_text) in doc.chunks {
                    let fp = self.index.projector().project_document_or_chunk(
                        &doc.path,
                        &chunk_text,
                        self.index.rri(),
                    );
                    fps.push(FingerprintRecord {
                        id: format!("{}:chunk:{}", doc.path, chunk_idx),
                        fingerprint: fp,
                        modality: Modality::Code,
                    });
                }
            } else {
                for (chunk_idx, chunk_text) in doc.chunks {
                    let fp = self.index.projector().project_document_or_chunk(
                        &doc.path,
                        &chunk_text,
                        self.index.rri(),
                    );
                    fps.push(FingerprintRecord {
                        id: if chunk_idx == 0 {
                            doc.path.clone()
                        } else {
                            format!("{}:chunk:{}", doc.path, chunk_idx)
                        },
                        fingerprint: fp,
                        modality: Modality::Docs,
                    });
                }
            }
        }

        if !fps.is_empty() {
            self.index.index_fingerprints(&fps)?;
        }
        Ok(())
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

impl std::ops::Deref for BinaryV2Algorithm {
    type Target = BinaryV2SearchIndex;

    fn deref(&self) -> &Self::Target {
        &self.index
    }
}

impl std::ops::DerefMut for BinaryV2Algorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.index
    }
}

impl RetrievalAlgorithm for BinaryV2Algorithm {
    fn name(&self) -> &'static str {
        "binaryv2"
    }

    fn open(&mut self, index_dir: &Path) -> Result<()> {
        let bin_path = index_dir.join("fingerprints_v2.bin");
        self.index_path = Some(bin_path.clone());
        if bin_path.exists() {
            self.index = BinaryV2SearchIndex::load_from_path(&bin_path)?;
        }
        Ok(())
    }

    fn index_document(&mut self, doc: &ParsedArtifact) -> Result<()> {
        let modality = if doc.is_code { Modality::Code } else { Modality::Docs };

        // 1. Train RRI co-occurrence statistics unsupervised from document text and chunks
        if let Some(ref content) = doc.raw_content {
            let tokens = tokenize_code_text(content);
            self.index.rri_mut().train_chunk(&tokens);
        }
        for chunk in &doc.chunks {
            let tokens = tokenize_code_text(&chunk.text);
            self.index.rri_mut().train_chunk(&tokens);
        }
        for sym in &doc.symbols {
            let mut sym_tokens = tokenize_code_text(&sym.name);
            sym_tokens.extend(tokenize_code_text(&sym.signature));
            if let Some(ref d) = sym.docstring {
                sym_tokens.extend(tokenize_code_text(d));
            }
            self.index.rri_mut().train_chunk(&sym_tokens);
        }

        // 2. Stage artifact for two-pass global RRI convergence
        self.staged_artifacts.push(StagedArtifact {
            path: doc.path.clone(),
            is_code: doc.is_code,
            raw_content: doc.raw_content.clone(),
            symbols: doc.symbols.clone(),
            grammar_semantics: doc.grammar_semantics.clone(),
            chunks: doc.chunks.iter().map(|c| (c.chunk_index, c.text.clone())).collect(),
        });

        // 3. Immediate local projection (ensures instant availability prior to commit)
        let mut fps = Vec::new();
        if let Some(ref content) = doc.raw_content {
            let fp = self.index.projector().project_document_or_chunk(
                &doc.path,
                content,
                self.index.rri(),
            );
            fps.push(FingerprintRecord { id: doc.path.clone(), fingerprint: fp, modality });
        }

        if doc.is_code {
            for (i, sym) in doc.symbols.iter().enumerate() {
                let sem = doc.grammar_semantics.get(i);
                let fp =
                    self.index.projector().project_symbol(sym, &doc.path, sem, self.index.rri());
                fps.push(FingerprintRecord {
                    id: format!("{}#{}", doc.path, sym.scope_path),
                    fingerprint: fp,
                    modality: Modality::Code,
                });
            }

            for chunk in &doc.chunks {
                let fp = self.index.projector().project_document_or_chunk(
                    &doc.path,
                    &chunk.text,
                    self.index.rri(),
                );
                fps.push(FingerprintRecord {
                    id: format!("{}:chunk:{}", doc.path, chunk.chunk_index),
                    fingerprint: fp,
                    modality: Modality::Code,
                });
            }
        } else {
            for chunk in &doc.chunks {
                let fp = self.index.projector().project_document_or_chunk(
                    &doc.path,
                    &chunk.text,
                    self.index.rri(),
                );
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
        self.staged_artifacts.retain(|a| a.path != path);
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        // Pass 2: Re-project all staged artifacts with the complete, globally converged RRI model
        self.flush_staged()?;

        if let Some(ref path) = self.index_path {
            if !self.index.is_empty() {
                self.index.save_to_path(path)?;
            }
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        self.index.clear();
        self.staged_artifacts.clear();
        Ok(())
    }

    fn search(&self, query: &str, limit: usize, modality: Modality) -> Result<Vec<SearchResult>> {
        let q_fp = self.index.project_query(query);
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
