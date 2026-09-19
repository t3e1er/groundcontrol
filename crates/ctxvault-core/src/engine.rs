//! Engine: coordinates persistence (SQLite), BM25 index (Tantivy), knowledge graph (petgraph),
//! and vector index (HNSW) with optional embedding support.
//!
//! The [`Engine`] is the top-level orchestrator for a single corpus. It manages
//! indexing, delta scanning, and provides unified access to all subsystems.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use ctxvault_common::config::{ChunkingConfig, CorpusConfig, IndexMode};
use ctxvault_common::ports::{GraphStore, MetadataCatalog};
use ctxvault_common::types::{
    ChunkEmbedPolicy, ChunkRecord, Document, Edge, EntityKind, FileFormat, IndexingState,
    IndexingStatus, Modality,
};
use ctxvault_common::{Error, Result};

use crate::embedding::Embedder;
use crate::graph::KnowledgeGraph;
use crate::index::{
    pipeline::{AsyncEmbeddingPipeline, ParsedFileRecord},
    BM25Index,
};
use crate::parser;
use crate::parser::chunker;
use crate::persistence::Store;
use crate::template::Template;
use crate::vector_index::VectorIndex;

/// Detailed indexing status response for client queries and monitoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingStatusResponse {
    /// Corpus identifier/name.
    pub corpus_id: String,
    /// Current status (idle, indexing, paused, error, completed).
    pub status: IndexingStatus,
    /// Total markdown files discovered.
    pub total_files: usize,
    /// Count of markdown files successfully committed.
    pub indexed_files: usize,
    /// Progress as a percentage (0.0 - 100.0).
    pub progress_percent: f64,
    /// Relative path of last committed file.
    pub last_processed_path: Option<String>,
    /// When indexing started (Unix timestamp seconds).
    pub started_at: i64,
    /// When status was last updated (Unix timestamp seconds).
    pub updated_at: i64,
    /// Total elapsed time in seconds.
    pub elapsed_seconds: i64,
    /// Estimated indexing throughput in documents per second.
    pub estimated_throughput_docs_per_sec: f64,
    /// Estimated time remaining in seconds until completion.
    pub estimated_time_remaining_seconds: f64,
    /// Error message if status is Error.
    pub error_message: Option<String>,
}

/// A text chunk staged for vectorized batch embedding.
#[derive(Debug, Clone)]
pub struct PendingChunk {
    /// Document relative path.
    pub doc_path: String,
    /// Chunk index within document.
    pub chunk_index: usize,
    /// Prepared context-prefixed text for embedding.
    pub text: String,
    /// Policy determining if this chunk receives a dense vector embedding.
    pub embed_policy: ChunkEmbedPolicy,
    /// Coarse modality tag ("code" / "docs") carried into the vector index.
    pub modality: String,
}

/// Coordinates persistence, full-text index, knowledge graph, and vector index for a corpus.
pub struct Engine {
    config: CorpusConfig,
    store: Store,
    bm25: BM25Index,
    graph: KnowledgeGraph,
    vector_index: Option<VectorIndex>,
    binary_index: crate::search::binary::BinarySearchIndex,
    embedder: RwLock<Option<Arc<Embedder>>>,
    index_dir: PathBuf,
    exclude_matcher: Arc<crate::index::exclude::ExcludeMatcher>,
    classifier: Arc<crate::index::classifier::FileClassifier>,
}

/// Result of a delta scan comparing filesystem state against the index.
#[derive(Debug, Clone)]
pub struct DeltaScanResult {
    /// Files that exist on disk but were not previously indexed.
    pub new_files: Vec<String>,
    /// Files whose content hash changed since last indexing.
    pub modified_files: Vec<String>,
    /// Files that were indexed but no longer exist on disk.
    pub deleted_files: Vec<String>,
}

impl Engine {
    /// Assemble an `Engine` from already-constructed adapters.
    ///
    /// This is the injecting seam of the ports-and-adapters refactor (Approach B):
    /// the caller (composition root) constructs the concrete adapters and hands
    /// them in here. Per Approach B, this builder injects **concrete** adapters
    /// (`Store`, `BM25Index`, `KnowledgeGraph`, `Option<VectorIndex>`) — not
    /// generic port parameters — and `Engine` remains a single concrete type. No
    /// generics are introduced. Construction is not yet relocated to the
    /// composition root (that is a later task); for now `Engine::open` builds the
    /// pieces and delegates its final assembly to this constructor.
    ///
    /// This is pure assembly: it performs no I/O, no staleness reconciliation, and
    /// no edge-type persistence. The embedder is left lazily uninitialized
    /// (`None`) and created on first use via `ensure_embedder()`, so it is not a
    /// parameter here.
    ///
    /// - `config`: Corpus configuration.
    /// - `index_dir`: Path to the `.index/` directory (already created by the caller).
    /// - `store`: Opened SQLite metadata store.
    /// - `bm25`: Opened Tantivy BM25 index.
    /// - `graph`: Loaded or fresh knowledge graph.
    /// - `vector_index`: Loaded or fresh vector index, or `None` in Fast Mode.
    /// - `binary_index`: Loaded or fresh 256-bit binary fingerprints index.
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
    ///
    /// - `config`: Corpus configuration (includes path, chunking settings, graph edge types).
    /// - `index_dir`: Path to the `.index/` directory. Will be created if it doesn't exist.
    ///
    /// Initializes SQLite store, Tantivy BM25 index, knowledge graph, and vector index (if in Full mode).
    /// The embedder is initialized lazily on first use via `ensure_embedder()`.
    ///
    /// This is a thin compatibility entry point that delegates to
    /// [`crate::engine_builder::EngineBuilder::open`], which owns the adapter
    /// construction. The construction sequence lives in exactly one place there;
    /// this method retains no inline construction.
    pub fn open(config: CorpusConfig, index_dir: &Path) -> Result<Self> {
        crate::engine_builder::EngineBuilder::open(config, index_dir)
    }

    /// Ensure the embedder is initialized. Returns Ok(true) if available, Ok(false) if skipped.
    ///
    /// In Fast Mode, this immediately returns Ok(false) without loading the model.
    /// In Full Mode, the embedder is lazily created to avoid model download during tests or when
    /// vector indexing is not needed.
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

