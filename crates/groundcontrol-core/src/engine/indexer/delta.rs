//! Incremental delta scanning and selective path synchronization.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use groundcontrol_common::config::{EdgeSource, IndexMode};
use groundcontrol_common::types::{ChunkEmbedPolicy, Document, ParsedArtifact};
use groundcontrol_common::Result;

use crate::classifier::FileClassifier;
use crate::engine::state::Engine;
use crate::engine::types::{walk_markdown_files, DeltaScanResult};
use crate::index::pipeline::AsyncEmbeddingPipeline;

use super::stage_parse::{chunk_to_pending, parse_file_record};

impl Engine {
    /// Perform a full delta scan of the repository.
    pub fn delta_scan(&mut self) -> Result<DeltaScanResult> {
        self.delta_scan_paginated(50)
    }

    /// Perform a paginated delta scan: compare filesystem against stored file records.
    pub fn delta_scan_paginated(&mut self, batch_size: usize) -> Result<DeltaScanResult> {
        let commit_batch_size = if batch_size == 0 || batch_size == 50 { 500 } else { batch_size };
        if self.config.index_mode != IndexMode::Fast {
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
        let embedding_pipeline = if self.config.index_mode == IndexMode::Fast {
            None
        } else {
            self.embedder_arc().map(AsyncEmbeddingPipeline::new)
        };

        let tag_configs: Vec<_> = self
            .config
            .graph
            .edge_types
            .iter()
            .filter(|et| et.source == EdgeSource::Tag)
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
        if self.config.index_mode != IndexMode::Fast {
            let _ = self.ensure_embedder();
        }
        let embedding_pipeline = if self.config.index_mode == IndexMode::Fast {
            None
        } else {
            self.embedder_arc().map(AsyncEmbeddingPipeline::new)
        };

        let tag_configs: Vec<_> = self
            .config
            .graph
            .edge_types
            .iter()
            .filter(|et| et.source == EdgeSource::Tag)
            .cloned()
            .collect();
        let mut all_docs: Vec<Document> = Vec::new();
        let classifier = FileClassifier::new(&corpus_path, &self.config);

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
}
