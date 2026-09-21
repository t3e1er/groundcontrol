//! Embedding provider port.

use crate::Result;

/// Embedding provider port: the dense-embedding contract for a corpus.
///
/// This is the domain-facing contract for the ONNX-backed `Embedder`. It covers
/// the embedding capabilities consumers actually reach for across the port
/// boundary: encoding a single search query, encoding a batch of texts, and
/// reporting the output dimensionality. Every signature speaks only `&str` /
/// `&[&str]` inputs and plain `Vec<f32>` / `Vec<Vec<f32>>` / `usize` outputs —
/// no backend type (`ort::*`, `tokenizers::Tokenizer`, the core-local
/// `ModelName` enum, or `HardwareGovernor`) ever crosses this boundary, so
/// consumers depend on the contract rather than on ONNX Runtime.
///
/// Signatures mirror the concrete adapter's inherent methods exactly so the
/// adapter forwards trivially, and the surface is deliberately **object-safe**
/// (no generic methods, no `Self`-returning methods, no associated consts): it
/// is usable both as a generic trait bound — the intended hot-path wiring per
/// the generic-vs-`dyn` decision above — and, if ever needed, behind
/// `Arc<dyn EmbeddingProvider>`.
///
/// # Exclusions (surface deliberately not on the port)
///
/// - **`model_name()` / model-version / max-seq-len.** `model_name()` returns a
///   core-local `ModelName` enum that wraps ONNX/tokenizer backend knowledge;
///   returning it would leak a backend-coupled type across the port, and
///   relocating `ModelName` into `common` would drag model/backend knowledge
///   into the dependency-light crate. Its only two uses are inside the adapter's
///   own crate — `version_string()` (engine.rs) and `max_seq_len()`
///   (pipeline.rs) — where the caller holds a *concrete* `Embedder` /
///   `Arc<Embedder>`, not a port-typed value. No consumer needs a model version
///   *through* the port today, so none of these are on the contract; they stay
///   inherent, backend-coupled methods on the adapter.
/// - **`average_embeddings`.** It is a pure, stateless *associated* (static)
///   function (`Embedder::average_embeddings(&[Vec<f32>]) -> Option<Vec<f32>>`)
///   with no `&self`, called as `Embedder::average_embeddings(...)`. It is not a
///   per-instance provider capability, and a static method on a trait would
///   break object-safety. It stays an inherent associated fn on the adapter.
/// - **`tokenizer()` / `governor()` / `session_count()` /
///   `has_token_type_ids()` / `is_gpu_disabled()` / `reset_gpu_disabled()`.**
///   These expose backend or backend-adjacent types (`tokenizers::Tokenizer`,
///   `Arc<dyn HardwareGovernor>`) or internal accelerator state. The async
///   indexing pipeline that the task text alludes to holds the *concrete*
///   `Arc<Embedder>` (not the port) and reaches these inherent accessors
///   directly; no consumer accesses them through a port-typed value, so they are
///   excluded to keep every port signature backend-free.
///
/// `embed()` (embed a single text) is likewise **not** on the port: it is the
/// internal sibling that [`EmbeddingProvider::embed_query`] wraps, has no
/// cross-boundary caller, and is trivially `embed_batch`-of-one. Consumers cross
/// the boundary via `embed_query`; `embed` stays inherent.
///
/// Construction (loading the ONNX model + tokenizer) is deliberately **not**
/// part of this port: it is an adapter/composition-root concern. The port
/// describes only the runtime behaviour an embedding provider must offer.
pub trait EmbeddingProvider {
    /// Embed a search query string into a single dense vector.
    ///
    /// Returns one L2-normalized embedding vector for the query.
    fn embed_query(&self, query: &str) -> Result<Vec<f32>>;

    /// Embed a batch of text strings into dense vectors.
    ///
    /// Returns one L2-normalized embedding vector per input string, in the exact
    /// order of `texts`.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    /// Get the output dimensionality of the embeddings this provider produces.
    fn dimensions(&self) -> usize;
}
