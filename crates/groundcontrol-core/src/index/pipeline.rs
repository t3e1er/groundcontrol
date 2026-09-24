//! Asynchronous decoupled embedding pipeline.
//!
//! Decouples CPU document parsing, Tree-sitter AST traversal, Tantivy BM25 indexing,
//! and SQLite metadata persistence from GPU/DirectML tensor forward passes.
//!
//! Architecture:
//! 1. Producer Thread (`Engine` on main thread):
//!    - Discovers files, executes AST/markdown parsing, writes Tantivy BM25 and SQLite metadata.
//!    - Anchor chunks (`ChunkEmbedPolicy::Anchor`) are sent into a bounded channel (`chunk_tx`).
//!    - Never stalls waiting for GPU forward passes.
//!    - Drains completed embeddings without blocking via [`AsyncEmbeddingPipeline::try_recv_completed`].
//! 2. Double-Buffered Staging & Tokenization Prefetch Worker (CPU thread):
//!    - Coalesces incoming anchor chunks into saturated batches (64–128 chunks).
//!    - Pre-tokenizes text on CPU using HuggingFace BPE tokenizer.
//!    - Packs and pads tensors into contiguous flat host arrays (`input_ids`, `attention_mask`).
//!    - Dispatches [`StagedBatch`] to a double-buffered channel (`staged_tx`, capacity 2).
//! 3. Dedicated GPU Inference Worker (DirectML execution thread):
//!    - Pulls pre-staged batches from the `staged_rx` channel receiver.
//!    - Dispatches contiguous GEMM command lists via DirectML ONNX Runtime session.
//!    - Performs average pooling and L2 normalization on GPU hidden state tensors.
//!    - Emits [`CompletedBatch`] to `completed_tx`.
//! 4. Lock-Free Vector Insertion:
//!    - `VectorIndex` remains exclusively owned on the main thread with zero mutex locks.
//!    - Completed batches are applied incrementally at file loop boundaries and committed at shutdown.

