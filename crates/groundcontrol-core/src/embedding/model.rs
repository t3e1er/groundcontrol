use std::path::{Path, PathBuf};

use groundcontrol_common::{Error, Result};

/// Supported embedding model names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelName {
    /// jinaai/jina-embeddings-v2-base-code (768 dimensions, 8192 token window, code + NL, INT8 dynamic quantization).
    JinaEmbeddingsV2BaseCode,
}

impl ModelName {
    /// Get output dimensions for this model.
    pub fn dimensions(&self) -> usize {
        768
    }

    /// Parse a model name string into a `ModelName`.
    pub fn from_str_name(s: &str) -> Option<Self> {
        let lower = s.to_lowercase();
        let name = if let Some(idx) = lower.find('/') { &lower[idx + 1..] } else { &lower };
        match name {
            "jina-embeddings-v2-base-code"
            | "jina-embeddings-v2-base-code-int8"
            | "jina-code-int8"
            | "jina-code"
            | "jina" => Some(Self::JinaEmbeddingsV2BaseCode),
            _ if lower.contains("jina") => Some(Self::JinaEmbeddingsV2BaseCode),
            _ => None,
        }
    }

    /// Get the canonical version string for this model.
    pub fn version_string(&self) -> &'static str {
        "jina-embeddings-v2-base-code-int8"
    }

    /// Directory name for sidecar model storage.
    pub fn model_dir_name(&self) -> &'static str {
        "jina-embeddings-v2-base-code"
    }

    /// Candidate ONNX subpaths to probe within a model directory.
    pub fn onnx_candidate_subpaths(&self) -> &[&'static str] {
        &["onnx/model_quantized.onnx"]
    }

    /// Maximum context token sequence length.
    pub fn max_seq_len(&self) -> usize {
        1024
    }
}

impl Default for ModelName {
    fn default() -> Self {
        Self::JinaEmbeddingsV2BaseCode
    }
}

/// Helper to check whether a directory contains tokenizer.json and any of the candidate ONNX models.
pub(crate) fn check_directory_for_model(
    dir: &Path,
    candidate_subpaths: &[&'static str],
) -> Option<(PathBuf, PathBuf)> {
    let tokenizer_path = dir.join("tokenizer.json");
    if !tokenizer_path.exists() {
        return None;
    }
    for sub in candidate_subpaths {
        let onnx_path = dir.join(sub);
        if onnx_path.exists() {
            return Some((onnx_path, tokenizer_path));
        }
    }
    None
}

/// Locate the ONNX model + `tokenizer.json` using sidecar resolution.
pub(crate) fn resolve_model_files(model_name: &ModelName) -> Result<(PathBuf, PathBuf)> {
    let candidate_subpaths = model_name.onnx_candidate_subpaths();

    // Priority 1: Check CTX_MODELS_DIR
    if let Ok(models_dir) = std::env::var("CTX_MODELS_DIR") {
        let base = PathBuf::from(models_dir);
        let candidates = [base.join(model_name.model_dir_name()), base];
        for dir in candidates {
            if let Some((onnx, tok)) = check_directory_for_model(&dir, candidate_subpaths) {
                tracing::info!(
                    model = %model_name.version_string(),
                    onnx = %onnx.display(),
                    "found model in CTX_MODELS_DIR"
                );
                return Ok((onnx, tok));
            }
        }
    }

    // Priority 2: Check sidecar directory relative to executable (<exe_dir>/models/<model>/)
    // Priority 3: Check <exe_dir>/../models/<model>/ for cargo test / deps builds
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let primary = exe_dir.join("models").join(model_name.model_dir_name());
            if let Some((onnx, tok)) = check_directory_for_model(&primary, candidate_subpaths) {
                tracing::info!(
                    model = %model_name.version_string(),
                    onnx = %onnx.display(),
                    "found sidecar ONNX model and tokenizer"
                );
                return Ok((onnx, tok));
            }

            if let Some(parent) = exe_dir.parent() {
                let parent_models = parent.join("models").join(model_name.model_dir_name());
                if let Some((onnx, tok)) =
                    check_directory_for_model(&parent_models, candidate_subpaths)
                {
                    tracing::info!(
                        model = %model_name.version_string(),
                        onnx = %onnx.display(),
                        "found parent sidecar ONNX model and tokenizer"
                    );
                    return Ok((onnx, tok));
                }
            }
        }
    }

    Err(Error::Index(format!(
        "Embedding model '{}' not found. Mirror the Hugging Face repo layout: place \
         'onnx/model_quantized.onnx' and 'tokenizer.json' under '<exe_dir>/models/{}/' \
         (or set 'CTX_MODELS_DIR' to the parent models directory). Run scripts/fetch-model.sh \
         (or scripts/fetch-model.ps1) to download them.",
        model_name.version_string(),
        model_name.model_dir_name()
    )))
}
