//! Indexing pipeline, delta scanning, batching, commits, and re-embedding.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use groundcontrol_common::config::{ChunkingConfig, IndexMode};
use groundcontrol_common::types::{
    ChunkEmbedPolicy, ChunkRecord, Document, FileFormat, IndexingState, IndexingStatus,
};
use groundcontrol_common::{Error, Result};

use crate::embedding::Embedder;
use crate::index::pipeline::AsyncEmbeddingPipeline;
use groundcontrol_common::types::ParsedArtifact;

use super::state::Engine;
use super::types::{now_unix, walk_markdown_files, DeltaScanResult, PendingChunk};

/// Convert a domain chunk into a pending chunk for vector embedding.
fn chunk_to_pending(
    chunk: &groundcontrol_common::types::Chunk,
    doc_path: &str,
    doc_title: &str,
    is_code: bool,
) -> PendingChunk {
    let section = chunk.heading_chain.as_deref().unwrap_or("").trim();
    let text = if !doc_title.is_empty() && !section.is_empty() {
        format!("{} > {}: {}", doc_title, section, chunk.text)
    } else if !doc_title.is_empty() {
        format!("{}: {}", doc_title, chunk.text)
    } else if !section.is_empty() {
        format!("{}: {}", section, chunk.text)
    } else {
        chunk.text.clone()
    };
    let modality = chunk
        .entity_kind
        .as_ref()
        .map(groundcontrol_common::types::EntityKind::modality_tag)
        .unwrap_or_else(|| if is_code { "code" } else { "docs" })
        .to_string();
    PendingChunk {
        doc_path: doc_path.to_string(),
        chunk_index: chunk.chunk_index,
        text,
        embed_policy: chunk.embed_policy,
        modality,
    }
}

