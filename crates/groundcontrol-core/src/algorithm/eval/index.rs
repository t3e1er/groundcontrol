//! Algorithmic index builder, loader, and query runner for evaluation.

use std::path::Path;
use std::time::Instant;

use groundcontrol_common::config::CorpusConfig;
use groundcontrol_common::types::Modality;
use groundcontrol_common::{Error, Result};
use serde::{Deserialize, Serialize};

use super::config::AlgoConfig;
use super::hit::AlgoHit;
use crate::engine::Engine;

/// Performance and capacity statistics from an index operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    /// Number of source documents indexed.
    pub documents: usize,
    /// Number of AST and document nodes in the knowledge graph.
    pub graph_nodes: usize,
    /// Number of typed edges in the knowledge graph.
    pub graph_edges: usize,
    /// Elapsed indexing time in milliseconds.
    pub time_ms: f64,
}

/// An indexed corpus ready for per-algorithm querying and evaluation.
///
/// Holds a groundcontrol `Engine` internally and executes queries via direct in-process Rust function calls.
pub struct AlgorithmicIndex {
    engine: Engine,
    config: AlgoConfig,
}

impl AlgorithmicIndex {
    /// Build a new index for the corpus at `corpus_path` with `.index/` under the corpus root.
    pub fn build(corpus_path: &Path, config: AlgoConfig) -> Result<(Self, IndexStats)> {
        let index_dir = corpus_path.join(".index");
        Self::build_with_index_dir(corpus_path, &index_dir, config)
    }

    /// Build a new index for the corpus at `corpus_path` storing indices in `index_dir`.
    pub fn build_with_index_dir(
        corpus_path: &Path,
        index_dir: &Path,
        config: AlgoConfig,
    ) -> Result<(Self, IndexStats)> {
        if !corpus_path.exists() {
            return Err(Error::NotFound(format!(
                "corpus path does not exist: {}",
                corpus_path.display()
            )));
        }

        let config_file = corpus_path.join("groundcontrol.toml");
        let corpus_config = if config_file.exists() {
            let config_str = std::fs::read_to_string(&config_file)?;
            toml::from_str(&config_str)
                .map_err(|e| Error::Config(format!("Failed to parse groundcontrol.toml: {e}")))?
        } else {
            let mut exclude = groundcontrol_common::config::ExcludeConfig::default();
            let gitignore_path = corpus_path.join(".gitignore");
            if gitignore_path.exists() {
                exclude.import_gitignore(&gitignore_path);
            }
            CorpusConfig {
                path: corpus_path.to_string_lossy().to_string(),
                exclude,
                ..Default::default()
            }
        };

        let start_time = Instant::now();
        let mut engine = Engine::open(corpus_config, index_dir)?;
        engine.binary_index_mut().set_projection_kind(config.binary_projection);

        let documents = engine.full_reindex()?;
        engine.commit()?;
        let time_ms = start_time.elapsed().as_secs_f64() * 1000.0;

        let graph_stats = engine.knowledge_graph().stats();
        let stats = IndexStats {
            documents,
            graph_nodes: graph_stats.node_count,
            graph_edges: graph_stats.edge_count,
            time_ms,
        };

        Ok((Self { engine, config }, stats))
    }

    /// Load an existing index for the corpus at `corpus_path`.
    pub fn load(corpus_path: &Path, config: AlgoConfig) -> Result<Self> {
        let index_dir = corpus_path.join(".index");
        Self::load_with_index_dir(corpus_path, &index_dir, config)
    }

    /// Load an existing index from a specific `index_dir`.
    pub fn load_with_index_dir(
        corpus_path: &Path,
        index_dir: &Path,
        config: AlgoConfig,
    ) -> Result<Self> {
        if !corpus_path.exists() {
            return Err(Error::NotFound(format!(
                "corpus path does not exist: {}",
                corpus_path.display()
            )));
        }

        let config_file = corpus_path.join("groundcontrol.toml");
        let corpus_config = if config_file.exists() {
            let config_str = std::fs::read_to_string(&config_file)?;
            toml::from_str(&config_str)
                .map_err(|e| Error::Config(format!("Failed to parse groundcontrol.toml: {e}")))?
        } else {
            CorpusConfig { path: corpus_path.to_string_lossy().to_string(), ..Default::default() }
        };

        let mut engine = Engine::open(corpus_config, index_dir)?;
        engine.binary_index_mut().set_projection_kind(config.binary_projection);

        Ok(Self { engine, config })
    }

    /// Access the current algorithm configuration.
    pub fn config(&self) -> &AlgoConfig {
        &self.config
    }

    /// Update the algorithm configuration.
    pub fn set_config(&mut self, config: AlgoConfig) {
        self.engine.binary_index_mut().set_projection_kind(config.binary_projection);
        self.config = config;
    }

    /// Execute isolated binary Hamming query.
    pub fn query_binary(&self, query: &str, k: usize, modality: Modality) -> Result<Vec<AlgoHit>> {
        super::query::execute_binary_query(&self.engine, &self.config, query, k, modality)
    }

    /// Execute isolated binaryv2 multi-channel semantic Hamming query.
    pub fn query_binary_v2(
        &self,
        query: &str,
        k: usize,
        modality: Modality,
    ) -> Result<Vec<AlgoHit>> {
        super::query::execute_binary_v2_query(&self.engine, &self.config, query, k, modality)
    }

    /// Execute isolated BM25 lexical query.
    pub fn query_bm25(&self, query: &str, k: usize, modality: Modality) -> Result<Vec<AlgoHit>> {
        super::query::execute_bm25_query(&self.engine, query, k, modality)
    }

    /// Execute isolated PPR diffusion query with BM25 seed.
    pub fn query_ppr(&self, query: &str, k: usize, modality: Modality) -> Result<Vec<AlgoHit>> {
        super::query::execute_ppr_query(&self.engine, &self.config, query, k, modality)
    }

    /// Execute fast algorithmic hybrid query (BM25 + Binary + PPR).
    pub fn query_fast(&self, query: &str, k: usize, modality: Modality) -> Result<Vec<AlgoHit>> {
        super::query::execute_fast_query(&self.engine, query, k, modality)
    }

    /// Execute pure dense ONNX neural embeddings query.
    pub fn query_semantic(
        &self,
        query: &str,
        k: usize,
        modality: Modality,
    ) -> Result<Vec<AlgoHit>> {
        super::query::execute_semantic_query(&self.engine, query, k, modality)
    }

    /// Execute full 3-signal hybrid query.
    pub fn query_hybrid(&self, query: &str, k: usize, modality: Modality) -> Result<Vec<AlgoHit>> {
        super::query::execute_hybrid_query(&self.engine, query, k, modality)
    }

    /// Reference to internal engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
}
