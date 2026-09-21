//! Core-side engine builder/factory.
//!
//! [`EngineBuilder`] owns the path-derivation, backend-construction, vector
//! staleness reconciliation, and edge-type persistence that used to live inline
//! in `Engine::open`. It is the single place where the concrete adapters
//! (`Store`, `BM25Index`, `KnowledgeGraph`, `VectorIndex`) are constructed and
//! then handed to [`Engine::from_parts`] for pure assembly.
//!
//! Per Approach B of the ports-and-adapters refactor, `Engine` stays a single
//! concrete type and nothing is generic: this builder wires the concrete
//! adapters (which `core` legitimately owns) and injects them into the engine.
//! `Engine::open` is a thin delegate to [`EngineBuilder::open`], so all existing
//! call sites keep working while the adapter-construction sequence lives in
//! exactly one place.

use std::fs;
use std::path::Path;

use tracing::{info, warn};

use groundcontrol_common::config::CorpusConfig;
use groundcontrol_common::types::EdgeTypeRecord;
use groundcontrol_common::Result;

use crate::engine::Engine;
use crate::graph::KnowledgeGraph;
use crate::index::BM25Index;
use crate::persistence::Store;
use crate::vector_index::VectorIndex;

/// Core-side factory that constructs the concrete adapters for an [`Engine`].
///
/// Construction (path derivation, opening the SQLite store / Tantivy index,
/// loading or creating the knowledge graph and vector index, vector staleness
/// reconciliation, and edge-type persistence) lives here, ending in a call to
/// [`Engine::from_parts`]. This is callable by both `CorpusManager` and the CLI
/// composition root so there is no duplicate construction path.
pub struct EngineBuilder;

impl EngineBuilder {
    /// Create or open an engine for a corpus.
    ///
    /// - `config`: Corpus configuration (includes path, chunking settings, graph edge types).
    /// - `index_dir`: Path to the `.index/` directory. Will be created if it doesn't exist.
    ///
    /// Initializes SQLite store, Tantivy BM25 index, knowledge graph, and vector index (if in Full mode).
    /// The embedder is initialized lazily on first use via `Engine::ensure_embedder()`.
    pub fn open(config: CorpusConfig, index_dir: &Path) -> Result<Engine> {
        // 1. Create index directory if needed.
        fs::create_dir_all(index_dir)?;

        // 2. Open SQLite store and persist active corpus configuration.
        let db_path = index_dir.join("meta.db");
        let store = Store::open(&db_path)?;
        if let Ok(json_cfg) = serde_json::to_string(&config) {
            let _ = store.set_config("corpus_config", &json_cfg);
        }

        // 3. Open BM25 index.
        let tantivy_path = index_dir.join("tantivy");
        let bm25 = BM25Index::open(&tantivy_path)?;

        // 4. Load or create knowledge graph.
        let graph_path = index_dir.join("graph.bin");
        let graph = if graph_path.exists() {
            KnowledgeGraph::load(&graph_path).unwrap_or_else(|e| {
                warn!("Failed to load graph from disk, starting fresh: {}", e);
                KnowledgeGraph::new()
            })
        } else {
            KnowledgeGraph::new()
        };

        // 5. Load or create vector index (skipped entirely in Fast Mode).
        let vector_index = match config.index_mode {
            groundcontrol_common::config::IndexMode::Fast => {
                info!(corpus = %config.name, "Fast Mode enabled: skipping vector index allocation and ONNX embedder initialization");
                None
            }
            groundcontrol_common::config::IndexMode::Full => {
                let configured_model_name =
                    crate::embedding::ModelName::from_str_name(&config.embedding.model)
                        .unwrap_or_default();
                let configured_dimensions = configured_model_name.dimensions();
                let configured_model_version = configured_model_name.version_string();

                let vector_path = index_dir.join("vectors.bin");
                let mut vi = if vector_path.exists() {
                    VectorIndex::load_binary(&vector_path).unwrap_or_else(|e| {
                        warn!("Failed to load vector index from disk, starting fresh: {}", e);
                        VectorIndex::new_default(configured_dimensions)
                    })
                } else {
                    VectorIndex::new_default(configured_dimensions)
                };

                // Check model version staleness and dimension match.
                if vi.dimensions() != configured_dimensions {
                    warn!(
                        "Vector index dimension mismatch: stored={}, configured={}. Recreating index with {} dimensions and marking stale.",
                        vi.dimensions(),
                        configured_dimensions,
                        configured_dimensions
                    );
                    vi = VectorIndex::new_default(configured_dimensions);
                    vi.set_model_version(&configured_model_version);
                    vi.mark_stale();
                } else if let Some(stored_version) = vi.model_version() {
                    let is_compatible = stored_version == configured_model_version
                        || (stored_version.starts_with("jina-embeddings-v2-base-code")
                            && configured_model_version
                                .starts_with("jina-embeddings-v2-base-code"));
                    if !is_compatible {
                        warn!(
                            "Embedding model version mismatch: stored='{}', configured='{}'. Vectors marked as stale.",
                            stored_version, configured_model_version
                        );
                        vi.mark_stale();
                    } else {
                        vi.clear_stale();
                    }
                } else if !vi.is_empty() {
                    warn!(
                        "Vector index has no model_version metadata. Marking as stale for safety."
                    );
                    vi.mark_stale();
                } else {
                    vi.set_model_version(&configured_model_version);
                }

                Some(vi)
            }
        };

        // 6. Register edge type configs in the persistence store.
        let edge_type_records: Vec<EdgeTypeRecord> = config
            .graph
            .edge_types
            .iter()
            .map(|et| EdgeTypeRecord {
                name: et.name.clone(),
                source: format!("{:?}", et.source).to_lowercase(),
                weight: et.weight,
                bidirectional: et.bidirectional,
                field: et.field.clone(),
                config: None,
            })
            .collect();
        if !edge_type_records.is_empty() {
            store.insert_edge_types(&edge_type_records)?;
        }

        // 7. Load or create binary search index (fingerprints.bin).
        let fingerprints_path = index_dir.join("fingerprints.bin");
        let binary_index = if fingerprints_path.exists() {
            crate::search::binary::BinarySearchIndex::load_from_path(&fingerprints_path)
                .unwrap_or_else(|e| {
                    warn!("Failed to load fingerprints from disk, starting fresh: {}", e);
                    crate::search::binary::BinarySearchIndex::new()
                })
        } else {
            crate::search::binary::BinarySearchIndex::new()
        };

        Ok(Engine::from_parts(
            config,
            index_dir.to_path_buf(),
            store,
            bm25,
            graph,
            vector_index,
            binary_index,
        ))
    }
}