impl Engine {
    /// Staged file indexing: parses, chunks, updates persistence, and broadcasts to all
    /// registered retrieval algorithms without immediately triggering embedding inference.
    pub fn index_file_staged(
        &mut self,
        rel_path: &str,
        content: &str,
    ) -> Result<(Vec<PendingChunk>, Option<Document>)> {
        let path = Path::new(rel_path);
        let bytes = content.as_bytes();
        let hash = blake3::hash(bytes).to_hex().to_string();

        let record = parse_file_record(
            rel_path,
            path,
            bytes,
            hash,
            &self.classifier,
            &self.config.chunking,
            self.config.index_mode,
        )?;

        let tag_configs: Vec<_> = self
            .config
            .graph
            .edge_types
            .iter()
            .filter(|et| et.source == groundcontrol_common::config::EdgeSource::Tag)
            .cloned()
            .collect();

        let pending = if self.config.index_mode == groundcontrol_common::config::IndexMode::Full
            && !record.is_code
        {
            record
                .chunks
                .iter()
                .map(|c| {
                    chunk_to_pending(
                        c,
                        &record.path,
                        record.title.as_deref().unwrap_or(""),
                        record.is_code,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let doc = record.doc_metadata.clone();

        let mut dummy_docs = Vec::new();
        self.ingest_parsed_record(record, &tag_configs, &mut dummy_docs)?;

        debug!("Staged file: {}", rel_path);
        Ok((pending, doc))
    }

    /// Flush a batch of pending chunks into the vector index in a single vectorized forward pass.
    pub fn flush_chunk_buffer(&mut self, buffer: &[PendingChunk]) -> Result<()> {
        if buffer.is_empty() {
            return Ok(());
        }

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
            let modality = file_chunks[0].modality.as_str();

            if let Some(ref mut vi) = self.vector_index_mut() {
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

        self.store.delete_file(rel_path)?;
        self.remove_artifact(rel_path)?;

        debug!("Removed file: {}", rel_path);
        Ok(())
    }

    /// Ingest a single parsed file record into SQLite persistence and broadcast to all retrieval algorithms.
    pub(crate) fn ingest_parsed_record(
        &mut self,
        record: ParsedArtifact,
        tag_configs: &[groundcontrol_common::config::EdgeTypeConfig],
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

        let file_title = record.title.clone().unwrap_or_else(|| {
            Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path).to_string()
        });

        // 1. SQLite Store
        self.store.insert_file(
            path,
            &record.hash,
            modified_at,
            record.doc_metadata.as_ref().and_then(|d| d.template.as_deref()),
            Some(&file_title),
            record.format,
        )?;

        // 2. Chunks and symbols
        self.store.delete_chunks_for_file(path)?;
        let chunk_records: Vec<ChunkRecord> = record
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
        self.store.insert_chunks(path, &chunk_records)?;

        if record.is_code {
            self.store.save_code_symbols(path, &record.symbols)?;
            self.store.clear_external_refs_for_file(path)?;
            if !record.external_refs.is_empty() {
                self.store.insert_external_refs(path, &record.external_refs)?;
            }
        }

        // 3. Broadcast to all retrieval algorithms
        self.broadcast_artifact(&record)?;

        // 4. Document edge rules & Tag edge accumulation
        if let Some(mut doc) = record.doc_metadata {
            let edge_configs = self.effective_edge_configs_for_document(&doc, None);
            self.graph.build_edges_for_document(&doc, &edge_configs, &[]);
            for edge in &record.graph_edges {
                self.graph.add_edge(
                    &edge.source,
                    &edge.target,
                    &edge.edge_type,
                    edge.weight,
                    edge.provenance.clone(),
                    groundcontrol_common::config::EdgeClass::Structural,
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
    pub(crate) fn read_file_lossy(path: &Path) -> std::io::Result<String> {
        let bytes = fs::read(path)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Perform a delta scan with default batch size.
    pub fn delta_scan(&mut self) -> Result<DeltaScanResult> {
        self.delta_scan_paginated(50)
    }

    /// Perform a paginated delta scan: compare filesystem against stored file records.
    pub fn delta_scan_paginated(&mut self, batch_size: usize) -> Result<DeltaScanResult> {
        let commit_batch_size = if batch_size == 0 || batch_size == 50 { 500 } else { batch_size };
        if self.config.index_mode != groundcontrol_common::config::IndexMode::Fast {
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
                _ => {}
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
            if self.config.index_mode == groundcontrol_common::config::IndexMode::Fast {
                None
            } else {
                self.embedder_arc().map(AsyncEmbeddingPipeline::new)
            };

        let tag_configs: Vec<_> = self
            .config
            .graph
            .edge_types
            .iter()
            .filter(|et| et.source == groundcontrol_common::config::EdgeSource::Tag)
            .cloned()
            .collect();
        let mut all_docs: Vec<Document> = Vec::new();

        if !files_to_index.is_empty() {
            let num_cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
            let (work_tx, work_rx) = crossbeam_channel::unbounded::<(String, PathBuf)>();
            let (ast_tx, ast_rx) = crossbeam_channel::bounded::<ParsedArtifact>(2048);

            let chunk_tx_opt = embedding_pipeline.as_ref().and_then(|p| p.chunk_sender());
            let chunking_config = self.config.chunking.clone();
            let index_mode = self.config.index_mode;

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
                                ) {
                                    Ok(r) => r,
                                    Err(e) => {
                                        warn!("Failed to parse {}: {}", rel_path, e);
                                        continue;
                                    }
                                };

                                if let Some(ref tx) = chunk_tx_clone {
                                    if !record.is_code {
                                        for chunk in &record.chunks {
                                            if chunk.embed_policy == ChunkEmbedPolicy::Anchor {
                                                let pc = chunk_to_pending(
                                                    chunk,
                                                    &record.path,
                                                    record.title.as_deref().unwrap_or(""),
                                                    record.is_code,
                                                );
                                                if tx.send(pc).is_err() {
                                                    break;
                                                }
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
                        if let Some(ref mut vi) = self.vector_index_mut() {
                            let _ = pipeline.try_recv_completed(vi);
                        }
                    }

                    uncommitted_count += 1;

                    if uncommitted_count >= commit_batch_size
                        || last_commit_time.elapsed() >= commit_time_threshold
                    {
                        if let Some(ref pipeline) = embedding_pipeline {
                            if let Some(ref mut vi) = self.vector_index_mut() {
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

            if let Some(mut pipeline) = embedding_pipeline {
                if let Some(ref mut vi) = self.vector_index_mut() {
                    pipeline.finish(vi)?;
                }
            }

            if uncommitted_count > 0 {
                self.commit_intermediate()?;
            }

            if !tag_configs.is_empty() && !all_docs.is_empty() {
                self.graph.build_all_tag_edges(&tag_configs, &all_docs);
            }

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

    /// Incrementally synchronize a specific list of changed or deleted paths for a corpus.
    pub fn sync_delta_paths(&mut self, paths: &[PathBuf]) -> Result<DeltaScanResult> {
        let corpus_path = PathBuf::from(&self.config.path);
        let mut new_files = Vec::new();
        let mut modified_files = Vec::new();
        let mut deleted_files = Vec::new();

        self.ensure_vector_index();
        if self.config.index_mode != groundcontrol_common::config::IndexMode::Fast {
            let _ = self.ensure_embedder();
        }
        let embedding_pipeline =
            if self.config.index_mode == groundcontrol_common::config::IndexMode::Fast {
                None
            } else {
                self.embedder_arc().map(AsyncEmbeddingPipeline::new)
            };

        let tag_configs: Vec<_> = self
            .config
            .graph
            .edge_types
            .iter()
            .filter(|et| et.source == groundcontrol_common::config::EdgeSource::Tag)
            .cloned()
            .collect();
        let mut all_docs: Vec<Document> = Vec::new();
        let classifier = crate::index::classifier::FileClassifier::new(&corpus_path, &self.config);

        for path in paths {
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
                if self.store.get_file(&rel_path)?.is_some() {
                    self.remove_file(&rel_path)?;
                    deleted_files.push(rel_path);
                }
            } else {
                if self.exclude_matcher.is_excluded(&full_path, full_path.is_dir()) {
                    continue;
                }
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
                    ) {
                        Ok(r) => r,
                        Err(e) => {
                            warn!("Failed to parse {}: {}", rel_path, e);
                            continue;
                        }
                    };

                    if let Some(ref pipeline) = embedding_pipeline {
                        if let Some(tx) = pipeline.chunk_sender() {
                            if !record.is_code {
                                for chunk in &record.chunks {
                                    if chunk.embed_policy == ChunkEmbedPolicy::Anchor {
                                        let pc = chunk_to_pending(
                                            chunk,
                                            &record.path,
                                            record.title.as_deref().unwrap_or(""),
                                            record.is_code,
                                        );
                                        let _ = tx.send(pc);
                                    }
                                }
                            }
                        }
                    }

                    if let Err(e) = self.ingest_parsed_record(record, &tag_configs, &mut all_docs) {
                        warn!("Failed to ingest {}: {}", rel_path, e);
                        continue;
                    }

                    if let Some(ref pipeline) = embedding_pipeline {
                        if let Some(ref mut vi) = self.vector_index_mut() {
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
            if let Some(ref mut vi) = self.vector_index_mut() {
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
    pub fn full_reindex(&mut self) -> Result<usize> {
        self.full_reindex_paginated(50, false)
    }

    /// Paginated, resumable full reindex: scans corpus directory in configurable batches.
    pub fn full_reindex_paginated(&mut self, batch_size: usize, resume: bool) -> Result<usize> {
        let commit_batch_size = if batch_size == 0 || batch_size == 50 { 500 } else { batch_size };
        let corpus_id = self.config.name.clone();
        let corpus_path = PathBuf::from(&self.config.path);
        let mut disk_files =
            walk_markdown_files(&corpus_path, &self.exclude_matcher, &self.classifier)?;
        disk_files.sort_by(|a, b| a.0.cmp(&b.0));
        let total_files = disk_files.len();

        self.ensure_vector_index();
        if self.config.index_mode != groundcontrol_common::config::IndexMode::Fast {
            let _ = self.ensure_embedder();
        }

        let mut stored_map: HashMap<String, String> = HashMap::new();

        if !resume {
            let existing = self.store.list_files()?;
            for file in &existing {
                self.store.delete_file(&file.path)?;
            }
            self.clear_algorithms()?;
            self.store.reset_indexing_state(&corpus_id)?;
        } else {
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
            .filter(|et| et.source == groundcontrol_common::config::EdgeSource::Tag)
            .cloned()
            .collect();
        let mut all_docs: Vec<Document> = Vec::new();
        let embedding_pipeline =
            if self.config.index_mode == groundcontrol_common::config::IndexMode::Fast {
                None
            } else {
                self.embedder_arc().map(AsyncEmbeddingPipeline::new)
            };

        let num_cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
        let (work_tx, work_rx) = crossbeam_channel::unbounded::<(String, PathBuf)>();
        let (ast_tx, ast_rx) = crossbeam_channel::bounded::<ParsedArtifact>(2048);

        let chunk_tx_opt = embedding_pipeline.as_ref().and_then(|p| p.chunk_sender());
        let chunking_config = self.config.chunking.clone();
        let index_mode = self.config.index_mode;

        for file_entry in disk_files {
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
                let stored_map_ref = &stored_map;
                let chunking_ref = &chunking_config;
                let classifier_clone = self.classifier.clone();

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
                            ) {
                                Ok(r) => r,
                                Err(e) => {
                                    warn!("Failed to parse {}: {}", rel_path, e);
                                    continue;
                                }
                            };

                            if let Some(ref tx) = chunk_tx_clone {
                                if !record.is_code {
                                    for chunk in &record.chunks {
                                        if chunk.embed_policy == ChunkEmbedPolicy::Anchor {
                                            let pc = chunk_to_pending(
                                                chunk,
                                                &record.path,
                                                record.title.as_deref().unwrap_or(""),
                                                record.is_code,
                                            );
                                            if tx.send(pc).is_err() {
                                                break;
                                            }
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
                    if let Some(ref mut vi) = self.vector_index_mut() {
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
                        if let Some(ref mut vi) = self.vector_index_mut() {
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

        if let Some(mut pipeline) = embedding_pipeline {
            if let Some(ref mut vi) = self.vector_index_mut() {
                pipeline.finish(vi)?;
            }
        }

        if uncommitted_count > 0 {
            self.commit_intermediate()?;
        }

        if !tag_configs.is_empty() && !all_docs.is_empty() {
            self.graph.build_all_tag_edges(&tag_configs, &all_docs);
        }

        let _ = self.resolve_cross_file_code_edges();

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

    /// Commit all pending changes across all registered retrieval algorithms and SQLite.
    pub fn commit(&mut self) -> Result<()> {
        let _ = self.store.commit_batch();
        let edge_records = self.graph.graph().get_all_edge_records();
        self.store.clear_all_edges()?;
        self.store.insert_edges(&edge_records)?;
        self.graph.save(&self.index_dir.join("graph.bin"))?;
        if let Some(ref vi) = self.vector_index() {
            if vi.is_dirty() && !vi.is_empty() {
                let _ = vi.save_binary(&self.index_dir.join("vectors.bin"));
            }
        }
        if !self.binary.is_empty() {
            let _ = self.binary.save_to_path(&self.index_dir.join("fingerprints.bin"));
        }
        self.commit_algorithms()?;
        Ok(())
    }

    /// Read the exact UTF-8 text slice of a chunk directly from the source file on disk.
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

    /// Re-embed all chunks with the current model, replacing old vectors.
    pub fn reembed(&mut self) -> Result<usize> {
        if self.is_fast_mode() || self.dense.is_none() {
            return Err(Error::Index(
                "re-embedding is unavailable in fast mode. Re-index with index_mode = 'full'"
                    .to_string(),
            ));
        }

        let available = self.ensure_embedder()?;
        if !available {
            return Err(Error::Index("embedder not available — cannot re-embed".to_string()));
        }
        let embedder = self.embedder_arc().unwrap();

        let files = self.store.list_files()?;
        if let Some(ref mut dense) = self.dense {
            dense.clear()?;
        }

        let mut total_chunks = 0usize;
        let mut chunk_buffer: Vec<PendingChunk> = Vec::new();

        let corpus_path = std::path::PathBuf::from(&self.config.path);
        for file in &files {
            let is_code = crate::parser::code::is_code_file(std::path::Path::new(&file.path));
            if is_code {
                continue;
            }

            let full_path = corpus_path.join(&file.path);
            let parsed_chunks: Option<(Vec<groundcontrol_common::types::Chunk>, Option<String>)> =
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
                let mut chunks: Vec<groundcontrol_common::types::Chunk> =
                    Vec::with_capacity(chunk_records.len());
                for cr in chunk_records {
                    let text = self
                        .fetch_chunk_text(&file.path, cr.start_byte, cr.end_byte)
                        .unwrap_or_default();
                    chunks.push(
                        groundcontrol_common::types::Chunk::new(
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
                    .map(groundcontrol_common::types::EntityKind::modality_tag)
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

        if !chunk_buffer.is_empty() {
            self.flush_chunk_buffer(&chunk_buffer)?;
            chunk_buffer.clear();
        }

        let model_version = embedder.model_name().version_string().to_string();
        if let Some(ref mut vi) = self.vector_index_mut() {
            vi.set_model_version(&model_version);
            vi.clear_stale();
        }

        self.store.set_config("embedding_model", &model_version)?;
        self.commit()?;

        info!(
            "Re-embedding complete: {} chunks re-embedded with model '{}'",
            total_chunks, model_version
        );

        Ok(total_chunks)
    }
}

/// Parse a single file (polyglot code or markdown) in a thread-safe, lock-free manner.
pub(crate) fn parse_file_record(
    rel_path: &str,
    full_path: &Path,
    bytes: &[u8],
    hash: String,
    classifier: &crate::index::classifier::FileClassifier,
    chunking_config: &ChunkingConfig,
    _index_mode: IndexMode,
) -> Result<ParsedArtifact> {
    let classification = classifier.classify(full_path, Some(bytes));

    match classification {
        crate::index::classifier::FileClassification::Code(_) => {
            let content = String::from_utf8_lossy(bytes).into_owned();
            let path = Path::new(rel_path);
            let file_title = path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string());
            let parse_res = crate::parser::code::chunker::CodeChunker::parse_and_chunk(
                path,
                &content,
                chunking_config,
            );

            let mut raw_chunks = Vec::new();
            let mut symbols = Vec::new();
            let mut grammar_semantics = Vec::new();
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
                grammar_semantics = res.grammar_semantics;
            }

            Ok(ParsedArtifact {
                path: rel_path.to_string(),
                hash,
                is_code: true,
                format: FileFormat::Source,
                title: file_title,
                doc_metadata: None,
                symbols,
                grammar_semantics,
                chunks: raw_chunks,
                graph_edges,
                external_refs,
                raw_content: Some(content),
                projection_text: None,
            })
        }
        crate::index::classifier::FileClassification::MarkdownDoc => {
            let content = String::from_utf8_lossy(bytes).into_owned();
            let path = Path::new(rel_path);
            let doc = crate::parser::parse_document(path, &content)?;
            let chunks =
                crate::parser::chunker::chunk_document(rel_path, &doc.content, chunking_config);

            let title = doc.title.clone();
            Ok(ParsedArtifact {
                path: rel_path.to_string(),
                hash,
                is_code: false,
                format: FileFormat::Source,
                title,
                doc_metadata: Some(doc),
                symbols: Vec::new(),
                grammar_semantics: Vec::new(),
                chunks,
                graph_edges: Vec::new(),
                external_refs: Vec::new(),
                raw_content: Some(content),
                projection_text: None,
            })
        }
        crate::index::classifier::FileClassification::Document(fmt) => {
            let registry = crate::parser::document::DocumentExtractorRegistry::new();
            let extracted = registry.extract(full_path, fmt, bytes)?;
            let chunks = crate::parser::chunker::chunk_document(
                rel_path,
                &extracted.normalized_text,
                chunking_config,
            );

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
                graph_edges.push(groundcontrol_common::types::Edge {
                    source: rel_path.to_string(),
                    target: link.target,
                    edge_type: "references".to_string(),
                    weight: 0.8,
                    provenance: groundcontrol_common::types::EdgeProvenance::MarkdownLink,
                    target_corpus: None,
                    confidence: Some(groundcontrol_common::types::ResolutionConfidence::High),
                    target_path: None,
                    target_symbol: None,
                    target_kind: None,
                });
            }

            let title = extracted.title;
            Ok(ParsedArtifact {
                path: rel_path.to_string(),
                hash,
                is_code: false,
                format: fmt,
                title,
                doc_metadata: Some(doc),
                symbols: Vec::new(),
                grammar_semantics: Vec::new(),
                chunks,
                graph_edges,
                external_refs: Vec::new(),
                raw_content: Some(extracted.normalized_text.clone()),
                projection_text: Some(extracted.normalized_text),
            })
        }
        crate::index::classifier::FileClassification::Ignored => Err(Error::Parse {
            path: rel_path.to_string(),
            message: format!("file '{rel_path}' is ignored or unsupported format"),
        }),
    }
}
