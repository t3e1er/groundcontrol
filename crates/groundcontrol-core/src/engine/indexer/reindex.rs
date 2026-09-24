//! Full repository reindexing, worker loop, batching, and state resumption.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use groundcontrol_common::config::{EdgeSource, IndexMode};
use groundcontrol_common::types::{
    ChunkEmbedPolicy, Document, IndexingState, IndexingStatus, ParsedArtifact,
};
use groundcontrol_common::Result;

use crate::engine::state::Engine;
use crate::engine::types::{now_unix, walk_markdown_files};
use crate::index::pipeline::AsyncEmbeddingPipeline;

use super::stage_parse::{chunk_to_pending, parse_file_record};

impl Engine {
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
        if self.config.index_mode != IndexMode::Fast {
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
            .filter(|et| et.source == EdgeSource::Tag)
            .cloned()
            .collect();
        let mut all_docs: Vec<Document> = Vec::new();
        let embedding_pipeline = if self.config.index_mode == IndexMode::Fast {
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
}
