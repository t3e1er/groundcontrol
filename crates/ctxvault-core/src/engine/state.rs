//! Core Engine state, assembly, lifecycle, and adapter accessors.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use tracing::warn;

use ctxvault_common::config::CorpusConfig;
use ctxvault_common::ports::{GraphStore, MetadataCatalog};
use ctxvault_common::Result;

use crate::embedding::Embedder;
use crate::graph::KnowledgeGraph;
use crate::index::BM25Index;
use crate::persistence::Store;
use crate::vector_index::VectorIndex;

/// Coordinates persistence, full-text index, knowledge graph, and vector index for a corpus.
pub struct Engine {
    pub(crate) config: CorpusConfig,
    pub(crate) store: Store,
    pub(crate) bm25: BM25Index,
    pub(crate) graph: KnowledgeGraph,
    pub(crate) vector_index: Option<VectorIndex>,
    pub(crate) binary_index: crate::search::binary::BinarySearchIndex,
    pub(crate) embedder: RwLock<Option<Arc<Embedder>>>,
    pub(crate) index_dir: PathBuf,
    pub(crate) exclude_matcher: Arc<crate::index::exclude::ExcludeMatcher>,
    pub(crate) classifier: Arc<crate::index::classifier::FileClassifier>,
}

impl Engine {
    /// Assemble an `Engine` from already-constructed adapters.
    pub fn from_parts(
        config: CorpusConfig,
        index_dir: PathBuf,
        store: Store,
        bm25: BM25Index,
        graph: KnowledgeGraph,
        vector_index: Option<VectorIndex>,
        binary_index: crate::search::binary::BinarySearchIndex,
    ) -> Self {
        let corpus_root = PathBuf::from(&config.path);
        let exclude_matcher =
            Arc::new(crate::index::exclude::ExcludeMatcher::new(&corpus_root, &config.exclude));
        let classifier =
            Arc::new(crate::index::classifier::FileClassifier::new(&corpus_root, &config));
        Self {
            config,
            store,
            bm25,
            graph,
            vector_index,
            binary_index,
            embedder: RwLock::new(None), // Lazily initialized
            index_dir,
            exclude_matcher,
            classifier,
        }
    }

    /// Create or open an engine for a corpus.
    pub fn open(config: CorpusConfig, index_dir: &Path) -> Result<Self> {
        crate::engine_builder::EngineBuilder::open(config, index_dir)
    }

    /// Ensure the embedder is initialized. Returns Ok(true) if available, Ok(false) if skipped.
    pub fn ensure_embedder(&self) -> Result<bool> {
        if self.config.index_mode == ctxvault_common::config::IndexMode::Fast {
            return Ok(false);
        }
        {
            let guard = self.embedder.read().unwrap();
            if guard.is_some() {
                return Ok(true);
            }
        }

        let model_str = &self.config.embedding.model;
        match Embedder::from_config(model_str) {
            Ok(embedder) => {
                let arc = Arc::new(embedder);
                let mut guard = self.embedder.write().unwrap();
                if guard.is_none() {
                    *guard = Some(arc);
                }
                Ok(true)
            }
            Err(e) => {
                warn!("Could not initialize embedder, vector indexing disabled: {}", e);
                Ok(false)
            }
        }
    }

    /// Return an `Arc<Embedder>` clone if the embedder is initialized.
    pub fn embedder_arc(&self) -> Option<Arc<Embedder>> {
        self.embedder.read().unwrap().clone()
    }

