//! Stage-by-stage indexing performance and resource profiler.

use std::fs;
use std::path::Path;
use std::time::Instant;

use ctxvault_common::config::CorpusConfig;
use ctxvault_common::ports::GraphStore;
use ctxvault_core::engine::Engine;
use serde::{Deserialize, Serialize};

use super::disk::{DiskBreakdown, DiskProfiler};
use super::memory::{MemoryMetrics, MemoryTracker};

/// Comprehensive report of an indexing run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingProfileReport {
    /// Name or path of the corpus.
    pub corpus_path: String,
    /// Total wall-clock time in milliseconds.
    pub total_elapsed_ms: f64,
    /// Reindex stage time in milliseconds.
    pub reindex_stage_ms: f64,
    /// Commit & flush stage time in milliseconds.
    pub commit_stage_ms: f64,
    /// Dense reembed stage time in milliseconds (if executed).
    pub reembed_stage_ms: Option<f64>,
    /// Number of documents/files reindexed.
    pub documents_indexed: usize,
    /// Number of chunks embedded (if reembed was run).
    pub chunks_embedded: Option<usize>,
    /// Graph node count.
    pub graph_node_count: usize,
    /// Graph edge count.
    pub graph_edge_count: usize,
    /// Throughput: documents indexed per second.
    pub docs_per_second: f64,
    /// Memory metrics captured during indexing.
    pub memory: MemoryMetrics,
    /// Disk footprint breakdown after indexing.
    pub disk: DiskBreakdown,
}

/// Options configuring an index profiling run.
#[derive(Debug, Clone)]
pub struct IndexProfilerOptions {
    /// Whether to run dense ONNX vector embedding (`reembed`).
    pub include_dense_embedding: bool,
    /// Whether to clean the existing `.index/` directory first for a cold-start benchmark.
    pub clean_cold_start: bool,
}

impl Default for IndexProfilerOptions {
    fn default() -> Self {
        Self { include_dense_embedding: false, clean_cold_start: false }
    }
}

/// Runner for profiling the indexing pipeline.
pub struct IndexProfiler;

impl IndexProfiler {
    /// Execute and profile a full indexing run on the specified corpus directory.
    pub fn profile(
        corpus_dir: &Path,
        options: &IndexProfilerOptions,
    ) -> ctxvault_common::Result<IndexingProfileReport> {
        let index_dir = corpus_dir.join(".index");
        let config_path = corpus_dir.join("ctxvault.toml");

        if options.clean_cold_start && index_dir.exists() {
            let _ = fs::remove_dir_all(&index_dir);
        }

        // Clean stale lock files if directory exists
        let tantivy_dir = index_dir.join("tantivy");
        let _ = fs::remove_file(tantivy_dir.join(".tantivy-meta.lock"));
        let _ = fs::remove_file(tantivy_dir.join(".tantivy-writer.lock"));

        let mut config: CorpusConfig = if config_path.exists() {
            let config_str = fs::read_to_string(&config_path).map_err(|e| {
                ctxvault_common::Error::Config(format!("Failed to read ctxvault.toml: {e}"))
            })?;
            toml::from_str(&config_str).map_err(|e| {
                ctxvault_common::Error::Config(format!("Failed to parse ctxvault.toml: {e}"))
            })?
        } else {
            CorpusConfig { path: corpus_dir.to_string_lossy().to_string(), ..Default::default() }
        };

        if !options.include_dense_embedding {
            config.index_mode = ctxvault_common::config::IndexMode::Fast;
        }

        let mut mem_tracker = MemoryTracker::start();
        let start_total = Instant::now();

        // 1. Open engine
        let mut engine = Engine::open(config, &index_dir)?;
        mem_tracker.sample();

        // 2. Reindex stage (AST parse, Tantivy BM25, SIF + Binary Fingerprints, Graph edges)
        let t_reindex_start = Instant::now();
        let documents_indexed = engine.full_reindex()?;
        let reindex_stage_ms = t_reindex_start.elapsed().as_secs_f64() * 1000.0;
        mem_tracker.sample();

        // 3. Optional dense neural embedding stage
        let (chunks_embedded, reembed_stage_ms) = if options.include_dense_embedding {
            let _ = engine.ensure_embedder()?;
            let t_embed_start = Instant::now();
            let count = engine.reembed()?;
            let ms = t_embed_start.elapsed().as_secs_f64() * 1000.0;
            mem_tracker.sample();
            (Some(count), Some(ms))
        } else {
            (None, None)
        };

        // 4. Commit stage
        let t_commit_start = Instant::now();
        engine.commit()?;
        let commit_stage_ms = t_commit_start.elapsed().as_secs_f64() * 1000.0;

        let total_elapsed_ms = start_total.elapsed().as_secs_f64() * 1000.0;
        let memory = mem_tracker.finish();

        let graph_stats = engine.graph().stats();
        let docs_per_second = if total_elapsed_ms > 0.0 {
            (documents_indexed as f64 / total_elapsed_ms) * 1000.0
        } else {
            0.0
        };

        let disk = DiskProfiler::profile(corpus_dir).map_err(|e| ctxvault_common::Error::Io(e))?;

        Ok(IndexingProfileReport {
            corpus_path: corpus_dir.display().to_string(),
            total_elapsed_ms,
            reindex_stage_ms,
            commit_stage_ms,
            reembed_stage_ms,
            documents_indexed,
            chunks_embedded,
            graph_node_count: graph_stats.node_count,
            graph_edge_count: graph_stats.edge_count,
            docs_per_second,
            memory,
            disk,
        })
    }
}