    /// Staged file indexing: parses, chunks, updates persistence, BM25, vector removal,
    /// and graph edges without immediately triggering embedding inference.
    ///
    /// Returns pending chunks ready for batched embedding, along with the parsed markdown
    /// document (if markdown) for constructing global tag edges without re-parsing.
    pub fn index_file_staged(
        &mut self,
        rel_path: &str,
        content: &str,
    ) -> Result<(Vec<PendingChunk>, Option<Document>)> {
        let path = Path::new(rel_path);
        let modified_at = now_unix();

        if crate::parser::code::is_code_file(path) {
            let parse_res = crate::parser::code::chunker::CodeChunker::parse_and_chunk(
                path,
                content,
                &self.config.chunking,
            );

            let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
            let file_title =
                path.file_name().and_then(|n| n.to_str()).unwrap_or(rel_path).to_string();

            // 1. Store file record in persistence
            self.store.insert_file(
                rel_path,
                &content_hash,
                modified_at,
                None,
                Some(&file_title),
                FileFormat::Source,
            )?;

            let pending = Vec::new();

            // 2. Chunks and symbols
            if let Some(res) = parse_res {
                self.store.delete_chunks_for_file(rel_path)?;
                let chunk_records: Vec<ChunkRecord> = res
                    .chunks
                    .iter()
                    .map(|c| ChunkRecord {
                        chunk_index: c.chunk_index,
                        start_byte: c.start_byte,
                        end_byte: c.end_byte,
                        start_line: c.start_line,
                        end_line: c.end_line,
                    })
                    .collect();
                self.store.insert_chunks(rel_path, &chunk_records)?;
                self.store.save_code_symbols(rel_path, &res.symbols)?;

                // 3. BM25
                self.bm25.remove_document(rel_path)?;
                self.bm25.add_document(rel_path, Some(&file_title), &[], &res.chunks)?;

                // 3b. Binary Fingerprints (Pillars 2 & 3)
                {
                    use ctxvault_common::types::FingerprintRecord;
                    let mut fps = Vec::new();
                    // File-level fingerprint for code
                    let file_fp = self.binary_index.project_query(content).unwrap_or_default();
                    fps.push(FingerprintRecord {
                        id: rel_path.to_string(),
                        fingerprint: file_fp,
                        modality: Modality::Code,
                    });
                    for sym in &res.symbols {
                        let fp_text = format!(
                            "{} {} {}",
                            sym.name,
                            sym.signature,
                            sym.docstring.as_deref().unwrap_or("")
                        );
                        let fp = self.binary_index.project_query(&fp_text).unwrap_or_default();
                        fps.push(FingerprintRecord {
                            id: format!("{}#{}", rel_path, sym.scope_path),
                            fingerprint: fp,
                            modality: Modality::Code,
                        });
                    }
                    for chunk in &res.chunks {
                        let fp = self.binary_index.project_query(&chunk.text).unwrap_or_default();
                        fps.push(FingerprintRecord {
                            id: format!("{rel_path}:chunk:{}", chunk.chunk_index),
                            fingerprint: fp,
                            modality: Modality::Code,
                        });
                    }
                    if !fps.is_empty() {
                        let _ = self.binary_index.index_fingerprints(&fps);
                    }
                }

                // 4. Vector index: clear existing vectors for this code file (code is not dense embedded).
                if let Some(ref mut vi) = self.vector_index {
                    vi.remove_document(rel_path);
                }
                // Code modality is served by Binary Hamming + AST Graph + BM25 without dense vectors.

                // 5. Code Graph
                self.graph.remove_edges_for_node(rel_path);
                let symbol_index =
                    crate::graph::code::CodeGraphExtractor::build_symbol_index(&res.symbols);
                let extraction =
                    crate::graph::code::CodeGraphExtractor::extract_edges_for_file_with_index(
                        path,
                        content,
                        &res.symbols,
                        &symbol_index,
                    );
                for edge in &extraction.edges {
                    self.graph.add_code_edge(edge);
                }

                // 6. Persist unresolved external references, replacing any prior
                //    set so re-indexing this file stays idempotent (mirrors the
                //    graph edge clear above).
                self.store.clear_external_refs_for_file(rel_path)?;
                if !extraction.external_refs.is_empty() {
                    self.store.insert_external_refs(rel_path, &extraction.external_refs)?;
                }
            }

            debug!("Staged code file: {}", rel_path);
            return Ok((pending, None));
        }

        // 1. Parse document.
        let doc = parser::parse_document(Path::new(rel_path), content)?;

        // 2. Chunk document.
        let chunks = chunker::chunk_document(rel_path, &doc.content, &self.config.chunking);

        // 3. Store file record in persistence.
        self.store.insert_file(
            rel_path,
            &doc.content_hash,
            modified_at,
            doc.template.as_deref(),
            doc.title.as_deref(),
            FileFormat::Source,
        )?;

        // 4. Delete old chunks and insert new ones.
        self.store.delete_chunks_for_file(rel_path)?;
        let chunk_records: Vec<ChunkRecord> = chunks
            .iter()
            .map(|c| ChunkRecord {
                chunk_index: c.chunk_index,
                start_byte: c.start_byte,
                end_byte: c.end_byte,
                start_line: c.start_line,
                end_line: c.end_line,
            })
            .collect();
        self.store.insert_chunks(rel_path, &chunk_records)?;

        // 5. Remove old document from BM25, add new.
        self.bm25.remove_document(rel_path)?;
        self.bm25.add_document(rel_path, doc.title.as_deref(), &doc.tags, &chunks)?;

        // 5b. Binary Fingerprints (Pillars 2 & 3)
        {
            use ctxvault_common::types::FingerprintRecord;
            let mut fps = Vec::new();
            for chunk in &chunks {
                let fp = self.binary_index.project_query(&chunk.text).unwrap_or_default();
                fps.push(FingerprintRecord {
                    id: if chunk.chunk_index == 0 {
                        rel_path.to_string()
                    } else {
                        format!("{rel_path}:chunk:{}", chunk.chunk_index)
                    },
                    fingerprint: fp,
                    modality: Modality::Docs,
                });
            }
            if !fps.is_empty() {
                let _ = self.binary_index.index_fingerprints(&fps);
            }
        }

        // 6. Vector index: clear existing vectors for this doc
        if let Some(ref mut vi) = self.vector_index {
            vi.remove_document(rel_path);
        }

        // Build context-prefixed text for embedding in Full mode (skipped in Fast mode).
        let doc_title = doc.title.as_deref().unwrap_or("").trim();
        let pending: Vec<PendingChunk> =
            if self.config.index_mode == ctxvault_common::config::IndexMode::Full {
                chunks
                    .iter()
                    .map(|c| {
                        let section = c.heading_chain.as_deref().unwrap_or("").trim();
                        let text = if !doc_title.is_empty() && !section.is_empty() {
                            format!("{} > {}: {}", doc_title, section, c.text)
                        } else if !doc_title.is_empty() {
                            format!("{}: {}", doc_title, c.text)
                        } else if !section.is_empty() {
                            format!("{}: {}", section, c.text)
                        } else {
                            c.text.clone()
                        };
                        let modality = c
                            .entity_kind
                            .as_ref()
                            .map(EntityKind::modality_tag)
                            .unwrap_or("docs")
                            .to_string();
                        PendingChunk {
                            doc_path: rel_path.to_string(),
                            chunk_index: c.chunk_index,
                            text,
                            embed_policy: c.embed_policy,
                            modality,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };

        // 7. Remove old edges and rebuild from document.
        self.graph.remove_edges_for_node(rel_path);
        let edge_configs = self.effective_edge_configs_for_document(&doc, None);
        self.graph.build_edges_for_document(&doc, &edge_configs, &[]);

        debug!("Staged markdown file: {}", rel_path);
        Ok((pending, Some(doc)))
    }

    /// Flush a batch of pending chunks into the vector index in a single vectorized forward pass.
    /// Only anchor chunks receive dense vector embeddings; graph-only chunks are skipped.
    pub fn flush_chunk_buffer(&mut self, buffer: &[PendingChunk]) -> Result<()> {
        if buffer.is_empty() {
            return Ok(());
        }

        // Partition buffer into anchor chunks and graph-only chunks.
        // In Full mode, only docs chunks with Anchor embed policy are embedded.
        let anchor_chunks: Vec<&PendingChunk> = buffer
            .iter()
            .filter(|c| c.embed_policy == ChunkEmbedPolicy::Anchor && c.modality != "code")
            .collect();

        tracing::debug!(
            total = buffer.len(),
            anchors = anchor_chunks.len(),
            graph_only = buffer.len() - anchor_chunks.len(),
            "flush_chunk_buffer: partitioned by embed policy"
        );

        if anchor_chunks.is_empty() {
            return Ok(());
        }

        let embedder = match self.embedder_arc() {
            Some(emb) => emb,
            None => return Ok(()),
        };

        let texts: Vec<&str> = anchor_chunks.iter().map(|c| c.text.as_str()).collect();
        let embeddings = match embedder.embed_batch(&texts) {
            Ok(embs) => embs,
            Err(e) => {
                warn!(
                    "Failed to generate embeddings for batch of {} anchor chunks: {}",
                    anchor_chunks.len(),
                    e
                );
                return Ok(());
            }
        };

        if embeddings.len() != anchor_chunks.len() {
            warn!(
                "Embedding count mismatch: expected {}, got {}",
                anchor_chunks.len(),
                embeddings.len()
            );
            return Ok(());
        }

        // Group by contiguous document slices (zero allocation, zero hash map overhead)
        let mut start = 0;
        while start < anchor_chunks.len() {
            let doc_path = &anchor_chunks[start].doc_path;
            let mut end = start + 1;
            while end < anchor_chunks.len() && anchor_chunks[end].doc_path == *doc_path {
                end += 1;
            }

            let file_chunks = &anchor_chunks[start..end];
            let file_embeddings = &embeddings[start..end];

            let chunk_indices: Vec<Option<usize>> =
                file_chunks.iter().map(|c| Some(c.chunk_index)).collect();
            // All chunks for a doc_path share the same file, hence the same modality.
            let modality = file_chunks[0].modality.as_str();

            if let Some(ref mut vi) = self.vector_index {
                let _ = vi.add_batch(file_embeddings, doc_path, &chunk_indices, false, modality);

                if let Some(doc_embedding) = Embedder::average_embeddings(file_embeddings) {
                    let _ = vi.add(&doc_embedding, doc_path, None, true, modality);
                }
            }

            start = end;
        }

        Ok(())
    }

    /// Index a single file. Parses, chunks, stores metadata, indexes in Tantivy,
    /// embeds in vector index (if embedder available), and builds graph edges.
    pub fn index_file(&mut self, rel_path: &str, content: &str) -> Result<()> {
        let (pending, _doc) = self.index_file_staged(rel_path, content)?;
        if !pending.is_empty() {
            self.flush_chunk_buffer(&pending)?;
        }
        Ok(())
    }

    /// Remove a file from all indices (persistence, BM25, vector, graph).
    pub fn remove_file(&mut self, rel_path: &str) -> Result<()> {
        let proj_path = self.projection_path(rel_path);
        if proj_path.is_file() {
            let _ = fs::remove_file(proj_path);
        }

        // 1. Delete from persistence (cascades chunks).
        self.store.delete_file(rel_path)?;

        // 2. Remove from BM25.
        self.bm25.remove_document(rel_path)?;

        // 3. Remove from vector index.
        if let Some(ref mut vi) = self.vector_index {
            vi.remove_document(rel_path);
        }

        // 4. Remove edges from graph.
        self.graph.remove_edges_for_node(rel_path);

        // 5. Remove node from graph (ignore error if node doesn't exist).
        let _ = self.graph.remove_node(rel_path);

        debug!("Removed file: {}", rel_path);
        Ok(())
    }

    /// Load all markdown templates defined in the corpus templates directory,
    /// using either explicitly configured templates_dir or auto-discovery fallbacks.
    pub fn load_templates(&self) -> Result<HashMap<String, Template>> {
        let corpus_path = Path::new(&self.config.path);
        let (_resolved, templates) =
            Template::discover_and_load(corpus_path, self.config.templates_dir.as_deref())?;
        Ok(templates)
    }

    /// Discover and load all markdown templates along with the resolved relative directory.
    pub fn discover_templates(
        &self,
    ) -> Result<(Option<std::path::PathBuf>, HashMap<String, Template>)> {
        let corpus_path = Path::new(&self.config.path);
        Template::discover_and_load(corpus_path, self.config.templates_dir.as_deref())
    }

    /// Compute the effective edge type configurations for a document, combining
    /// corpus-level edge configs with any edge declarations in the document's template.
    pub fn effective_edge_configs_for_document(
        &self,
        doc: &Document,
        templates: Option<&HashMap<String, Template>>,
    ) -> Vec<ctxvault_common::config::EdgeTypeConfig> {
        let mut configs = self.config.graph.edge_types.clone();
        if let Some(ref tmpl_name) = doc.template {
            let loaded = if templates.is_none() { self.load_templates().ok() } else { None };
            let tmpl_map = templates.or(loaded.as_ref());
            if let Some(tmpl) = tmpl_map.and_then(|m| m.get(tmpl_name)) {
                for edge in &tmpl.edges {
                    configs.push(edge.to_edge_type_config());
                }
            }
        }
        configs
    }

    /// Perform a delta scan with default batch size.
    pub fn delta_scan(&mut self) -> Result<DeltaScanResult> {
        self.delta_scan_paginated(50)
    }

    /// Ingest a single parsed file record into SQLite persistence, BM25, graph, and vector index.
    fn ingest_parsed_record(
        &mut self,
        record: ParsedFileRecord,
        tag_configs: &[ctxvault_common::config::EdgeTypeConfig],
        all_docs: &mut Vec<Document>,
    ) -> Result<()> {
        let path = &record.path;
        let modified_at = now_unix();

        if let Some(ref text) = record.projection_text {
            let proj_path = self.projection_path(path);
            if let Some(parent) = proj_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::write(&proj_path, text.as_bytes()) {
                warn!("Failed to write projection for {path}: {e}");
            }
        }

        if record.is_code {
            let file_title =
                Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path).to_string();

            // 1. SQLite Store
            self.store.insert_file(
                path,
                &record.hash,
                modified_at,
                None,
                Some(&file_title),
                record.format,
            )?;

            // 2. Chunks and symbols
            self.store.delete_chunks_for_file(path)?;
            let chunk_records: Vec<ChunkRecord> = record
                .raw_chunks
                .iter()
                .map(|c| ChunkRecord {
                    chunk_index: c.chunk_index,
                    start_byte: c.start_byte,
                    end_byte: c.end_byte,
                    start_line: c.start_line,
                    end_line: c.end_line,
                })
                .collect();
            self.store.insert_chunks(path, &chunk_records)?;
            self.store.save_code_symbols(path, &record.symbols)?;

            // 3. BM25
            self.bm25.remove_document(path)?;
            self.bm25.add_document(path, Some(&file_title), &[], &record.raw_chunks)?;

            // 3b. Binary Fingerprints (Pillars 2 & 3)
            if !record.fingerprints.is_empty() {
                let _ = self.binary_index.index_fingerprints(&record.fingerprints);
            }

            // 4. Vector index: clear existing vectors for this doc
            if let Some(ref mut vi) = self.vector_index {
                vi.remove_document(path);
            }

            // 5. Code Graph
            self.graph.remove_edges_for_node(path);
            for edge in &record.graph_edges {
                self.graph.add_code_edge(edge);
            }

            // 6. Unresolved external references (replace prior set for idempotency).
            self.store.clear_external_refs_for_file(path)?;
            if !record.external_refs.is_empty() {
                self.store.insert_external_refs(path, &record.external_refs)?;
            }
        } else if let Some(mut doc) = record.doc_metadata {
            let file_title = doc.title.clone().unwrap_or_else(|| {
                Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path).to_string()
            });

            // 1. SQLite Store
            self.store.insert_file(
                path,
                &record.hash,
                modified_at,
                doc.template.as_deref(),
                doc.title.as_deref().or(Some(&file_title)),
                record.format,
            )?;

            // 2. Chunks
            self.store.delete_chunks_for_file(path)?;
            let chunk_records: Vec<ChunkRecord> = record
                .raw_chunks
                .iter()
                .map(|c| ChunkRecord {
                    chunk_index: c.chunk_index,
                    start_byte: c.start_byte,
                    end_byte: c.end_byte,
                    start_line: c.start_line,
                    end_line: c.end_line,
                })
                .collect();
            self.store.insert_chunks(path, &chunk_records)?;

            // 3. BM25
            self.bm25.remove_document(path)?;
            self.bm25.add_document(path, doc.title.as_deref(), &doc.tags, &record.raw_chunks)?;

            // 3b. Binary Fingerprints (Pillars 2 & 3)
            if !record.fingerprints.is_empty() {
                let _ = self.binary_index.index_fingerprints(&record.fingerprints);
            }

            // 4. Vector index: clear existing vectors for this doc
            if let Some(ref mut vi) = self.vector_index {
                vi.remove_document(path);
            }

            // 5. Graph
            self.graph.remove_edges_for_node(path);
            let edge_configs = self.effective_edge_configs_for_document(&doc, None);
            self.graph.build_edges_for_document(&doc, &edge_configs, &[]);
            for edge in &record.graph_edges {
                self.graph.add_edge(
                    &edge.source,
                    &edge.target,
                    &edge.edge_type,
                    edge.weight,
                    edge.provenance.clone(),
                    ctxvault_common::config::EdgeClass::Structural,
                );
            }

            if !tag_configs.is_empty() && !doc.tags.is_empty() {
                doc.content.clear();
                doc.wikilinks.clear();
                all_docs.push(doc);
            }
        }

        Ok(())
    }

    /// Helper to read file bytes and decode as UTF-8 lossily so non-UTF8 characters never throw.
    fn read_file_lossy(path: &Path) -> std::io::Result<String> {
        let bytes = fs::read(path)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Perform a paginated delta scan: compare filesystem against stored file records.
    ///
    /// Automatically re-indexes changed files and removes deleted ones with intermediate commits.
    /// Returns a summary of what changed.
    pub fn delta_scan_paginated(&mut self, batch_size: usize) -> Result<DeltaScanResult> {
        let commit_batch_size = if batch_size == 0 || batch_size == 50 { 500 } else { batch_size };
        if self.config.index_mode != ctxvault_common::config::IndexMode::Fast {
            let _ = self.ensure_embedder();
        }

        // 1. List all files currently in persistence.
        let stored_files = self.store.list_files()?;
        let stored_map: HashMap<String, String> =
            stored_files.into_iter().map(|f| (f.path.clone(), f.content_hash.clone())).collect();

        // 2. Walk the corpus directory.
        let corpus_path = PathBuf::from(&self.config.path);
        let disk_files =
            walk_markdown_files(&corpus_path, &self.exclude_matcher, &self.classifier)?;

        let mut new_files = Vec::new();
        let mut modified_files = Vec::new();
        let mut seen_on_disk = HashMap::new();
        let mut files_to_index = Vec::new();

        for (rel_path, full_path) in &disk_files {
            let _ = seen_on_disk.insert(rel_path.clone(), ());
            let bytes = match fs::read(full_path) {
                Ok(b) => b,
                Err(e) => {
                    warn!("{}: {}", rel_path, e);
                    continue;
                }
            };
            let hash = blake3::hash(&bytes).to_hex().to_string();

            match stored_map.get(rel_path) {
                None => {
                    new_files.push(rel_path.clone());
                    files_to_index.push((rel_path.clone(), full_path.clone()));
                }
                Some(stored_hash) if *stored_hash != hash => {
                    modified_files.push(rel_path.clone());
                    files_to_index.push((rel_path.clone(), full_path.clone()));
                }
                _ => {
                    // Unchanged
                }
            }
        }

        // 3. Find deleted files (in store but not on disk).
        let mut deleted_files = Vec::new();
        for path in stored_map.keys() {
            if !seen_on_disk.contains_key(path) {
                self.remove_file(path)?;
                deleted_files.push(path.clone());
            }
        }

        self.ensure_vector_index();
        let embedding_pipeline =
            if self.config.index_mode == ctxvault_common::config::IndexMode::Fast {
                None
            } else {
                self.embedder_arc().map(AsyncEmbeddingPipeline::new)
            };

        let tag_configs: Vec<_> = self
            .config
            .graph
            .edge_types
            .iter()
            .filter(|et| et.source == ctxvault_common::config::EdgeSource::Tag)
            .cloned()
            .collect();
        let mut all_docs: Vec<Document> = Vec::new();

        if !files_to_index.is_empty() {
            let num_cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
            let (work_tx, work_rx) = crossbeam_channel::unbounded::<(String, PathBuf)>();
            let (ast_tx, ast_rx) = crossbeam_channel::bounded::<ParsedFileRecord>(2048);

            let chunk_tx_opt = embedding_pipeline.as_ref().and_then(|p| p.chunk_sender());
            let chunking_config = self.config.chunking.clone();
            let index_mode = self.config.index_mode;
            let sif_engine = self.binary_index.sif();

            for file_entry in files_to_index {
                let _ = work_tx.send(file_entry);
            }
            drop(work_tx);

            let mut uncommitted_count = 0usize;
            let mut last_commit_time = Instant::now();
            let commit_time_threshold = Duration::from_secs(30);

            std::thread::scope(|s| {
                for i in 0..num_cpus {
                    let work_rx_clone = work_rx.clone();
                    let ast_tx_clone = ast_tx.clone();
                    let chunk_tx_clone = chunk_tx_opt.clone();
                    let chunking_ref = &chunking_config;
                    let classifier_clone = self.classifier.clone();
                    let sif_clone = sif_engine.clone();

                    std::thread::Builder::new()
                        .name(format!("indexer-worker-{}", i))
                        .stack_size(16 * 1024 * 1024)
                        .spawn_scoped(s, move || {
                            while let Ok((rel_path, full_path)) = work_rx_clone.recv() {
                                let bytes = match fs::read(&full_path) {
                                    Ok(b) => b,
                                    Err(e) => {
                                        warn!("Failed to read {}: {}", rel_path, e);
                                        continue;
                                    }
                                };
                                let hash = blake3::hash(&bytes).to_hex().to_string();

                                let record = match parse_file_record(
                                    &rel_path,
                                    &full_path,
                                    &bytes,
                                    hash,
                                    &classifier_clone,
                                    chunking_ref,
                                    index_mode,
                                    &sif_clone,
                                ) {
                                    Ok(r) => r,
                                    Err(e) => {
                                        warn!("Failed to parse {}: {}", rel_path, e);
                                        continue;
                                    }
                                };

                                if let Some(ref tx) = chunk_tx_clone {
                                    for chunk in &record.chunks {
                                        if chunk.embed_policy == ChunkEmbedPolicy::Anchor {
                                            if tx.send(chunk.clone()).is_err() {
                                                break;
                                            }
                                        }
                                    }
                                }

                                if ast_tx_clone.send(record).is_err() {
                                    break;
                                }
                            }
                        })
                        .expect("failed to spawn indexer worker thread");
                }

                // Drop our local handles so channels disconnect when workers finish
                drop(chunk_tx_opt);
                drop(ast_tx);

                let _ = self.store.begin_batch();

                while let Ok(record) = ast_rx.recv() {
                    let path = record.path.clone();
                    if let Err(e) = self.ingest_parsed_record(record, &tag_configs, &mut all_docs) {
                        warn!("Failed to ingest {}: {}", path, e);
                        continue;
                    }

                    if let Some(ref pipeline) = embedding_pipeline {
                        if let Some(ref mut vi) = self.vector_index {
                            let _ = pipeline.try_recv_completed(vi);
                        }
                    }

                    uncommitted_count += 1;
                    if uncommitted_count >= commit_batch_size
                        || last_commit_time.elapsed() >= commit_time_threshold
                    {
                        if let Some(ref pipeline) = embedding_pipeline {
                            if let Some(ref mut vi) = self.vector_index {
                                let _ = pipeline.try_recv_completed(vi);
                            }
                        }
                        let _ = self.store.commit_batch();
                        if let Err(e) = self.commit_intermediate() {
                            warn!("Intermediate commit failed: {}", e);
                        }
                        let _ = self.store.begin_batch();
                        uncommitted_count = 0;
                        last_commit_time = Instant::now();
                    }
                }

                let _ = self.store.commit_batch();
            });
        }

        // Finish embedding pipeline and commit remaining changes.
        if let Some(mut pipeline) = embedding_pipeline {
            if let Some(ref mut vi) = self.vector_index {
                pipeline.finish(vi)?;
            }
        }
        if !new_files.is_empty() || !modified_files.is_empty() {
            let _ = self.resolve_cross_file_code_edges();
        }
        self.commit()?;
        let _ = self.store.checkpoint();

        info!(
            "Delta scan complete: {} new, {} modified, {} deleted",
            new_files.len(),
            modified_files.len(),
            deleted_files.len()
        );

        Ok(DeltaScanResult { new_files, modified_files, deleted_files })
    }

    /// Incrementally synchronize a specific list of changed or deleted paths.
    ///
    /// Ideal for continuous file watchers (`notify`) where specific file events are known,
    /// avoiding full directory tree traversal.
    pub fn sync_delta_paths(&mut self, paths: &[PathBuf]) -> Result<DeltaScanResult> {
        let corpus_path = PathBuf::from(&self.config.path);
        let mut new_files = Vec::new();
        let mut modified_files = Vec::new();
        let mut deleted_files = Vec::new();

        self.ensure_vector_index();
        if self.config.index_mode != ctxvault_common::config::IndexMode::Fast {
            let _ = self.ensure_embedder();
        }
        let embedding_pipeline =
            if self.config.index_mode == ctxvault_common::config::IndexMode::Fast {
                None
            } else {
                self.embedder_arc().map(AsyncEmbeddingPipeline::new)
            };
        let sif_engine = self.binary_index.sif();

        let tag_configs: Vec<_> = self
            .config
            .graph
            .edge_types
            .iter()
            .filter(|et| et.source == ctxvault_common::config::EdgeSource::Tag)
            .cloned()
            .collect();
        let mut all_docs: Vec<Document> = Vec::new();
        let classifier = crate::index::classifier::FileClassifier::new(&corpus_path, &self.config);

        for path in paths {
            // Determine relative path within corpus
            let rel_path = if path.is_absolute() {
                match path.strip_prefix(&corpus_path) {
                    Ok(p) => p.to_string_lossy().replace('\\', "/"),
                    Err(_) => path.to_string_lossy().replace('\\', "/"),
                }
            } else {
                path.to_string_lossy().replace('\\', "/")
            };

            let full_path = if path.is_absolute() { path.clone() } else { corpus_path.join(path) };

            if !full_path.exists() {
                // File was deleted
                if self.store.get_file(&rel_path)?.is_some() {
                    self.remove_file(&rel_path)?;
                    deleted_files.push(rel_path);
                }
            } else {
                if self.exclude_matcher.is_excluded(&full_path, full_path.is_dir()) {
                    continue;
                }
                // File exists: check if new or modified
                let bytes = match fs::read(&full_path) {
                    Ok(b) => b,
                    Err(e) => {
                        warn!("Failed to read {}: {}", rel_path, e);
                        continue;
                    }
                };
                let hash = blake3::hash(&bytes).to_hex().to_string();
                let stored_file = self.store.get_file(&rel_path)?;

                let is_new = stored_file.is_none();
                let is_modified = stored_file.as_ref().map_or(false, |f| f.content_hash != hash);

                if is_new || is_modified {
                    let record = match parse_file_record(
                        &rel_path,
                        &full_path,
                        &bytes,
                        hash,
                        &classifier,
                        &self.config.chunking,
                        self.config.index_mode,
                        &sif_engine,
                    ) {
                        Ok(r) => r,
                        Err(e) => {
                            warn!("Failed to parse {}: {}", rel_path, e);
                            continue;
                        }
                    };

                    if let Some(ref pipeline) = embedding_pipeline {
                        if let Some(tx) = pipeline.chunk_sender() {
                            for chunk in &record.chunks {
                                if chunk.embed_policy == ChunkEmbedPolicy::Anchor {
                                    let _ = tx.send(chunk.clone());
                                }
                            }
                        }
                    }

                    if let Err(e) = self.ingest_parsed_record(record, &tag_configs, &mut all_docs) {
                        warn!("Failed to ingest {}: {}", rel_path, e);
                        continue;
                    }

                    if let Some(ref pipeline) = embedding_pipeline {
                        if let Some(ref mut vi) = self.vector_index {
                            let _ = pipeline.try_recv_completed(vi);
                        }
                    }

                    if is_new {
                        new_files.push(rel_path);
                    } else {
                        modified_files.push(rel_path);
                    }
                }
            }
        }

        if let Some(mut pipeline) = embedding_pipeline {
            if let Some(ref mut vi) = self.vector_index {
                pipeline.finish(vi)?;
            }
        }

        if !new_files.is_empty() || !modified_files.is_empty() {
            let _ = self.resolve_cross_file_code_edges();
        }

        self.commit()?;
        let _ = self.store.checkpoint();

        info!(
            "sync_delta_paths complete: {} new, {} modified, {} deleted",
            new_files.len(),
            modified_files.len(),
            deleted_files.len()
        );

        Ok(DeltaScanResult { new_files, modified_files, deleted_files })
    }

    /// Full reindex with default parameters (batch_size=50, resume=false).
    ///
    /// Returns the number of files indexed.
    pub fn full_reindex(&mut self) -> Result<usize> {
        self.full_reindex_paginated(50, false)
    }

    /// Paginated, resumable full reindex: scans corpus directory in configurable batches.
    ///
    /// - `batch_size`: Number of documents processed before flushing/checkpointing (default 500).
    /// - `resume`: If true, skips files already committed with identical content hash.
    ///
    /// Performs intermediate commits of SQLite, Tantivy, Vectors, Graph, and updates `indexing_state`.
    pub fn full_reindex_paginated(&mut self, batch_size: usize, resume: bool) -> Result<usize> {
        let commit_batch_size = if batch_size == 0 || batch_size == 50 { 500 } else { batch_size };
        let corpus_id = self.config.name.clone();
        let corpus_path = PathBuf::from(&self.config.path);
        let mut disk_files =
            walk_markdown_files(&corpus_path, &self.exclude_matcher, &self.classifier)?;
        disk_files.sort_by(|a, b| a.0.cmp(&b.0));
        let total_files = disk_files.len();

        // Ensure embedder and vector index are available for indexing.
        self.ensure_vector_index();
        if self.config.index_mode != ctxvault_common::config::IndexMode::Fast {
            let _ = self.ensure_embedder();
        }

        let mut stored_map: HashMap<String, String> = HashMap::new();

        if !resume {
            // Fresh rebuild: clear store, Tantivy, graph, vector index, and reset state
            let existing = self.store.list_files()?;
            for file in &existing {
                self.store.delete_file(&file.path)?;
                self.bm25.remove_document(&file.path)?;
            }
            self.graph = KnowledgeGraph::new();
            if let Some(ref mut vi) = self.vector_index {
                *vi = VectorIndex::new_default(vi.dimensions());
            }
            self.store.reset_indexing_state(&corpus_id)?;
        } else {
            // Resuming: load existing indexed files and their content hashes
            let existing = self.store.list_files()?;
            for file in existing {
                let _ = stored_map.insert(file.path, file.content_hash);
            }
        }

        let started_at = now_unix();
        let mut state = IndexingState {
            corpus_id: corpus_id.clone(),
            status: IndexingStatus::Indexing,
            total_files,
            indexed_files: 0,
            last_processed_path: None,
            started_at,
            updated_at: started_at,
            error_message: None,
        };

        // If resuming, calculate already-indexed matching files
        if resume {
            let mut matched_count = 0usize;
            for (rel_path, full_path) in &disk_files {
                if let Some(stored_hash) = stored_map.get(rel_path) {
                    if let Ok(content) = fs::read_to_string(full_path) {
                        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
                        if hash == *stored_hash {
                            matched_count += 1;
                        }
                    }
                }
            }
            state.indexed_files = matched_count;
        }

        self.store.update_indexing_state(&state)?;

        let tag_configs: Vec<_> = self
            .config
            .graph
            .edge_types
            .iter()
            .filter(|et| et.source == ctxvault_common::config::EdgeSource::Tag)
            .cloned()
            .collect();
        let mut all_docs: Vec<Document> = Vec::new();
        let embedding_pipeline =
            if self.config.index_mode == ctxvault_common::config::IndexMode::Fast {
                None
            } else {
                self.embedder_arc().map(AsyncEmbeddingPipeline::new)
            };
        let sif_engine = self.binary_index.sif();

        // Stage A: Setup parallel parsing channels and worker pool
        let num_cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
        let (work_tx, work_rx) = crossbeam_channel::unbounded::<(String, PathBuf)>();
        let (ast_tx, ast_rx) = crossbeam_channel::bounded::<ParsedFileRecord>(2048);

        let chunk_tx_opt = embedding_pipeline.as_ref().and_then(|p| p.chunk_sender());
        let chunking_config = self.config.chunking.clone();
        let index_mode = self.config.index_mode;

        // Populate work queue
        for file_entry in disk_files {
            let _ = work_tx.send(file_entry);
        }
        drop(work_tx); // Close producer end of work channel so workers drain and finish

        let mut uncommitted_count = 0usize;
        let mut last_commit_time = Instant::now();
        let commit_time_threshold = Duration::from_secs(30);

        std::thread::scope(|s| {
            // 1. Spawn Stage A Worker Threads
            for i in 0..num_cpus {
                let work_rx_clone = work_rx.clone();
                let ast_tx_clone = ast_tx.clone();
                let chunk_tx_clone = chunk_tx_opt.clone();
                let stored_map_ref = &stored_map;
                let chunking_ref = &chunking_config;
                let classifier_clone = self.classifier.clone();
                let sif_clone = sif_engine.clone();

                std::thread::Builder::new()
                    .name(format!("indexer-worker-{}", i))
                    .stack_size(16 * 1024 * 1024)
                    .spawn_scoped(s, move || {
                        while let Ok((rel_path, full_path)) = work_rx_clone.recv() {
                            let bytes = match fs::read(&full_path) {
                                Ok(b) => b,
                                Err(e) => {
                                    warn!("Failed to read {}: {}", rel_path, e);
                                    continue;
                                }
                            };
                            let hash = blake3::hash(&bytes).to_hex().to_string();

                            if resume {
                                if let Some(stored_hash) = stored_map_ref.get(&rel_path) {
                                    if *stored_hash == hash {
                                        continue;
                                    }
                                }
                            }

                            let record = match parse_file_record(
                                &rel_path,
                                &full_path,
                                &bytes,
                                hash,
                                &classifier_clone,
                                chunking_ref,
                                index_mode,
                                &sif_clone,
                            ) {
                                Ok(r) => r,
                                Err(e) => {
                                    warn!("Failed to parse {}: {}", rel_path, e);
                                    continue;
                                }
                            };

                            // Stream anchor chunks to GPU prefetch worker (Stage B)
                            if let Some(ref tx) = chunk_tx_clone {
                                for chunk in &record.chunks {
                                    if chunk.embed_policy == ChunkEmbedPolicy::Anchor {
                                        if tx.send(chunk.clone()).is_err() {
                                            break;
                                        }
                                    }
                                }
                            }

                            // Emit parsed record to Stage C storage sink
                            if ast_tx_clone.send(record).is_err() {
                                break;
                            }
                        }
                    })
                    .expect("failed to spawn indexer worker thread");
            }

            // Drop our local handles so channels disconnect when workers finish
            drop(chunk_tx_opt);
            drop(ast_tx);

            let _ = self.store.begin_batch();

            // 2. Main Thread acts as Dedicated Stage C Storage & Persistence Sink
            while let Ok(record) = ast_rx.recv() {
                let path = record.path.clone();
                if let Err(e) = self.ingest_parsed_record(record, &tag_configs, &mut all_docs) {
                    warn!("Failed to ingest {}: {}", path, e);
                    continue;
                }

                if let Some(ref pipeline) = embedding_pipeline {
                    if let Some(ref mut vi) = self.vector_index {
                        let _ = pipeline.try_recv_completed(vi);
                    }
                }

                uncommitted_count += 1;
                state.indexed_files += 1;
                state.last_processed_path = Some(path);

                if uncommitted_count >= commit_batch_size
                    || last_commit_time.elapsed() >= commit_time_threshold
                {
                    if let Some(ref pipeline) = embedding_pipeline {
                        if let Some(ref mut vi) = self.vector_index {
                            let _ = pipeline.try_recv_completed(vi);
                        }
                    }
                    let _ = self.store.commit_batch();
                    if let Err(e) = self.commit_intermediate() {
                        warn!("Intermediate commit failed: {}", e);
                    }
                    let _ = self.store.begin_batch();
                    state.updated_at = now_unix();
                    let _ = self.store.update_indexing_state(&state);
                    debug!(
                        "Committed batch of {} files ({}/{} total, elapsed {:.1}s)",
                        uncommitted_count,
                        state.indexed_files,
                        total_files,
                        last_commit_time.elapsed().as_secs_f32()
                    );
                    uncommitted_count = 0;
                    last_commit_time = Instant::now();
                }
            }

            let _ = self.store.commit_batch();
        });

        // 3. Stage B Completion: drain remaining in-flight batches and join GPU threads
        if let Some(mut pipeline) = embedding_pipeline {
            if let Some(ref mut vi) = self.vector_index {
                pipeline.finish(vi)?;
            }
        }

        // Commit any remaining uncommitted files before post-processing passes
        if uncommitted_count > 0 {
            self.commit_intermediate()?;
        }

        // Second pass: build tag-based edges with all documents available
        if !tag_configs.is_empty() && !all_docs.is_empty() {
            self.graph.build_all_tag_edges(&tag_configs, &all_docs);
        }

        // Second pass: resolve cross-file code call graph edges now that all symbols are indexed
        let _ = self.resolve_cross_file_code_edges();

        // Final commit and mark Completed (saves graph.bin and syncs all edges to SQLite once)
        self.commit()?;
        let _ = self.store.checkpoint();
        state.status = IndexingStatus::Completed;
        state.updated_at = now_unix();
        state.indexed_files = total_files;
        self.store.update_indexing_state(&state)?;

        info!("Paginated indexing complete: {} total files indexed/verified", total_files);

        Ok(total_files)
    }

    /// Commit pending lexical updates and checkpoints metadata without
    /// incurring quadratic graph re-serialization and edge table rewrites.
    pub fn commit_intermediate(&mut self) -> Result<()> {
        let _ = self.store.commit_batch();
        self.bm25.commit()?;
        let _ = self.store.checkpoint();
        Ok(())
    }

    /// Commit all pending changes (Tantivy commit, SQLite edges sync, graph save, vector index save).
    pub fn commit(&mut self) -> Result<()> {
        let _ = self.store.commit_batch();
        self.bm25.commit()?;
        let edge_records = self.graph.get_all_edge_records();
        self.store.clear_all_edges()?;
        self.store.insert_edges(&edge_records)?;
        self.graph.save(&self.index_dir.join("graph.bin"))?;
        // Save vector index (only if it has data and has unpersisted changes).
        if let Some(ref vi) = self.vector_index {
            if vi.is_dirty() && !vi.is_empty() {
                vi.save_binary(&self.index_dir.join("vectors.bin")).unwrap_or_else(|e| {
                    warn!("Failed to save vector index: {}", e);
                });
            }
        }
        // Save binary fingerprints index.
        if !self.binary_index.is_empty() {
            let _ = self.binary_index.save_to_path(&self.index_dir.join("fingerprints.bin"));
        }
        Ok(())
    }

    /// Ingest a pre-computed SCIP (Source Code Intelligence Protocol) index into the knowledge graph.
    ///
    /// Reads the SCIP binary protobuf file, extracts high-confidence symbol definitions (`defines`)
    /// and cross-file references/calls (`calls`), stores them in the graph, and commits the graph state.
    pub fn ingest_scip(&mut self, scip_path: &Path) -> Result<crate::graph::scip::ScipIngestStats> {
        info!("Ingesting SCIP index from: {}", scip_path.display());
        let (edges, stats) = crate::graph::scip::ScipIngester::extract_edges_from_file(scip_path)?;

        for edge in &edges {
            self.graph.add_code_edge(edge);
        }

        let edge_records = self.graph.get_all_edge_records();
        self.store.clear_all_edges()?;
        self.store.insert_edges(&edge_records)?;
        self.graph.save(&self.index_dir.join("graph.bin"))?;

        info!(
            "SCIP ingestion complete: {} documents, {} definitions, {} calls, {} edges added",
            stats.documents_processed,
            stats.definitions_extracted,
            stats.calls_extracted,
            stats.edges_added
        );

        Ok(stats)
    }

    /// Second-pass cross-file AST call graph resolution.
    ///
    /// After all files in the corpus are indexed and their symbols stored in SQLite,
    /// this pass walks all code files, resolves cross-file call sites against the complete
    /// corpus symbol index, and inserts the resulting `calls` edges into the knowledge graph.
    pub fn resolve_cross_file_code_edges(&mut self) -> Result<usize> {
        let all_symbols = self.store.get_all_code_symbols()?;
        if all_symbols.is_empty() {
            return Ok(0);
        }

        let symbol_index = crate::graph::code::CodeGraphExtractor::build_symbol_index(&all_symbols);
        let corpus_path = PathBuf::from(&self.config.path);
        let files = self.store.list_files()?;
        let mut edges_added = 0usize;

        for f in &files {
            let rel_p = Path::new(&f.path);
            if !crate::parser::code::is_code_file(rel_p) {
                continue;
            }

            let full_path = corpus_path.join(&f.path);
            let content = match Self::read_file_lossy(&full_path) {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to read code file {}: {}", f.path, e);
                    continue;
                }
            };

            let file_symbols = self.store.get_code_symbols_for_file(&f.path)?;
            let extraction =
                crate::graph::code::CodeGraphExtractor::extract_edges_for_file_with_index(
                    rel_p,
                    &content,
                    &file_symbols,
                    &symbol_index,
                );

            for edge in &extraction.edges {
                self.graph.add_code_edge(edge);
                edges_added += 1;
            }

            // Persist unresolved call/import targets as external references for a
            // later cross-corpus reconciliation pass. Clear-then-insert per file
            // keeps re-indexing idempotent and does not affect intra-repo edges.
            self.store.clear_external_refs_for_file(&f.path)?;
            if !extraction.external_refs.is_empty() {
                self.store.insert_external_refs(&f.path, &extraction.external_refs)?;
            }
        }

        info!("Cross-file code call resolution complete: {} edges added/updated", edges_added);
        Ok(edges_added)
    }

    /// Build a [`CoreSearchService`](crate::search_service::CoreSearchService)
    /// over this engine's resolved retrieval backends.
    ///
    /// The service borrows the engine's BM25 index, optional vector index, and
    /// knowledge graph, and holds an owned `Arc` clone of the current embedder
    /// (if initialized). Consumers dispatch search modes through the
    /// [`SearchService`](ctxvault_common::ports::SearchService) port without
    /// naming any concrete backend type.
    ///
    /// Callers that need semantic search must still call
    /// [`Engine::ensure_embedder`] first (guarded by
    /// [`Engine::has_vector_index`]); this builder does not initialize it.
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

    /// Check whether the engine is running in Fast Mode.
    pub fn is_fast_mode(&self) -> bool {
        self.config.index_mode == ctxvault_common::config::IndexMode::Fast
    }

    /// Access the in-memory binary search index.
    pub fn binary_index(&self) -> &crate::search::binary::BinarySearchIndex {
        &self.binary_index
    }

    /// Access the mutable in-memory binary search index.
    pub fn binary_index_mut(&mut self) -> &mut crate::search::binary::BinarySearchIndex {
        &mut self.binary_index
    }

    /// Update the index mode dynamically, allocating or dropping the vector index as appropriate.
    pub fn set_index_mode(&mut self, mode: ctxvault_common::config::IndexMode) {
        self.config.index_mode = mode;
        match self.config.index_mode {
            ctxvault_common::config::IndexMode::Fast => {
                self.vector_index = None;
            }
            ctxvault_common::config::IndexMode::Full => {
                self.ensure_vector_index();
            }
        }
    }

    /// Ensure the vector index is initialized if running in Full or DocsEmbed mode.
    pub fn ensure_vector_index(&mut self) {
        if self.vector_index.is_none()
            && self.config.index_mode != ctxvault_common::config::IndexMode::Fast
        {
            let configured_model_name =
                crate::embedding::ModelName::from_str_name(&self.config.embedding.model)
                    .unwrap_or_default();
            let configured_dimensions = configured_model_name.dimensions();
            let vector_path = self.index_dir.join("vectors.bin");
            let vi = if vector_path.exists() {
                VectorIndex::load_binary(&vector_path)
                    .unwrap_or_else(|_| VectorIndex::new_default(configured_dimensions))
            } else {
                VectorIndex::new_default(configured_dimensions)
            };
            self.vector_index = Some(vi);
        }
    }

    /// Whether a vector index is present (absent in Fast Mode).
    pub fn has_vector_index(&self) -> bool {
        self.vector_index.is_some()
    }

    /// Whether the embedder has been initialized for this engine.
    pub fn embedder_active(&self) -> bool {
        self.embedder.read().unwrap().is_some()
    }

    /// Internal: current embedder handle (an `Arc` clone) if initialized.
    fn embedder_arc(&self) -> Option<Arc<Embedder>> {
        self.embedder.read().unwrap().clone()
    }

    /// Number of vectors currently in the vector index (0 in Fast Mode).
    pub fn vector_count(&self) -> usize {
        self.vector_index.as_ref().map(|vi| vi.len()).unwrap_or(0)
    }

    /// Return the active hardware acceleration provider (e.g. "DirectML (GPU)", "CPU").
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

    /// Get current indexing progress and throughput statistics.
    pub fn get_indexing_status(&self) -> Result<IndexingStatusResponse> {
        let corpus_id = &self.config.name;
        let stored = self.store.get_indexing_state(corpus_id)?;
        let now = now_unix();

        if let Some(state) = stored {
            let total = state.total_files;
            let indexed = state.indexed_files;
            let progress_percent = if total > 0 {
                ((indexed as f64 / total as f64) * 100.0).min(100.0)
            } else if state.status == IndexingStatus::Completed {
                100.0
            } else {
                0.0
            };

            let elapsed = if state.status == IndexingStatus::Indexing {
                now.saturating_sub(state.started_at)
            } else {
                state.updated_at.saturating_sub(state.started_at)
            };

            let throughput = if elapsed > 0 { indexed as f64 / elapsed as f64 } else { 0.0 };

            let remaining_files = total.saturating_sub(indexed);
            let time_remaining = if throughput > 0.0 && state.status == IndexingStatus::Indexing {
                remaining_files as f64 / throughput
            } else {
                0.0
            };

            Ok(IndexingStatusResponse {
                corpus_id: state.corpus_id,
                status: state.status,
                total_files: total,
                indexed_files: indexed,
                progress_percent: (progress_percent * 100.0).round() / 100.0,
                last_processed_path: state.last_processed_path,
                started_at: state.started_at,
                updated_at: state.updated_at,
                elapsed_seconds: elapsed,
                estimated_throughput_docs_per_sec: (throughput * 100.0).round() / 100.0,
                estimated_time_remaining_seconds: (time_remaining * 100.0).round() / 100.0,
                error_message: state.error_message,
            })
        } else {
            // No indexing state recorded yet: check store files
            let count = self.store.list_files().map(|f| f.len()).unwrap_or(0);
            Ok(IndexingStatusResponse {
                corpus_id: corpus_id.clone(),
                status: if count > 0 { IndexingStatus::Completed } else { IndexingStatus::Idle },
                total_files: count,
                indexed_files: count,
                progress_percent: if count > 0 { 100.0 } else { 0.0 },
                last_processed_path: None,
                started_at: 0,
                updated_at: 0,
                elapsed_seconds: 0,
                estimated_throughput_docs_per_sec: 0.0,
                estimated_time_remaining_seconds: 0.0,
                error_message: None,
            })
        }
    }

    /// Get a port-typed reference to the graph for traversal/queries.
    pub fn graph(&self) -> &impl GraphStore {
        &self.graph
    }

    /// Get a concrete reference to the underlying KnowledgeGraph.
    pub fn knowledge_graph(&self) -> &crate::graph::KnowledgeGraph {
        &self.graph
    }

    /// Get a port-typed mutable reference to the graph for manipulation.
    pub fn graph_mut(&mut self) -> &mut impl GraphStore {
        &mut self.graph
    }

    /// Execute a Cypher-Lite linear path pattern match across code and doc entities.
    pub fn graph_match(
        &self,
        pattern: &str,
        edge_class: Option<&str>,
        where_clause: Option<&str>,
        limit: usize,
        max_depth: usize,
    ) -> Result<ctxvault_common::types::GraphMatchResult> {
        let parsed = crate::graph::query::parse_path_pattern(pattern)?;
        let qe = crate::graph::query::QueryEngine::new(&self.store);
        qe.execute_match(&parsed, edge_class, where_clause, limit, max_depth)
    }

    /// Compute direct degree affordances for a node.
    pub fn compute_affordances(&self, path: &str) -> ctxvault_common::types::GraphAffordances {
        self.graph.compute_affordances(path)
    }

    /// Format immediate 1-hop graph neighborhood as a compact Cypher-Lite ASCII expression.
    pub fn format_cypher_affordances(&self, path: &str, max_neighbors: usize) -> Option<String> {
        self.graph.format_cypher_affordances(path, max_neighbors)
    }

    /// Return the total in-degree of a node directly without allocating affordance maps.
    pub fn in_degree(&self, path: &str) -> usize {
        self.graph.in_degree(path)
    }

    /// Return all active distinct edge types present in the graph, optionally filtered by EdgeClass.
    pub fn active_edge_types(
        &self,
        class_filter: Option<ctxvault_common::config::EdgeClass>,
    ) -> Vec<String> {
        self.graph.active_edge_types(class_filter)
    }

    /// Build the set of graph node keys that represent code entities.
    ///
    /// Used by the search layer to classify a result path as code vs docs for
    /// modality filtering. Keys include each code symbol's `scope_path`, its
    /// defining file path, and the `<corpus>::scope_path` cross-corpus form.
    /// Returns an empty set on catalog read error (all paths then classify as
    /// docs, which is the safe default).
    pub fn code_paths_set(&self) -> std::collections::HashSet<String> {
        let mut set = std::collections::HashSet::new();
        let corpus = &self.config.name;
        if let Ok(symbols) = self.store.get_all_code_symbols() {
            for sym in symbols {
                let _ = set.insert(sym.scope_path.clone());
                let _ = set.insert(sym.file_path.clone());
                let _ = set.insert(format!("{}::{}", corpus, sym.scope_path));
            }
        }
        set
    }

    /// Get a port-typed reference to the metadata catalog for queries.
    pub fn store(&self) -> &impl MetadataCatalog {
        &self.store
    }

    /// Get the corpus config.
    pub fn config(&self) -> &CorpusConfig {
        &self.config
    }

    /// Get a mutable reference to the corpus config.
    pub fn config_mut(&mut self) -> &mut CorpusConfig {
        &mut self.config
    }

    /// Get the index directory for this engine.
    pub fn index_dir(&self) -> &Path {
        &self.index_dir
    }

    /// Get the compiled file exclude matcher for this engine.
    pub fn exclude_matcher(&self) -> &Arc<crate::index::exclude::ExcludeMatcher> {
        &self.exclude_matcher
    }

    /// Get the compiled file classifier for this engine.
    pub fn classifier(&self) -> &Arc<crate::index::classifier::FileClassifier> {
        &self.classifier
    }

    /// Returns the path to the derived text projection file for a given relative path.
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

    /// Analyze graph density, identifying hubs and orphans.
    ///
    /// Runs [`crate::analytics::analyze_density`] over this engine's graph,
    /// returning the top `top_hubs` most-connected nodes in the report.
    pub fn analyze_density(&self, top_hubs: usize) -> crate::analytics::DensityReport {
        crate::analytics::analyze_density(&self.graph, top_hubs)
    }

    /// Find queries where BM25 and vector search disagree.
    ///
    /// Embeds each query with this engine's embedder, then compares the BM25 and
    /// vector top-`top_k` results per query via
    /// [`crate::analytics::find_semantic_gaps`]. Returns an error in Fast Mode
    /// (no vector index). If the embedder is unavailable or any query fails to
    /// embed, returns `Ok(None)` so the caller can emit the "embedder not
    /// available" response; otherwise returns `Ok(Some(gaps))`.
    pub fn find_semantic_gaps(
        &self,
        queries: &[&str],
        top_k: usize,
    ) -> Result<Option<Vec<crate::analytics::SemanticGap>>> {
        let vector_index = match self.vector_index.as_ref() {
            Some(vi) => vi,
            None => {
                return Err(Error::Index(
                    "Semantic gap analysis is unavailable in fast mode. Re-index with index_mode = 'full' to enable vector search.".to_string(),
                ));
            }
        };

        // Embed each query (requires embedder).
        let query_embeddings: Vec<Vec<f32>> = if let Some(embedder) = self.embedder_arc() {
            queries.iter().filter_map(|q| embedder.embed_query(q).ok()).collect()
        } else {
            Vec::new()
        };

        // If not every query embedded, signal the "embedder unavailable" path.
        if query_embeddings.len() != queries.len() {
            return Ok(None);
        }

        let gaps = crate::analytics::find_semantic_gaps(
            &self.bm25,
            vector_index,
            queries,
            &query_embeddings,
            top_k,
        )?;
        Ok(Some(gaps))
    }

    /// Suggest chunks that may benefit from splitting.
    ///
    /// Runs [`crate::analytics::suggest_splits`] over this engine's catalog.
    pub fn suggest_splits(
        &self,
        max_chunk_chars: usize,
    ) -> Result<Vec<crate::analytics::SplitSuggestion>> {
        crate::analytics::suggest_splits(
            &self.store,
            Some(Path::new(&self.config.path)),
            max_chunk_chars,
        )
    }

    /// Read the exact UTF-8 text slice of a chunk directly from the source file on disk.
    ///
    /// Slices bytes `[start_byte..end_byte]` from the file at `rel_path` relative to `self.config.path`.
    pub fn fetch_chunk_text(
        &self,
        rel_path: &str,
        start_byte: usize,
        end_byte: usize,
    ) -> Result<String> {
        let proj_path = self.projection_path(rel_path);
        let target_path = if proj_path.is_file() {
            proj_path
        } else {
            Path::new(&self.config.path).join(rel_path)
        };

        let mut file = fs::File::open(&target_path).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("failed to open '{}': {e}", target_path.display()),
            ))
        })?;

        use std::io::{Read, Seek, SeekFrom};
        file.seek(SeekFrom::Start(start_byte as u64))?;
        let len = end_byte.saturating_sub(start_byte);
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    /// Generate a coverage report over the given test queries.
    ///
    /// Resolves all known note paths from this engine's catalog, then runs
    /// [`crate::analytics::coverage_report`] against its BM25 index.
    pub fn coverage_report(
        &self,
        queries: &[&str],
        top_k: usize,
    ) -> Result<crate::analytics::CoverageReport> {
        let files = self.store.list_files()?;
        let all_paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
        crate::analytics::coverage_report(&self.bm25, queries, &all_paths, top_k)
    }

    /// Re-embed all chunks with the current model, replacing old vectors.
    ///
    /// This performs a full re-embedding of all stored chunks:
    /// 1. Ensures embedder is initialized
    /// 2. Reads all chunks from the persistence store
    /// 3. Clears the vector index
    /// 4. Re-embeds all chunks and adds them to the vector index
    /// 5. Updates model_version metadata
    /// 6. Commits changes to disk
    ///
    /// Returns the number of chunks re-embedded.
    pub fn reembed(&mut self) -> Result<usize> {
        if self.is_fast_mode() || self.vector_index.is_none() {
            return Err(Error::Index(
                "re-embedding is unavailable in fast mode. Re-index with index_mode = 'full'"
                    .to_string(),
            ));
        }

        // 1. Ensure embedder is available.
        let available = self.ensure_embedder()?;
        if !available {
            return Err(Error::Index("embedder not available — cannot re-embed".to_string()));
        }
        let embedder = self.embedder_arc().unwrap();

        // 2. Get all files and their chunks from the store.
        let files = self.store.list_files()?;

        // 3. Reset vector index (preserve dimensions and params).
        let dims = self.vector_index.as_ref().unwrap().dimensions();
        self.vector_index = Some(VectorIndex::new_default(dims));

        // 4. Re-embed all chunks.
        let mut total_chunks = 0usize;
        let mut chunk_buffer: Vec<PendingChunk> = Vec::new();

        let corpus_path = std::path::PathBuf::from(&self.config.path);
        for file in &files {
            let is_code = crate::parser::code::is_code_file(std::path::Path::new(&file.path));
            if is_code {
                continue;
            }

            let full_path = corpus_path.join(&file.path);
            let parsed_chunks: Option<(Vec<ctxvault_common::types::Chunk>, Option<String>)> =
                std::fs::read_to_string(&full_path).ok().and_then(|content| {
                    let doc =
                        crate::parser::parse_document(std::path::Path::new(&file.path), &content)
                            .ok()?;
                    let chunks = crate::parser::chunker::chunk_document(
                        &file.path,
                        &doc.content,
                        &self.config.chunking,
                    );
                    Some((chunks, doc.title))
                });

            let (chunks, title) = if let Some((c, t)) = parsed_chunks {
                (c, t.or_else(|| file.title.clone()))
            } else {
                let chunk_records = self.store.get_chunks_for_file(&file.path)?;
                let mut chunks: Vec<ctxvault_common::types::Chunk> =
                    Vec::with_capacity(chunk_records.len());
                for cr in chunk_records {
                    let text = self
                        .fetch_chunk_text(&file.path, cr.start_byte, cr.end_byte)
                        .unwrap_or_default();
                    chunks.push(
                        ctxvault_common::types::Chunk::new(
                            file.path.clone(),
                            cr.chunk_index,
                            text,
                            cr.start_byte,
                            cr.end_byte,
                        )
                        .with_lines(cr.start_line, cr.end_line),
                    );
                }
                (chunks, file.title.clone())
            };

            if chunks.is_empty() {
                continue;
            }

            let doc_title = title.as_deref().unwrap_or("").trim();
            for c in &chunks {
                let section = c.heading_chain.as_deref().unwrap_or("").trim();
                let text = if !doc_title.is_empty() && !section.is_empty() {
                    format!("{} > {}: {}", doc_title, section, c.text)
                } else if !doc_title.is_empty() {
                    format!("{}: {}", doc_title, c.text)
                } else if !section.is_empty() {
                    format!("{}: {}", section, c.text)
                } else {
                    c.text.clone()
                };
                let modality = c
                    .entity_kind
                    .as_ref()
                    .map(EntityKind::modality_tag)
                    .unwrap_or("docs")
                    .to_string();
                chunk_buffer.push(PendingChunk {
                    doc_path: file.path.clone(),
                    chunk_index: c.chunk_index,
                    text,
                    embed_policy: c.embed_policy,
                    modality,
                });
            }

            if chunk_buffer.len() >= 64 {
                self.flush_chunk_buffer(&chunk_buffer)?;
                chunk_buffer.clear();
            }

            total_chunks += chunks.len();
        }

        // Flush any remaining buffered chunks
        if !chunk_buffer.is_empty() {
            self.flush_chunk_buffer(&chunk_buffer)?;
            chunk_buffer.clear();
        }

        // 5. Update model version metadata.
        let model_version = embedder.model_name().version_string().to_string();
        if let Some(ref mut vi) = self.vector_index {
            vi.set_model_version(&model_version);
            vi.clear_stale();
        }

        // 6. Store model version in persistence for audit trail.
        self.store.set_config("embedding_model", &model_version)?;

        // 7. Commit to disk.
        self.commit()?;

        info!(
            "Re-embedding complete: {} chunks re-embedded with model '{}'",
            total_chunks, model_version
        );

        Ok(total_chunks)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a single file (polyglot code or markdown) in a thread-safe, lock-free manner.
fn parse_file_record(
    rel_path: &str,
    full_path: &Path,
    bytes: &[u8],
    hash: String,
    classifier: &crate::index::classifier::FileClassifier,
    chunking_config: &ChunkingConfig,
    index_mode: IndexMode,
    sif: &crate::search::sif::SifEngine,
) -> Result<ParsedFileRecord> {
    let classification = classifier.classify(full_path, Some(bytes));

    match classification {
        crate::index::classifier::FileClassification::Code(_) => {
            let content = String::from_utf8_lossy(bytes);
            let path = Path::new(rel_path);
            let parse_res = crate::parser::code::chunker::CodeChunker::parse_and_chunk(
                path,
                &content,
                chunking_config,
            );

            let pending = Vec::new();
            let mut raw_chunks = Vec::new();
            let mut symbols = Vec::new();
            let mut graph_edges = Vec::new();
            let mut external_refs = Vec::new();

            if let Some(res) = parse_res {
                let symbol_index =
                    crate::graph::code::CodeGraphExtractor::build_symbol_index(&res.symbols);
                let extraction =
                    crate::graph::code::CodeGraphExtractor::extract_edges_for_file_with_index(
                        path,
                        &content,
                        &res.symbols,
                        &symbol_index,
                    );
                graph_edges = extraction.edges;
                external_refs = extraction.external_refs;

                raw_chunks = res.chunks;
                symbols = res.symbols;
            }

            // Project 256-bit binary fingerprints in Stage A worker thread (Pillars 2 & 3)
            use ctxvault_common::types::{FingerprintRecord, Modality};
            let mut fingerprints = Vec::with_capacity(1 + symbols.len() + raw_chunks.len());
            // File-level fingerprint for code
            let file_fp = sif.project_to_fingerprint(&content);
            fingerprints.push(FingerprintRecord {
                id: rel_path.to_string(),
                fingerprint: file_fp,
                modality: Modality::Code,
            });
            for sym in &symbols {
                let fp_text = format!(
                    "{} {} {}",
                    sym.name,
                    sym.signature,
                    sym.docstring.as_deref().unwrap_or("")
                );
                let fp = sif.project_to_fingerprint(&fp_text);
                fingerprints.push(FingerprintRecord {
                    id: format!("{}#{}", rel_path, sym.scope_path),
                    fingerprint: fp,
                    modality: Modality::Code,
                });
            }
            for chunk in &raw_chunks {
                let fp = sif.project_to_fingerprint(&chunk.text);
                fingerprints.push(FingerprintRecord {
                    id: format!("{rel_path}:chunk:{}", chunk.chunk_index),
                    fingerprint: fp,
                    modality: Modality::Code,
                });
            }

            Ok(ParsedFileRecord {
                path: rel_path.to_string(),
                hash,
                chunks: pending,
                raw_chunks,
                symbols,
                doc_metadata: None,
                graph_edges,
                external_refs,
                fingerprints,
                is_code: true,
                format: FileFormat::Source,
                projection_text: None,
            })
        }
        crate::index::classifier::FileClassification::MarkdownDoc => {
            let content = String::from_utf8_lossy(bytes);
            let path = Path::new(rel_path);
            let doc = parser::parse_document(path, &content)?;
            let chunks = chunker::chunk_document(rel_path, &doc.content, chunking_config);

            let doc_title = doc.title.as_deref().unwrap_or("").trim();
            let pending: Vec<PendingChunk> = if index_mode == IndexMode::Full {
                chunks
                    .iter()
                    .map(|c| {
                        let section = c.heading_chain.as_deref().unwrap_or("").trim();
                        let text = if !doc_title.is_empty() && !section.is_empty() {
                            format!("{} > {}: {}", doc_title, section, c.text)
                        } else if !doc_title.is_empty() {
                            format!("{}: {}", doc_title, c.text)
                        } else if !section.is_empty() {
                            format!("{}: {}", section, c.text)
                        } else {
                            c.text.clone()
                        };
                        let modality = c
                            .entity_kind
                            .as_ref()
                            .map(EntityKind::modality_tag)
                            .unwrap_or("docs")
                            .to_string();
                        PendingChunk {
                            doc_path: rel_path.to_string(),
                            chunk_index: c.chunk_index,
                            text,
                            embed_policy: c.embed_policy,
                            modality,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };

            // Project 256-bit binary fingerprints in Stage A worker thread (Pillars 2 & 3)
            use ctxvault_common::types::{FingerprintRecord, Modality};
            let mut fingerprints = Vec::with_capacity(chunks.len());
            for chunk in &chunks {
                let fp = sif.project_to_fingerprint(&chunk.text);
                fingerprints.push(FingerprintRecord {
                    id: if chunk.chunk_index == 0 {
                        rel_path.to_string()
                    } else {
                        format!("{rel_path}:chunk:{}", chunk.chunk_index)
                    },
                    fingerprint: fp,
                    modality: Modality::Docs,
                });
            }

            Ok(ParsedFileRecord {
                path: rel_path.to_string(),
                hash,
                chunks: pending,
                raw_chunks: chunks,
                symbols: Vec::new(),
                doc_metadata: Some(doc),
                graph_edges: Vec::new(),
                external_refs: Vec::new(),
                fingerprints,
                is_code: false,
                format: FileFormat::Source,
                projection_text: None,
            })
        }
        crate::index::classifier::FileClassification::Document(fmt) => {
            let registry = crate::parser::document::DocumentExtractorRegistry::new();
            let extracted = registry.extract(full_path, fmt, bytes)?;
            let chunks =
                chunker::chunk_document(rel_path, &extracted.normalized_text, chunking_config);

            let doc_title = extracted.title.as_deref().unwrap_or("").trim();
            let pending: Vec<PendingChunk> = if index_mode == IndexMode::Full {
                chunks
                    .iter()
                    .map(|c| {
                        let section = c.heading_chain.as_deref().unwrap_or("").trim();
                        let text = if !doc_title.is_empty() && !section.is_empty() {
                            format!("{} > {}: {}", doc_title, section, c.text)
                        } else if !doc_title.is_empty() {
                            format!("{}: {}", doc_title, c.text)
                        } else if !section.is_empty() {
                            format!("{}: {}", section, c.text)
                        } else {
                            c.text.clone()
                        };
                        let modality = c
                            .entity_kind
                            .as_ref()
                            .map(EntityKind::modality_tag)
                            .unwrap_or("docs")
                            .to_string();
                        PendingChunk {
                            doc_path: rel_path.to_string(),
                            chunk_index: c.chunk_index,
                            text,
                            embed_policy: c.embed_policy,
                            modality,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };

            let doc = Document {
                path: rel_path.to_string(),
                frontmatter: None,
                title: extracted.title.clone(),
                tags: Vec::new(),
                wikilinks: Vec::new(),
                template: None,
                content: extracted.normalized_text.clone(),
                content_hash: hash.clone(),
            };

            let mut graph_edges = Vec::new();
            for link in extracted.outbound_links {
                graph_edges.push(Edge {
                    source: rel_path.to_string(),
                    target: link.target,
                    edge_type: "references".to_string(),
                    weight: 0.8,
                    provenance: ctxvault_common::types::EdgeProvenance::MarkdownLink,
                    target_corpus: None,
                    confidence: Some(ctxvault_common::types::ResolutionConfidence::High),
                    target_path: None,
                    target_symbol: None,
                    target_kind: None,
                });
            }

            // Project 256-bit binary fingerprints in Stage A worker thread (Pillars 2 & 3)
            use ctxvault_common::types::{FingerprintRecord, Modality};
            let mut fingerprints = Vec::with_capacity(chunks.len());
            for chunk in &chunks {
                let fp = sif.project_to_fingerprint(&chunk.text);
                fingerprints.push(FingerprintRecord {
                    id: if chunk.chunk_index == 0 {
                        rel_path.to_string()
                    } else {
                        format!("{rel_path}:chunk:{}", chunk.chunk_index)
                    },
                    fingerprint: fp,
                    modality: Modality::Docs,
                });
            }

            Ok(ParsedFileRecord {
                path: rel_path.to_string(),
                hash,
                chunks: pending,
                raw_chunks: chunks,
                symbols: Vec::new(),
                doc_metadata: Some(doc),
                graph_edges,
                external_refs: Vec::new(),
                fingerprints,
                is_code: false,
                format: fmt,
                projection_text: Some(extracted.normalized_text),
            })
        }
        crate::index::classifier::FileClassification::Ignored => Err(Error::Parse {
            path: rel_path.to_string(),
            message: format!("file '{rel_path}' is ignored or unsupported format"),
        }),
    }
}

/// Current Unix timestamp in seconds.
fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("system time before epoch").as_secs() as i64
}

/// Recursively walk a directory and collect all indexable files (.md and polyglot source code).
/// Returns `(relative_path, absolute_path)` pairs.
fn walk_markdown_files(
    root: &Path,
    matcher: &crate::index::exclude::ExcludeMatcher,
    classifier: &crate::index::classifier::FileClassifier,
) -> Result<Vec<(String, PathBuf)>> {
    let mut results = Vec::new();
    if !root.exists() {
        return Ok(results);
    }
    walk_dir_recursive(root, root, matcher, classifier, &mut results)?;
    Ok(results)
}

fn walk_dir_recursive(
    root: &Path,
    current: &Path,
    matcher: &crate::index::exclude::ExcludeMatcher,
    classifier: &crate::index::classifier::FileClassifier,
    results: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    let entries = fs::read_dir(current)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Check exclusion for directories; prune subtrees immediately
            if matcher.is_excluded(&path, true) {
                continue;
            }
            walk_dir_recursive(root, &path, matcher, classifier, results)?;
        } else {
            // Check exclusion for individual files
            if matcher.is_excluded(&path, false) {
                continue;
            }
            let is_indexable = classifier.classify(&path, None).is_indexable();
            if is_indexable {
                let rel = path.strip_prefix(root).map_err(|e| {
                    Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                })?;
                // Normalize path separators to forward slashes.
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                results.push((rel_str, path.clone()));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ctxvault_common::types::CodeSymbolType;
    use tempfile::TempDir;

    /// Create a minimal corpus config pointing at the given path.
    fn test_config(corpus_path: &Path) -> CorpusConfig {
        CorpusConfig {
            name: "test".to_string(),
            path: corpus_path.to_string_lossy().to_string(),
            mode: ctxvault_common::config::CorpusMode::ReadWrite,
            index_mode: ctxvault_common::config::IndexMode::Full,
            chunking: ctxvault_common::config::ChunkingConfig {
                min_chunk_tokens: 1, // very low for tests
                ..Default::default()
            },
            embedding: ctxvault_common::config::EmbeddingConfig::default(),
            graph: ctxvault_common::config::GraphConfig {
                edge_types: vec![ctxvault_common::config::EdgeTypeConfig {
                    name: "Wikilink".to_string(),
                    source: ctxvault_common::config::EdgeSource::Wikilink,
                    weight: 1.0,
                    bidirectional: false,
                    field: None,
                    direction: None,
                    max_frequency: None,
                    class: None,
                    description: None,
                    allowed_source_templates: None,
                    allowed_target_templates: None,
                }],
            },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        }
    }

    #[test]
    fn test_fast_mode_skips_vectors_and_embedder() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        fs::write(corpus_dir.join("file1.md"), "# File 1\nSome test markdown content").unwrap();

        let mut config = test_config(&corpus_dir);
        config.index_mode = ctxvault_common::config::IndexMode::Fast;

        let index_dir = tmp.path().join("index");
        let mut engine = Engine::open(config, &index_dir).unwrap();
        assert!(engine.is_fast_mode());
        assert!(!engine.has_vector_index());
        assert_eq!(engine.ensure_embedder().unwrap(), false);

        let files = engine.full_reindex_paginated(10, false).unwrap();
        assert_eq!(files, 1);
        assert!(!engine.has_vector_index());
        assert_eq!(engine.store().list_files().unwrap().len(), 1);
    }

    #[test]
    fn test_full_mode_docs_vector_code_hamming() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();

        fs::write(
            corpus_dir.join("guide.md"),
            "# Architecture Guide\n\n## Overview\n\nFull mode indexes docs into dense vector store and code into binary hamming.\n",
        )
        .unwrap();

        fs::write(
            corpus_dir.join("lib.rs"),
            "pub struct EngineConfig {\n    pub name: String,\n}\n\npub fn run_engine() {}\n",
        )
        .unwrap();

        let mut config = test_config(&corpus_dir);
        config.index_mode = ctxvault_common::config::IndexMode::Full;

        let index_dir = tmp.path().join("index");
        let mut engine = Engine::open(config, &index_dir).unwrap();

        assert!(!engine.is_fast_mode());
        assert!(engine.has_vector_index());

        let files_indexed = engine.full_reindex_paginated(10, false).unwrap();
        assert_eq!(files_indexed, 2);

        // Verify SQLite store contains both files, chunks, and symbols
        assert_eq!(engine.store().list_files().unwrap().len(), 2);
        let code_symbols = engine.store().find_symbols_by_name("EngineConfig").unwrap();
        assert!(!code_symbols.is_empty());

        // Verify BM25 indexed both files
        let bm25_doc = engine.bm25.search("Architecture", 10).unwrap();
        assert!(!bm25_doc.is_empty());
        let bm25_code = engine.bm25.search("EngineConfig", 10).unwrap();
        assert!(!bm25_code.is_empty());

        // Verify Graph indexed both
        assert!(engine.graph().node_count() >= 2);

        // Staged chunk generation: code chunk pending vector queue must be empty
        let (code_pending, _) = engine.index_file_staged("src/main.rs", "pub struct Foo;").unwrap();
        assert!(code_pending.is_empty(), "Code chunks must not be staged for vector embedding");

        // Staged chunk generation: doc chunk pending vector queue must contain chunks
        let (doc_pending, _) = engine
            .index_file_staged("readme.md", "# Readme\n\n## Overview\n\nAnchor content.")
            .unwrap();
        let has_anchor = doc_pending
            .iter()
            .any(|c| c.embed_policy == ctxvault_common::types::ChunkEmbedPolicy::Anchor);
        assert!(has_anchor, "Doc chunks must be staged for vector embedding in Full mode");

        // Verify binary index has entries for code
        assert!(!engine.binary_index().is_empty());
    }

    #[test]
    fn test_open_creates_index_dir() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        let config = test_config(&corpus_dir);
        let _engine = Engine::open(config, &index_dir).unwrap();

        // Verify index directory structure was created.
        assert!(index_dir.exists());
        assert!(index_dir.join("meta.db").exists());
        assert!(index_dir.join("tantivy").exists());
    }

    #[test]
    fn test_index_and_search() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config, &index_dir).unwrap();

        let content =
            "# Rust Programming\n\nRust is a systems programming language focused on safety.\n";
        engine.index_file("rust.md", content).unwrap();
        engine.commit().unwrap();

        // Verify searchable via BM25.
        let results = engine.bm25.search("systems programming", 10).unwrap();
        assert!(!results.is_empty(), "Should find indexed file via search");
        assert_eq!(results[0].path, "rust.md");

        // Verify stored in persistence.
        let file = engine.store().get_file("rust.md").unwrap();
        assert!(file.is_some());
        assert_eq!(file.unwrap().title.as_deref(), Some("Rust Programming"));
    }

    #[test]
    fn test_remove_file() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config, &index_dir).unwrap();

        let content = "# To Remove\n\nThis note will be removed.\n";
        engine.index_file("remove_me.md", content).unwrap();
        engine.commit().unwrap();

        // Confirm it's indexed.
        assert!(engine.store().get_file("remove_me.md").unwrap().is_some());

        // Remove it.
        engine.remove_file("remove_me.md").unwrap();
        engine.commit().unwrap();

        // Verify gone from all stores.
        assert!(engine.store().get_file("remove_me.md").unwrap().is_none());
        let results = engine.bm25.search("removed", 10).unwrap();
        assert!(results.iter().all(|r| r.path != "remove_me.md"), "File should be gone from BM25");
        assert!(!engine.graph().contains_node("remove_me.md"));
    }

    #[test]
    fn test_delta_scan() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        // Create initial files.
        fs::write(corpus_dir.join("existing.md"), "# Existing\n\nOriginal content.\n").unwrap();
        fs::write(corpus_dir.join("will_modify.md"), "# Will Modify\n\nOriginal.\n").unwrap();
        fs::write(corpus_dir.join("will_delete.md"), "# Will Delete\n\nGoing away.\n").unwrap();

        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config, &index_dir).unwrap();

        // Initial full index.
        let count = engine.full_reindex().unwrap();
        assert_eq!(count, 3);

        // Now modify one file, delete one, and add a new one.
        fs::write(
            corpus_dir.join("will_modify.md"),
            "# Will Modify\n\nUpdated content that is different.\n",
        )
        .unwrap();
        fs::remove_file(corpus_dir.join("will_delete.md")).unwrap();
        fs::write(corpus_dir.join("new_file.md"), "# New File\n\nBrand new.\n").unwrap();

        // Run delta scan.
        let result = engine.delta_scan().unwrap();

        assert_eq!(result.new_files, vec!["new_file.md"]);
        assert_eq!(result.modified_files, vec!["will_modify.md"]);
        assert_eq!(result.deleted_files, vec!["will_delete.md"]);

        // Verify the new file is searchable.
        let search = engine.bm25.search("brand new", 10).unwrap();
        assert!(search.iter().any(|r| r.path == "new_file.md"));

        // Verify deleted file is gone.
        assert!(engine.store().get_file("will_delete.md").unwrap().is_none());
    }

    #[test]
    fn test_full_reindex() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        fs::write(corpus_dir.join("alpha.md"), "# Alpha\n\nFirst note.\n").unwrap();
        fs::write(corpus_dir.join("beta.md"), "# Beta\n\nSecond note.\n").unwrap();
        fs::write(corpus_dir.join("gamma.md"), "# Gamma\n\nThird note with [[alpha]] link.\n")
            .unwrap();

        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config, &index_dir).unwrap();

        let count = engine.full_reindex().unwrap();
        assert_eq!(count, 3);

        // All files should be in the store.
        let files = engine.store().list_files().unwrap();
        assert_eq!(files.len(), 3);

        // Graph should have the wikilink edge from gamma to alpha.
        let fwd = engine.graph().forwardlinks("gamma.md", None);
        let targets = fwd.get("Wikilink").unwrap_or(&Vec::new()).clone();
        assert!(
            targets.contains(&"alpha".to_string()),
            "Expected wikilink edge from gamma to alpha"
        );

        // Verify search works.
        let results = engine.bm25.search("Second note", 10).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].path, "beta.md");
    }

    #[test]
    fn test_model_version_set_on_new_index() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        let config = test_config(&corpus_dir);
        let engine = Engine::open(config, &index_dir).unwrap();

        // New empty vector index should not be stale.
        assert!(!engine.vectors_stale());
    }

    #[test]
    fn test_model_version_mismatch_marks_stale() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        // Create a vector index file with a different model version.
        fs::create_dir_all(&index_dir).unwrap();
        let mut vi = VectorIndex::new_default(768);
        vi.set_model_version("some-other-model-v99");
        vi.add(&vec![0.1f32; 768], "test.md", Some(0), false, "text").unwrap();
        vi.save_binary(&index_dir.join("vectors.bin")).unwrap();

        let config = test_config(&corpus_dir);
        let engine = Engine::open(config, &index_dir).unwrap();

        // Should be marked stale due to version mismatch.
        assert!(engine.vectors_stale());
        assert_eq!(engine.stored_model_version(), Some("some-other-model-v99"));
    }

    #[test]
    fn test_model_version_no_version_marks_stale() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        // Create a vector index file WITHOUT model_version.
        fs::create_dir_all(&index_dir).unwrap();
        let mut vi = VectorIndex::new_default(768);
        vi.add(&vec![0.1f32; 768], "test.md", Some(0), false, "text").unwrap();
        vi.save_binary(&index_dir.join("vectors.bin")).unwrap();

        let config = test_config(&corpus_dir);
        let engine = Engine::open(config, &index_dir).unwrap();

        // Vectors with no model_version with data should be marked stale.
        assert!(engine.vectors_stale());
    }

    #[test]
    fn test_corpus_config_persistence() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        let config = test_config(&corpus_dir);
        let engine = Engine::open(config, &index_dir).unwrap();

        // Set and get config.
        engine.store().set_config("embedding_model", "all-minilm-l6-v2").unwrap();
        let value = engine.store().get_config("embedding_model").unwrap();
        assert_eq!(value, Some("all-minilm-l6-v2".to_string()));

        // Non-existent key returns None.
        let missing = engine.store().get_config("nonexistent").unwrap();
        assert_eq!(missing, None);
    }

    #[test]
    fn test_paginated_reindex_and_status() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        // Write 15 files
        for i in 0..15 {
            fs::write(
                corpus_dir.join(format!("doc_{:02}.md", i)),
                format!("# Document {}\n\nContent for note {}\n", i, i),
            )
            .unwrap();
        }

        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config, &index_dir).unwrap();

        // Index in batches of 5
        let count = engine.full_reindex_paginated(5, false).unwrap();
        assert_eq!(count, 15);

        // Check indexing status
        let status = engine.get_indexing_status().unwrap();
        assert_eq!(status.corpus_id, "test");
        assert_eq!(status.status, IndexingStatus::Completed);
        assert_eq!(status.total_files, 15);
        assert_eq!(status.indexed_files, 15);
        assert_eq!(status.progress_percent, 100.0);
    }

    #[test]
    fn test_indexing_resumption() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");

        // Write 10 files
        for i in 0..10 {
            fs::write(
                corpus_dir.join(format!("doc_{:02}.md", i)),
                format!("# Document {}\n\nContent for note {}\n", i, i),
            )
            .unwrap();
        }

        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config.clone(), &index_dir).unwrap();

        // 1. First index all 10 files
        let count = engine.full_reindex_paginated(4, false).unwrap();
        assert_eq!(count, 10);

        // 2. Add 5 more files to corpus
        for i in 10..15 {
            fs::write(
                corpus_dir.join(format!("doc_{:02}.md", i)),
                format!("# Document {}\n\nContent for note {}\n", i, i),
            )
            .unwrap();
        }

        // 3. Open fresh engine instance and resume indexing
        let mut resumed_engine = Engine::open(config, &index_dir).unwrap();
        let resumed_count = resumed_engine.full_reindex_paginated(4, true).unwrap();
        assert_eq!(resumed_count, 15);

        let files = resumed_engine.store().list_files().unwrap();
        assert_eq!(files.len(), 15);
    }

    #[test]
    fn test_polyglot_codebase_indexing_and_cross_modal_search() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(corpus_dir.join("docs/adr")).unwrap();
        fs::create_dir_all(corpus_dir.join("src")).unwrap();
        fs::create_dir_all(corpus_dir.join("scripts")).unwrap();
        let index_dir = tmp.path().join("index");

        // 1. Write markdown ADR
        let adr_content = r#"---
