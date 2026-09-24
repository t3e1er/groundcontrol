//! File ingestion, chunk flushing, and removal primitives.

use std::fs;
use std::path::Path;

use tracing::{debug, warn};

use groundcontrol_common::config::{EdgeClass, EdgeSource, IndexMode};
use groundcontrol_common::types::{ChunkEmbedPolicy, ChunkRecord, Document, ParsedArtifact};
use groundcontrol_common::Result;

use crate::embedding::Embedder;
use crate::engine::state::Engine;
use crate::engine::types::{now_unix, PendingChunk};

use super::stage_parse::{chunk_to_pending, parse_file_record};

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
            .filter(|et| et.source == EdgeSource::Tag)
            .cloned()
            .collect();

        let pending = if self.config.index_mode == IndexMode::Full && !record.is_code {
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
                    EdgeClass::Structural,
                );
            }

            if !tag_configs.is_empty() && !doc.tags.is_empty() {
                doc.content.clear();
                doc.links.clear();
                all_docs.push(doc);
            }
        }

        Ok(())
    }
}