    /// Construct a [`CoreSearchService`] borrowing all required ports from this engine.
    pub fn search_service(&self) -> crate::search_service::CoreSearchService<'_> {
        crate::search_service::CoreSearchService::new(
            &self.bm25,
            self.vector_index.as_ref(),
            Some(&self.binary_index),
            &self.graph,
            self.embedder_arc(),
            self.code_paths_set(),
        )
    }

    /// Whether this engine is running in fast mode (no vector index or embeddings).
    pub fn is_fast_mode(&self) -> bool {
        self.config.index_mode == ctxvault_common::config::IndexMode::Fast
    }

    /// Get a reference to the binary search index.
    pub fn binary_index(&self) -> &crate::search::binary::BinarySearchIndex {
        &self.binary_index
    }

    /// Get a mutable reference to the binary search index.
    pub fn binary_index_mut(&mut self) -> &mut crate::search::binary::BinarySearchIndex {
        &mut self.binary_index
    }

    /// Dynamically update the index mode (e.g. from Fast to Full).
    pub fn set_index_mode(&mut self, mode: ctxvault_common::config::IndexMode) {
        self.config.index_mode = mode;
        if mode == ctxvault_common::config::IndexMode::Full {
            self.ensure_vector_index();
        }
    }

    /// Ensure the vector index is instantiated (used when switching to Full mode).
    pub fn ensure_vector_index(&mut self) {
        if self.config.index_mode == ctxvault_common::config::IndexMode::Full
            && self.vector_index.is_none()
        {
            let dim = self.embedding_dimension();
            let mut vi = VectorIndex::new_default(dim);
            let vi_path = self.index_dir.join("vectors.bin");
            if vi_path.exists() {
                if let Ok(loaded) = VectorIndex::load(&vi_path) {
                    vi = loaded;
                }
            }
            self.vector_index = Some(vi);
        }
    }

    /// Check if a vector index is available (loaded and not Fast mode).
    pub fn has_vector_index(&self) -> bool {
        self.vector_index.is_some()
    }

    /// Check if the embedder is currently active and usable.
    pub fn embedder_active(&self) -> bool {
        if self.config.index_mode == ctxvault_common::config::IndexMode::Fast {
            return false;
        }
        let guard = self.embedder.read().unwrap();
        guard.is_some()
    }

    /// Number of vectors in the vector index (0 if not in Full mode).
    pub fn vector_count(&self) -> usize {
        self.vector_index.as_ref().map(|vi| vi.len()).unwrap_or(0)
    }

    /// Get hardware acceleration runtime status for embeddings.
    pub fn hardware_acceleration(&self) -> String {
        if let Some(embedder) = self.embedder.read().unwrap().as_ref() {
            let name = embedder.governor().provider_name();
            match name {
                "DirectML" => "DirectML (GPU)".to_string(),
                "CoreML" => "CoreML (GPU)".to_string(),
                "CUDA" => "CUDA (GPU)".to_string(),
                other => other.to_string(),
            }
        } else {
            "CPU".to_string()
        }
    }

    /// Get a reference to the knowledge graph implementing [`GraphStore`].
    pub fn graph(&self) -> &impl GraphStore {
        &self.graph
    }

    /// Get a direct reference to the concrete [`KnowledgeGraph`].
    pub fn knowledge_graph(&self) -> &crate::graph::KnowledgeGraph {
        &self.graph
    }

    /// Get a mutable reference to the knowledge graph implementing [`GraphStore`].
    pub fn graph_mut(&mut self) -> &mut impl GraphStore {
        &mut self.graph
    }

    /// Get a reference to the metadata store implementing [`MetadataCatalog`].
    pub fn store(&self) -> &impl MetadataCatalog {
        &self.store
    }

    /// Get a reference to the corpus configuration.
    pub fn config(&self) -> &CorpusConfig {
        &self.config
    }

    /// Get a mutable reference to the corpus configuration.
    pub fn config_mut(&mut self) -> &mut CorpusConfig {
        &mut self.config
    }

    /// Get the path to the `.index/` directory.
    pub fn index_dir(&self) -> &Path {
        &self.index_dir
    }

    /// Get the exclude matcher for this engine.
    pub fn exclude_matcher(&self) -> &Arc<crate::index::exclude::ExcludeMatcher> {
        &self.exclude_matcher
    }

    /// Get the file classifier for this engine.
    pub fn classifier(&self) -> &Arc<crate::index::classifier::FileClassifier> {
        &self.classifier
    }

    /// Return the on-disk cache path for a projectable binary/document file.
    pub fn projection_path(&self, rel_path: &str) -> PathBuf {
        self.index_dir.join("projections").join(format!("{}.txt", rel_path))
    }

    /// Get the embedding dimensions for this engine.
    pub fn embedding_dimension(&self) -> usize {
        self.vector_index.as_ref().map(|vi| vi.dimensions()).unwrap_or_else(|| {
            crate::embedding::ModelName::from_str_name(&self.config.embedding.model)
                .unwrap_or_default()
                .dimensions()
        })
    }

    /// Check whether vectors are stale (model version mismatch).
    pub fn vectors_stale(&self) -> bool {
        self.vector_index.as_ref().map(|vi| vi.is_stale()).unwrap_or(false)
    }

    /// Check whether the corpus has been indexed (has any files in the store).
    pub fn is_indexed(&self) -> bool {
        self.store.list_files().map(|f| !f.is_empty()).unwrap_or(false)
    }

    /// Get the model version stored in the vector index.
    pub fn stored_model_version(&self) -> Option<&str> {
        self.vector_index.as_ref().and_then(|vi| vi.model_version())
    }
}
