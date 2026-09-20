//! Hardware-accelerated embedding generation via ONNX Runtime (`ort`) and `tokenizers`.
//!
//! Provides zero-configuration GPU hardware acceleration across all platforms:
//! - Windows: Microsoft DirectML over DirectX 12 Compute (NVIDIA GTX/RTX, AMD Radeon APU, Intel Arc).
//! - macOS: Apple CoreML / Metal Performance Shaders (Apple Silicon M1-M4).
//! - Linux / Docker: Pure-Rust SIMD AVX2/AVX-512 CPU fallback with multi-chunk batching.
//!
//! Includes dynamic VRAM-budget scheduling and sort-and-pack tokenization to prevent
//! GPU Out-of-Memory (OOM) errors and unrecoverable DirectX 12 device-lost states.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ctxvault_common::{Error, Result};

#[cfg(target_os = "macos")]
use ort::ep::CoreML;
#[cfg(target_os = "windows")]
use ort::ep::DirectML;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use tokenizers::Tokenizer;

/// Hardware governors and AIMD batch sizing controllers.
pub mod governor;
/// Embedding tensor forward pass and pooling algorithms.
pub mod inference;
/// Supported embedding model specifications and sidecar resolution.
pub mod model;

#[cfg(target_os = "macos")]
pub use governor::CoreMlGovernor;
pub use governor::{default_hardware_governor, AimdController, CpuGovernor, HardwareGovernor};
#[cfg(target_os = "windows")]
pub use governor::{
    detect_gpu_vram_mb, directml_device_candidates, select_directml_device_id, DirectMlGovernor,
};
pub use inference::average_embeddings;
pub use model::ModelName;

use model::resolve_model_files;

#[cfg(test)]
mod tests;

/// Embedder wraps ONNX Runtime hardware-accelerated sessions for batch embedding generation.
pub struct Embedder {
    sessions: Vec<Mutex<Session>>,
    cpu_session: Option<Mutex<Session>>,
    tokenizer: Tokenizer,
    model_name: ModelName,
    governor: Arc<dyn HardwareGovernor>,
    has_token_type_ids: bool,
    gpu_disabled: AtomicBool,
}