title: ADR-0001 Hybrid Search
tags: [search, rrf, architecture]
---
# ADR-0001: Reciprocal Rank Fusion Search

We implement 4-way RRF hybrid search combining BM25, embeddings, and graph traversal.
"#;
        fs::write(corpus_dir.join("docs/adr/0001-hybrid-search.md"), adr_content).unwrap();

        // 2. Write Rust file
        let rust_code = r#"
/// Search engine implementation
pub struct Engine;

impl Engine {
    /// Execute hybrid search across all modalities
    pub fn search_hybrid(&self, query: &str) -> Vec<String> {
        let results = execute_rrf(query);
        results
    }
}

pub fn execute_rrf(q: &str) -> Vec<String> {
    vec![q.to_string()]
}
"#;
        fs::write(corpus_dir.join("src/search.rs"), rust_code).unwrap();

        // 3. Write TypeScript file
        let ts_code = r#"
export interface UserProfile {
    id: string;
    email: string;
}

export class UserService {
    /** Fetch user by ID */
    async getUser(id: string): Promise<UserProfile> {
        return { id, email: "user@example.com" };
    }
}
"#;
        fs::write(corpus_dir.join("src/user.ts"), ts_code).unwrap();

        // 4. Write Python script
        let py_code = r#"
class DataIngest:
    """Batch data ingestion pipeline."""
    def run_pipeline(self, batch):
        return len(batch)