use std::{
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::{bounded, unbounded, Receiver, RecvTimeoutError, Sender};

use crate::{embedding::Embedder, engine::PendingChunk, vector_index::VectorIndex};

use groundcontrol_common::{types::Edge, Error, Result};

/// Structural graph edge produced during AST code extraction or markdown linking.
pub type ASTEdge = Edge;

pub use groundcontrol_common::types::ParsedArtifact;

/// A pre-tokenized and padded batch of chunks staged in contiguous host memory arrays.
pub struct StagedBatch {
    /// Original pending chunks for indexing metadata.
    pub chunks: Vec<PendingChunk>,
    /// Number of sequences in the batch.
    pub batch_size: usize,
    /// Maximum sequence length in the batch.
    pub max_len: usize,
    /// Flattened input IDs array: `[batch_size * max_len]`.
    pub flat_input_ids: Vec<i64>,
    /// Flattened attention mask array: `[batch_size * max_len]`.
    pub flat_attention_mask: Vec<i64>,
    /// Optional flattened token type IDs array: `[batch_size * max_len]`.
    pub flat_token_type_ids: Option<Vec<i64>>,
}

/// A completed batch of embeddings ready to be applied to the vector index on the main thread.
pub struct CompletedBatch {
    /// Original pending chunks.
    pub chunks: Vec<PendingChunk>,
    /// L2-normalized embedding vectors matching each chunk.
    pub embeddings: Vec<Vec<f32>>,
}

/// Asynchronous multi-stage GPU embedding pipeline.
pub struct AsyncEmbeddingPipeline {
    chunk_tx: Option<Sender<PendingChunk>>,
    completed_rx: Receiver<CompletedBatch>,
    prefetch_handle: Option<JoinHandle<()>>,
    gpu_handles: Vec<JoinHandle<()>>,
}

impl AsyncEmbeddingPipeline {
    /// Initialize and launch the asynchronous embedding pipeline worker threads.
    pub fn new(embedder: Arc<Embedder>) -> Self {
        // Stage A -> Stage B Prefetch channel: high-watermark bounded queue (capacity 4,096)
        let (chunk_tx, chunk_rx) = bounded::<PendingChunk>(4096);

        // Stage B Prefetch -> GPU channel: bounded staging ring buffer (capacity 32 batches)
        let (staged_tx, staged_rx) = bounded::<StagedBatch>(32);

        // GPU -> Stage C completed channel: unbounded so GPU never stalls on completion
        let (completed_tx, completed_rx) = unbounded::<CompletedBatch>();

        let embedder_for_prefetch = Arc::clone(&embedder);
        let max_chunks_per_batch = embedder.governor().compute_adaptive_batch(256, 0).max(32);

        // 1. Launch Tokenization Prefetch Worker Thread
        let prefetch_handle = thread::Builder::new()
            .name("groundcontrol-prefetch".to_string())
            .spawn(move || {
                let mut accumulator: Vec<PendingChunk> = Vec::with_capacity(max_chunks_per_batch);

                loop {
                    // Accumulate chunks with a short timeout to prevent latency on batch boundaries
                    match chunk_rx.recv_timeout(Duration::from_millis(5)) {
                        Ok(chunk) => {
                            accumulator.push(chunk);
                            let target_cap = embedder_for_prefetch
                                .governor()
                                .compute_adaptive_batch(256, 0)
                                .max(16);
                            if accumulator.len() >= target_cap {
                                let batches = Self::stage_batches(
                                    std::mem::replace(
                                        &mut accumulator,
                                        Vec::with_capacity(target_cap),
                                    ),
                                    &embedder_for_prefetch,
                                );
                                let mut disconnected = false;
                                for staged in batches {
                                    if staged_tx.send(staged).is_err() {
                                        disconnected = true;
                                        break;
                                    }
                                }
                                if disconnected {
                                    break;
                                }
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if !accumulator.is_empty() {
                                let target_cap = embedder_for_prefetch
                                    .governor()
                                    .compute_adaptive_batch(256, 0)
                                    .max(16);
                                let batches = Self::stage_batches(
                                    std::mem::replace(
                                        &mut accumulator,
                                        Vec::with_capacity(target_cap),
                                    ),
                                    &embedder_for_prefetch,
                                );
                                let mut disconnected = false;
                                for staged in batches {
                                    if staged_tx.send(staged).is_err() {
                                        disconnected = true;
                                        break;
                                    }
                                }
                                if disconnected {
                                    break;
                                }
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            // Producer has finished sending; stage remaining chunks and exit
                            if !accumulator.is_empty() {
                                let batches =
                                    Self::stage_batches(accumulator, &embedder_for_prefetch);
                                for staged in batches {
                                    let _ = staged_tx.send(staged);
                                }
                            }
                            break;
                        }
                    }
                }
            })
            .expect("failed to spawn prefetch worker thread");

        // 2. Launch Dedicated GPU Inference Worker Threads
        // Windows DirectML requires serialized single-stream command submission to avoid DXGI device resets.
        #[cfg(target_os = "windows")]
        let num_workers = 1;
        #[cfg(not(target_os = "windows"))]
        let num_workers = if embedder.session_count() >= 2
            && embedder.governor().total_memory_bytes() >= 4 * 1024 * 1024 * 1024
        {
            2
        } else {
            1
        };

        let mut gpu_handles = Vec::with_capacity(num_workers);

        for session_index in 0..num_workers {
            let embedder_for_gpu = Arc::clone(&embedder);
            let staged_rx_clone = staged_rx.clone();
            let completed_tx_clone = completed_tx.clone();

            let handle = thread::Builder::new()
                .name(format!("groundcontrol-gpu-engine-{session_index}"))
                .spawn(move || {
                    let mut next_batch: Option<StagedBatch> = None;
                    loop {
                        let current = match next_batch.take() {
                            Some(b) => b,
                            None => match staged_rx_clone.recv() {
                                Ok(b) => b,
                                Err(_) => break, // Channel closed and drained
                            },
                        };

                        // Strategy 1: Pre-fetch Batch N+1 via non-blocking try_recv while Batch N is executing
                        if let Ok(b) = staged_rx_clone.try_recv() {
                            next_batch = Some(b);
                        }

                        let chunks = current.chunks;
                        let seq_len = current.max_len;
                        let start_instant = std::time::Instant::now();

                        match embedder_for_gpu.run_staged_tensor_batch(
                            session_index,
                            current.batch_size,
                            current.max_len,
                            current.flat_input_ids,
                            current.flat_attention_mask,
                            current.flat_token_type_ids,
                        ) {
                            Ok(embeddings) => {
                                let elapsed_ms = start_instant.elapsed().as_millis() as u64;
                                // Wire AIMD feedback loop into governor
                                let _ = embedder_for_gpu
                                    .governor()
                                    .compute_adaptive_batch(seq_len, elapsed_ms);

                                if completed_tx_clone
                                    .send(CompletedBatch { chunks, embeddings })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    session_index,
                                    "GPU inference failed in pipeline: {e}"
                                );
                            }
                        }
                    }
                })
                .expect("failed to spawn GPU engine worker thread");

            gpu_handles.push(handle);
        }

        Self {
            chunk_tx: Some(chunk_tx),
            completed_rx,
            prefetch_handle: Some(prefetch_handle),
            gpu_handles,
        }
    }

    /// Helper to convert a batch of chunks into sub-batches of contiguous host arrays
    /// using Sort-and-Pack length grouping and hardware-aware batch size caps.
    fn stage_batches(chunks: Vec<PendingChunk>, embedder: &Embedder) -> Vec<StagedBatch> {
        if chunks.is_empty() {
            return Vec::new();
        }

        let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        let encodings = match embedder.tokenizer().encode_batch(texts, true) {
            Ok(enc) => enc,
            Err(e) => {
                tracing::warn!("Tokenization failed for batch of {} chunks: {e}", chunks.len());
                return Vec::new();
            }
        };

        let num_chunks = chunks.len();
        let max_model_len = embedder.model_name().max_seq_len();

        // Sort indices by sequence length
        let mut sorted_indices: Vec<usize> = (0..num_chunks).collect();
        sorted_indices.sort_by_key(|&idx| encodings[idx].get_ids().len());

        let mut sub_batches: Vec<Vec<usize>> = Vec::new();
        let mut current_sub: Vec<usize> = Vec::new();

        for &idx in &sorted_indices {
            let seq_len = encodings[idx].get_ids().len().min(max_model_len).max(1);
            let max_allowed = embedder.governor().compute_adaptive_batch(seq_len, 0);

            if !current_sub.is_empty() && current_sub.len() >= max_allowed {
                sub_batches.push(current_sub);
                current_sub = Vec::new();
            }
            current_sub.push(idx);
        }
        if !current_sub.is_empty() {
            sub_batches.push(current_sub);
        }

        // Build StagedBatch for each sub-batch
        let mut staged_list = Vec::with_capacity(sub_batches.len());

        for sub_indices in sub_batches {
            let batch_size = sub_indices.len();
            let raw_max_len =
                sub_indices.iter().map(|&i| encodings[i].get_ids().len()).max().unwrap_or(1);
            let max_len = raw_max_len.min(max_model_len).max(1);

            let mut sub_chunks = Vec::with_capacity(batch_size);
            let mut flat_input_ids = Vec::with_capacity(batch_size * max_len);
            let mut flat_attention_mask = Vec::with_capacity(batch_size * max_len);
            let mut flat_token_type_ids = if embedder.has_token_type_ids() {
                Some(Vec::with_capacity(batch_size * max_len))
            } else {
                None
            };

            for &idx in &sub_indices {
                sub_chunks.push(chunks[idx].clone());
                let enc = &encodings[idx];
                let ids = enc.get_ids();
                let mask = enc.get_attention_mask();
                let cur_len = ids.len().min(max_len);

                for i in 0..cur_len {
                    flat_input_ids.push(ids[i] as i64);
                    flat_attention_mask.push(mask[i] as i64);
                }
                for _ in cur_len..max_len {
                    flat_input_ids.push(0i64);
                    flat_attention_mask.push(0i64);
                }

                if let Some(ref mut type_vec) = flat_token_type_ids {
                    let type_ids = enc.get_type_ids();
                    for i in 0..cur_len {
                        type_vec.push(type_ids[i] as i64);
                    }
                    for _ in cur_len..max_len {
                        type_vec.push(0i64);
                    }
                }
            }

            staged_list.push(StagedBatch {
                chunks: sub_chunks,
                batch_size,
                max_len,
                flat_input_ids,
                flat_attention_mask,
                flat_token_type_ids,
            });
        }

        staged_list
    }

    /// Send a single anchor chunk into the pipeline.
    pub fn send(&self, chunk: PendingChunk) -> Result<()> {
        if let Some(ref tx) = self.chunk_tx {
            tx.send(chunk).map_err(|e| Error::Index(format!("pipeline send error: {e}")))?;
        }
        Ok(())
    }

    /// Send a collection of anchor chunks into the pipeline.
    pub fn send_batch(&self, chunks: impl IntoIterator<Item = PendingChunk>) -> Result<()> {
        if let Some(ref tx) = self.chunk_tx {
            for chunk in chunks {
                tx.send(chunk).map_err(|e| Error::Index(format!("pipeline send error: {e}")))?;
            }
        }
        Ok(())
    }

    /// Get a clone of the channel sender for streaming chunks into the GPU prefetch intake.
    pub fn chunk_sender(&self) -> Option<Sender<PendingChunk>> {
        self.chunk_tx.clone()
    }

    /// Close the chunk intake channel so the prefetch worker can begin draining.
    pub fn close_intake(&mut self) {
        drop(self.chunk_tx.take());
    }

    /// Non-blocking drain: poll and insert any currently completed batches into the vector index.
    pub fn try_recv_completed(&self, vector_index: &mut VectorIndex) -> Result<usize> {
        let mut total_inserted = 0;
        while let Ok(completed) = self.completed_rx.try_recv() {
            total_inserted += Self::apply_completed_batch(completed, vector_index)?;
        }
        Ok(total_inserted)
    }

    /// Finish pipeline execution, drain remaining in-flight batches, join worker threads,
    /// and insert all remaining embeddings into the vector index.
    pub fn finish(&mut self, vector_index: &mut VectorIndex) -> Result<usize> {
        // 1. Close chunk_tx so prefetch worker terminates after draining
        self.close_intake();

        // 2. Wait for prefetch worker thread to terminate
        if let Some(handle) = self.prefetch_handle.take() {
            let _ = handle.join();
        }

        // 3. While GPU workers are processing staged batches, continuously drain completed batches
        let mut total_inserted = 0;
        let mut last_log = std::time::Instant::now();

        for handle in self.gpu_handles.drain(..) {
            while !handle.is_finished() {
                while let Ok(completed) = self.completed_rx.try_recv() {
                    total_inserted += Self::apply_completed_batch(completed, vector_index)?;
                }
                if last_log.elapsed() >= Duration::from_secs(5) {
                    tracing::info!(
                        total_inserted,
                        "Finishing embedding pipeline: waiting for GPU workers to drain queued batches..."
                    );
                    last_log = std::time::Instant::now();
                }
                thread::sleep(Duration::from_millis(20));
            }
            let _ = handle.join();
        }

        // 4. Final drain of completed_rx
        while let Ok(completed) = self.completed_rx.try_recv() {
            total_inserted += Self::apply_completed_batch(completed, vector_index)?;
        }

        tracing::info!(total_inserted, "Embedding pipeline cleanly finalized");
        Ok(total_inserted)
    }

    /// Helper to insert a completed batch into the vector index, grouping by document path.
    fn apply_completed_batch(
        completed: CompletedBatch,
        vector_index: &mut VectorIndex,
    ) -> Result<usize> {
        if completed.chunks.is_empty() || completed.embeddings.len() != completed.chunks.len() {
            return Ok(0);
        }

        let total_chunks = completed.chunks.len();
        let mut start = 0;

        while start < completed.chunks.len() {
            let doc_path = &completed.chunks[start].doc_path;
            let mut end = start + 1;
            while end < completed.chunks.len() && completed.chunks[end].doc_path == *doc_path {
                end += 1;
            }

            let file_chunks = &completed.chunks[start..end];
            let file_embeddings = &completed.embeddings[start..end];

            let chunk_indices: Vec<Option<usize>> =
                file_chunks.iter().map(|c| Some(c.chunk_index)).collect();
            // All chunks for a doc_path share the same file, hence the same modality.
            let modality = file_chunks[0].modality.as_str();

            vector_index.add_batch(file_embeddings, doc_path, &chunk_indices, false, modality)?;

            if let Some(doc_embedding) = Embedder::average_embeddings(file_embeddings) {
                vector_index.add(&doc_embedding, doc_path, None, true, modality)?;
            }

            start = end;
        }

        Ok(total_chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use groundcontrol_common::types::ChunkEmbedPolicy;

    #[test]
    fn test_pipeline_empty_finish() {
        let embedder = match Embedder::new_default() {
            Ok(e) => Arc::new(e),
            Err(_) => {
                eprintln!(
                    "Skipping test_pipeline_empty_finish: embedding model not downloaded locally"
                );
                return;
            }
        };
        let dimensions = embedder.dimensions();
        let mut pipeline = AsyncEmbeddingPipeline::new(embedder);
        let mut vector_index = VectorIndex::new(dimensions, 100, 64, 16);

        let inserted = pipeline.finish(&mut vector_index).expect("should finish cleanly");
        assert_eq!(inserted, 0);
        assert_eq!(vector_index.len(), 0);
    }

    #[test]
    fn test_pipeline_streaming_lifecycle() {
        let embedder = match Embedder::new_default() {
            Ok(e) => Arc::new(e),
            Err(_) => {
                eprintln!("Skipping test_pipeline_streaming_lifecycle: embedding model not downloaded locally");
                return;
            }
        };
        let dimensions = embedder.dimensions();
        let mut pipeline = AsyncEmbeddingPipeline::new(embedder);
        let mut vector_index = VectorIndex::new(dimensions, 100, 64, 16);

        // Send 6 anchor chunks across 2 documents
        for i in 0..3 {
            pipeline
                .send(PendingChunk {
                    doc_path: "doc_a.md".to_string(),
                    chunk_index: i,
                    text: format!(
                        "Document A section {i} describing architecture decisions and rules."
                    ),
                    embed_policy: ChunkEmbedPolicy::Anchor,
                    modality: "docs".to_string(),
                })
                .expect("send should succeed");
        }

        for i in 0..3 {
            pipeline
                .send(PendingChunk {
                    doc_path: "doc_b.md".to_string(),
                    chunk_index: i,
                    text: format!(
                        "Document B section {i} containing implementation guide and algorithms."
                    ),
                    embed_policy: ChunkEmbedPolicy::Anchor,
                    modality: "docs".to_string(),
                })
                .expect("send should succeed");
        }

        // Finish pipeline and assert all 6 chunks were processed
        let total_inserted =
            pipeline.finish(&mut vector_index).expect("pipeline should finish cleanly");
        assert_eq!(total_inserted, 6);

        // vector_index should have 6 chunk embeddings + 2 doc-level embeddings = 8 entries
        assert_eq!(vector_index.len(), 8);
    }
}