impl Embedder {
    /// Create a new embedder with the specified model and hardware governor.
    pub fn new(model_name: ModelName, governor: Arc<dyn HardwareGovernor>) -> Result<Self> {
        let (model_path, tokenizer_path) = resolve_model_files(&model_name)?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            Error::Index(format!("failed to load tokenizer from {}: {e}", tokenizer_path.display()))
        })?;

        let create_builder =
            |_target_device: Option<i32>| -> Result<ort::session::builder::SessionBuilder> {
                // `mut` is only needed on platforms that reassign `builder` to attach a
                // hardware execution provider below (Windows/DirectML, macOS/CoreML).
                #[cfg(any(target_os = "windows", target_os = "macos"))]
                let mut builder = Session::builder()
                    .map_err(|e| Error::Index(format!("failed to create session builder: {e}")))?
                    .with_optimization_level(GraphOptimizationLevel::Level1)
                    .map_err(|e| {
                        Error::Index(format!("failed to set graph optimization level: {e}"))
                    })?;
                #[cfg(not(any(target_os = "windows", target_os = "macos")))]
                let builder = Session::builder()
                    .map_err(|e| Error::Index(format!("failed to create session builder: {e}")))?
                    .with_optimization_level(GraphOptimizationLevel::Level1)
                    .map_err(|e| {
                        Error::Index(format!("failed to set graph optimization level: {e}"))
                    })?;

                #[cfg(target_os = "windows")]
                {
                    if let Some(device_id) = _target_device {
                        builder = builder
                            .with_execution_providers([DirectML::default()
                                .with_device_id(device_id)
                                .build()])
                            .map_err(|e| {
                                Error::Index(format!("failed to configure DirectML provider: {e}"))
                            })?;
                    }
                }

                #[cfg(target_os = "macos")]
                {
                    builder =
                        builder.with_execution_providers([CoreML::default().build()]).map_err(
                            |e| Error::Index(format!("failed to configure CoreML provider: {e}")),
                        )?;
                }

                Ok(builder)
            };

        #[cfg(target_os = "windows")]
        let (session_0, active_device_id) = {
            let candidates = directml_device_candidates();
            let mut last_err = None;
            let mut result = None;

            for &cand_id in &candidates {
                match create_builder(Some(cand_id))?.commit_from_file(&model_path) {
                    Ok(sess) => {
                        result = Some((sess, Some(cand_id)));
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(
                            device_id = cand_id,
                            error = %e,
                            "Failed to initialize DirectML session on candidate adapter; attempting next candidate"
                        );
                        last_err = Some(e);
                    }
                }
            }

            match result {
                Some(pair) => pair,
                None => {
                    tracing::warn!(
                        "All DirectML candidate adapters failed; falling back to CPU session"
                    );
                    let sess =
                        create_builder(None)?.commit_from_file(&model_path).map_err(|e| {
                            Error::Index(format!(
                                "failed to load ONNX model from {}: {e} (last GPU error: {:?})",
                                model_path.display(),
                                last_err
                            ))
                        })?;
                    (sess, None)
                }
            }
        };

        #[cfg(not(target_os = "windows"))]
        let session_0 = create_builder(None)?.commit_from_file(&model_path).map_err(|e| {
            Error::Index(format!("failed to load ONNX model from {}: {e}", model_path.display()))
        })?;

        let has_token_type_ids = session_0.inputs().iter().any(|i| i.name() == "token_type_ids");

        let mut sessions = vec![Mutex::new(session_0)];

        // DirectML on Windows requires serialized command submission to avoid DXGI device resets / TDR.
        // Multi-stream concurrent sessions are enabled on platforms with native multi-stream compute (macOS Metal).
        let has_sufficient_vram = governor.total_memory_bytes() >= 4 * 1024 * 1024 * 1024;
        let can_use_dual_stream = cfg!(target_os = "macos") && has_sufficient_vram;

        if can_use_dual_stream {
            #[cfg(target_os = "windows")]
            let dual_builder = create_builder(active_device_id);
            #[cfg(not(target_os = "windows"))]
            let dual_builder = create_builder(None);

            match dual_builder.and_then(|mut b| {
                b.commit_from_file(&model_path).map_err(|e| {
                    Error::Index(format!(
                        "failed to load dual ONNX session from {}: {e}",
                        model_path.display()
                    ))
                })
            }) {
                Ok(session_1) => {
                    tracing::info!("Dual hardware acceleration sessions initialized for concurrent inference streams");
                    sessions.push(Mutex::new(session_1));
                }
                Err(e) => {
                    tracing::warn!("Failed to initialize second concurrent session ({e}), continuing with single session");
                }
            }
        }

        // Prepare a resilient CPU fallback session
        let cpu_session = {
            let mut cpu_builder = Session::builder()
                .map_err(|e| Error::Index(format!("failed to create CPU session builder: {e}")))?
                .with_optimization_level(GraphOptimizationLevel::Level1)
                .map_err(|e| Error::Index(format!("failed to set CPU optimization level: {e}")))?;
            match cpu_builder.commit_from_file(&model_path) {
                Ok(sess) => Some(Mutex::new(sess)),
                Err(e) => {
                    tracing::warn!("Failed to initialize CPU fallback session: {e}");
                    None
                }
            }
        };

        tracing::info!(
            model = %model_name.version_string(),
            dimensions = model_name.dimensions(),
            token_type_ids = has_token_type_ids,
            sessions = sessions.len(),
            provider = governor.provider_name(),
            "embedder initialized with hardware acceleration and dynamic activation governor"
        );

        Ok(Self {
            sessions,
            cpu_session,
            tokenizer,
            model_name,
            governor,
            has_token_type_ids,
            gpu_disabled: AtomicBool::new(false),
        })
    }

    /// Create an embedder from a config model string (e.g., "jinaai/jina-embeddings-v2-base-code").
    pub fn from_config(model_str: &str) -> Result<Self> {
        let model_name = ModelName::from_str_name(model_str).unwrap_or_default();
        Self::new(model_name, default_hardware_governor())
    }

    /// Create an embedder with the default model and default platform hardware governor.
    pub fn new_default() -> Result<Self> {
        Self::new(ModelName::default(), default_hardware_governor())
    }

    /// Get the output dimensions of this embedder.
    pub fn dimensions(&self) -> usize {
        self.model_name.dimensions()
    }

    /// Get the model name.
    pub fn model_name(&self) -> &ModelName {
        &self.model_name
    }

    /// Get the hardware governor.
    pub fn governor(&self) -> &Arc<dyn HardwareGovernor> {
        &self.governor
    }

    /// Number of concurrent inference sessions available.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Check if hardware GPU acceleration is currently disabled (due to device lost / fallback).
    pub fn is_gpu_disabled(&self) -> bool {
        self.gpu_disabled.load(Ordering::Relaxed)
    }

    /// Reset GPU disabled state to re-attempt hardware acceleration.
    pub fn reset_gpu_disabled(&self) {
        self.gpu_disabled.store(false, Ordering::SeqCst);
    }

    /// Get a reference to the HuggingFace BPE tokenizer.
    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    /// Whether this model requires token type IDs.
    pub fn has_token_type_ids(&self) -> bool {
        self.has_token_type_ids
    }

    /// Embed a batch of text strings respecting dynamic VRAM budget and sort-and-pack tokenization.
    ///
    /// Returns one L2-normalized embedding vector per input string, in the exact order of `texts`.
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Tokenize all texts upfront
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| Error::Index(format!("tokenization failed: {e}")))?;

        let num_texts = encodings.len();
        if num_texts == 1 {
            return self.embed_encoded_sub_batch(&[&encodings[0]]);
        }

        // 2. Sort-and-Pack: sort indices by sequence length
        let max_model_len = self.model_name.max_seq_len();
        let mut sorted_indices: Vec<usize> = (0..num_texts).collect();
        sorted_indices.sort_by_key(|&idx| encodings[idx].get_ids().len());

        let mut results: Vec<Vec<f32>> = vec![Vec::new(); num_texts];
        let mut current_batch_indices: Vec<usize> = Vec::new();

        for &idx in &sorted_indices {
            let seq_len = encodings[idx].get_ids().len().min(max_model_len).max(1);
            let max_allowed_batch = self.governor.compute_adaptive_batch(seq_len, 0);

            if !current_batch_indices.is_empty() && current_batch_indices.len() >= max_allowed_batch
            {
                // Flush current sub-batch
                let sub_batch_encodings: Vec<&tokenizers::Encoding> =
                    current_batch_indices.iter().map(|&i| &encodings[i]).collect();
                let sub_embeddings = self.embed_encoded_sub_batch(&sub_batch_encodings)?;
                for (sub_i, &orig_idx) in current_batch_indices.iter().enumerate() {
                    results[orig_idx] = sub_embeddings[sub_i].clone();
                }
                current_batch_indices.clear();
            }

            current_batch_indices.push(idx);
        }

        // Flush any remaining items
        if !current_batch_indices.is_empty() {
            let sub_batch_encodings: Vec<&tokenizers::Encoding> =
                current_batch_indices.iter().map(|&i| &encodings[i]).collect();
            let sub_embeddings = self.embed_encoded_sub_batch(&sub_batch_encodings)?;
            for (sub_i, &orig_idx) in current_batch_indices.iter().enumerate() {
                results[orig_idx] = sub_embeddings[sub_i].clone();
            }
        }

        Ok(results)
    }

    /// Embed a pre-tokenized sub-batch in a single tensor forward pass.
    fn embed_encoded_sub_batch(
        &self,
        encodings: &[&tokenizers::Encoding],
    ) -> Result<Vec<Vec<f32>>> {
        if encodings.is_empty() {
            return Ok(Vec::new());
        }

        let batch_size = encodings.len();
        let max_model_len = self.model_name.max_seq_len();
        let raw_max_len = encodings.iter().map(|e| e.get_ids().len()).max().unwrap_or(1);
        let max_len = raw_max_len.min(max_model_len).max(1);

        let mut flat_input_ids = Vec::with_capacity(batch_size * max_len);
        let mut flat_attention_mask = Vec::with_capacity(batch_size * max_len);
        let mut flat_token_type_ids = if self.has_token_type_ids {
            Some(Vec::with_capacity(batch_size * max_len))
        } else {
            None
        };

        for enc in encodings {
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            let type_ids = enc.get_type_ids();

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
                for i in 0..cur_len {
                    type_vec.push(type_ids[i] as i64);
                }
                for _ in cur_len..max_len {
                    type_vec.push(0i64);
                }
            }
        }

        self.run_staged_tensor_batch(
            0,
            batch_size,
            max_len,
            flat_input_ids,
            flat_attention_mask,
            flat_token_type_ids,
        )
    }

    /// Execute a tensor forward pass on pre-staged contiguous flat arrays.
    pub(crate) fn run_staged_tensor_batch(
        &self,
        session_index: usize,
        batch_size: usize,
        max_len: usize,
        flat_input_ids: Vec<i64>,
        flat_attention_mask: Vec<i64>,
        flat_token_type_ids: Option<Vec<i64>>,
    ) -> Result<Vec<Vec<f32>>> {
        if flat_input_ids.is_empty() || batch_size == 0 || max_len == 0 {
            return Ok(Vec::new());
        }

        let use_hardware = !self.gpu_disabled.load(Ordering::Relaxed);
        let mut hw_error: Option<String> = None;
        let sub_start = std::time::Instant::now();

        if use_hardware {
            let session_idx = session_index % self.sessions.len();
            let mut session_guard = self.sessions[session_idx]
                .lock()
                .map_err(|e| Error::Index(format!("session {session_idx} lock poisoned: {e}")))?;

            let run_result = if let Some(ref type_ids) = flat_token_type_ids {
                let input_ids_val =
                    ort::value::Tensor::from_array(([batch_size, max_len], flat_input_ids.clone()))
                        .map_err(|e| {
                            Error::Index(format!("failed to construct input_ids tensor: {e}"))
                        })?;
                let attention_mask_val = ort::value::Tensor::from_array((
                    [batch_size, max_len],
                    flat_attention_mask.clone(),
                ))
                .map_err(|e| {
                    Error::Index(format!("failed to construct attention_mask tensor: {e}"))
                })?;
                let token_type_ids_val =
                    ort::value::Tensor::from_array(([batch_size, max_len], type_ids.clone()))
                        .map_err(|e| {
                            Error::Index(format!("failed to construct token_type_ids tensor: {e}"))
                        })?;

                session_guard.run(ort::inputs![
                    "input_ids" => input_ids_val,
                    "attention_mask" => attention_mask_val,
                    "token_type_ids" => token_type_ids_val,
                ])
            } else {
                let input_ids_val =
                    ort::value::Tensor::from_array(([batch_size, max_len], flat_input_ids.clone()))
                        .map_err(|e| {
                            Error::Index(format!("failed to construct input_ids tensor: {e}"))
                        })?;
                let attention_mask_val = ort::value::Tensor::from_array((
                    [batch_size, max_len],
                    flat_attention_mask.clone(),
                ))
                .map_err(|e| {
                    Error::Index(format!("failed to construct attention_mask tensor: {e}"))
                })?;

                session_guard.run(ort::inputs![
                    "input_ids" => input_ids_val,
                    "attention_mask" => attention_mask_val,
                ])
            };

            match run_result {
                Ok(outputs) => match outputs["last_hidden_state"].try_extract_tensor::<f32>() {
                    Ok(hidden_tensor) => {
                        let pool_res = inference::pool_embeddings(
                            batch_size,
                            max_len,
                            self.model_name.dimensions(),
                            &flat_attention_mask,
                            hidden_tensor.1,
                        );
                        tracing::debug!(session_idx, batch_size, max_len, elapsed = ?sub_start.elapsed(), "Sub-batch [GPU] completed");
                        return Ok(pool_res);
                    }
                    Err(e) => {
                        hw_error = Some(format!("failed to extract last_hidden_state tensor: {e}"));
                    }
                },
                Err(e) => {
                    hw_error =
                        Some(format!("hardware forward pass failed on session {session_idx}: {e}"));
                }
            }

            // Fallback triggered: disable GPU acceleration for remaining batches to prevent cascading timeouts
            self.gpu_disabled.store(true, Ordering::SeqCst);
            tracing::warn!(
                error = %hw_error.as_deref().unwrap_or("unknown"),
                "DirectML hardware forward pass failed; disabling GPU acceleration and activating CPU fallback"
            );
        }

        // Fall back to CPU session
        if let Some(ref cpu_session_mutex) = self.cpu_session {
            if let Some(ref err) = hw_error {
                tracing::warn!(
                    "Hardware forward pass failed ({err}); executing on CPU fallback session"
                );
            }

            let mut cpu_session = cpu_session_mutex
                .lock()
                .map_err(|e| Error::Index(format!("cpu session lock poisoned: {e}")))?;

            let input_ids_val =
                ort::value::Tensor::from_array(([batch_size, max_len], flat_input_ids)).map_err(
                    |e| Error::Index(format!("failed to construct input_ids tensor: {e}")),
                )?;
            let attention_mask_val = ort::value::Tensor::from_array((
                [batch_size, max_len],
                flat_attention_mask.clone(),
            ))
            .map_err(|e| Error::Index(format!("failed to construct attention_mask tensor: {e}")))?;

            let outputs = if let Some(type_ids) = flat_token_type_ids {
                let token_type_ids_val =
                    ort::value::Tensor::from_array(([batch_size, max_len], type_ids)).map_err(
                        |e| Error::Index(format!("failed to construct token_type_ids tensor: {e}")),
                    )?;

                cpu_session
                    .run(ort::inputs![
                        "input_ids" => input_ids_val,
                        "attention_mask" => attention_mask_val,
                        "token_type_ids" => token_type_ids_val,
                    ])
                    .map_err(|e| Error::Index(format!("CPU fallback forward pass failed: {e}")))?
            } else {
                cpu_session
                    .run(ort::inputs![
                        "input_ids" => input_ids_val,
                        "attention_mask" => attention_mask_val,
                    ])
                    .map_err(|e| Error::Index(format!("CPU fallback forward pass failed: {e}")))?
            };

            let hidden_tensor = outputs["last_hidden_state"]
                .try_extract_tensor::<f32>()
                .map_err(|e| Error::Index(format!("failed to extract last_hidden_state: {e}")))?;

            let pool_res = inference::pool_embeddings(
                batch_size,
                max_len,
                self.model_name.dimensions(),
                &flat_attention_mask,
                hidden_tensor.1,
            );
            tracing::debug!(batch_size, max_len, elapsed = ?sub_start.elapsed(), "Sub-batch [CPU] completed");
            return Ok(pool_res);
        }

        Err(Error::Index(format!(
            "forward pass failed and no CPU fallback available: {}",
            hw_error.unwrap_or_else(|| "hardware disabled".to_string())
        )))
    }

    /// Embed a single text string.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.embed_batch(&[text])?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| Error::Index("embedding returned no results".to_string()))
    }

    /// Compute a document-level embedding by averaging chunk embeddings.
    pub fn average_embeddings(embeddings: &[Vec<f32>]) -> Option<Vec<f32>> {
        inference::average_embeddings(embeddings)
    }

    /// Embed a search query string.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        self.embed(query)
    }
}

impl ctxvault_common::ports::EmbeddingProvider for Embedder {
    fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        Embedder::embed_query(self, query)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Embedder::embed_batch(self, texts)
    }

    fn dimensions(&self) -> usize {
        Embedder::dimensions(self)
    }
}