"#;
        fs::write(corpus_dir.join("scripts/process.py"), py_code).unwrap();

        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config, &index_dir).unwrap();

        // Perform full reindex
        let count = engine.full_reindex().unwrap();
        assert_eq!(count, 4, "Should index 1 markdown file + 3 polyglot code files");

        // Verify BM25 search across modalities
        let adr_hits = engine.bm25.search("Reciprocal Rank Fusion", 5).unwrap();
        assert!(!adr_hits.is_empty());
        assert_eq!(adr_hits[0].path, "docs/adr/0001-hybrid-search.md");

        let rust_hits = engine.bm25.search("search_hybrid modalities", 5).unwrap();
        assert!(!rust_hits.is_empty());
        assert_eq!(rust_hits[0].path, "src/search.rs");

        let ts_hits = engine.bm25.search("UserProfile getUser", 5).unwrap();
        assert!(!ts_hits.is_empty());
        assert_eq!(ts_hits[0].path, "src/user.ts");

        // Verify SQLite code_symbols catalog
        let rust_symbols = engine.store().get_code_symbols_for_file("src/search.rs").unwrap();
        assert!(rust_symbols
            .iter()
            .any(|s| s.name == "Engine" && s.symbol_type == CodeSymbolType::Struct));
        assert!(rust_symbols
            .iter()
            .any(|s| s.name == "search_hybrid" && s.symbol_type == CodeSymbolType::Function));
        assert!(rust_symbols
            .iter()
            .any(|s| s.name == "execute_rrf" && s.symbol_type == CodeSymbolType::Function));

        let ts_symbols = engine.store().get_code_symbols_for_file("src/user.ts").unwrap();
        assert!(ts_symbols
            .iter()
            .any(|s| s.name == "UserService" && s.symbol_type == CodeSymbolType::Class));
        assert!(ts_symbols
            .iter()
            .any(|s| s.name == "getUser" && s.symbol_type == CodeSymbolType::Method));

        // Verify graph edges (defines and calls)
        let edges = engine.graph().get_all_edges();
        assert!(edges.iter().any(|e| e.edge_type == "defines"
            && e.source == "src/search.rs"
            && e.target == "Engine"));
        assert!(edges.iter().any(|e| e.edge_type == "calls"
            && e.source == "Engine > search_hybrid"
            && e.target == "execute_rrf"));
    }

    #[test]
    fn test_indexing_excludes_test_and_node_modules() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("repo");
        fs::create_dir_all(corpus_dir.join("src")).unwrap();
        fs::create_dir_all(corpus_dir.join("tests")).unwrap();
        fs::create_dir_all(corpus_dir.join("node_modules").join("pkg")).unwrap();
        fs::create_dir_all(corpus_dir.join("target").join("debug")).unwrap();

        // Valid source file
        fs::write(corpus_dir.join("src").join("main.rs"), "fn main() { println!(\"hello\"); }")
            .unwrap();
        // Excluded test file in tests/
        fs::write(corpus_dir.join("tests").join("integration_test.rs"), "fn test_it() {}").unwrap();
        // Excluded test file in src/
        fs::write(corpus_dir.join("src").join("app.test.rs"), "fn app_test() {}").unwrap();
        // Excluded dependency
        fs::write(
            corpus_dir.join("node_modules").join("pkg").join("index.js"),
            "module.exports = {};",
        )
        .unwrap();
        // Excluded build artifact
        fs::write(corpus_dir.join("target").join("debug").join("out.rs"), "fn out() {}").unwrap();
        // .gitignore rule migrated into config
        fs::write(corpus_dir.join(".gitignore"), "secrets.rs\n").unwrap();
        fs::write(corpus_dir.join("secrets.rs"), "fn secret() {}").unwrap();

        let mut config = test_config(&corpus_dir);
        config.exclude.import_gitignore(&corpus_dir.join(".gitignore"));
        config.index_mode = ctxvault_common::config::IndexMode::Fast;

        let index_dir = tmp.path().join("index");
        let mut engine = Engine::open(config, &index_dir).unwrap();
        let sync_res = engine.delta_scan_paginated(500).unwrap();

        // Only src/main.rs should be indexed!
        assert_eq!(sync_res.new_files, vec!["src/main.rs"]);
        assert!(engine.store().get_file("src/main.rs").unwrap().is_some());
        assert!(engine.store().get_file("tests/integration_test.rs").unwrap().is_none());
        assert!(engine.store().get_file("src/app.test.rs").unwrap().is_none());
        assert!(engine.store().get_file("node_modules/pkg/index.js").unwrap().is_none());
        assert!(engine.store().get_file("secrets.rs").unwrap().is_none());
    }
}
