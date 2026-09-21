//! Vector store port.

use std::path::Path;

use crate::types::{Modality, VectorSearchResult};
use crate::Result;

/// Vector store port: the dense approximate-nearest-neighbor contract for a corpus.
///
/// This is the domain-facing contract for the HNSW-backed vector index. It
/// covers vector ingestion (single/batch add and per-document removal),
/// similarity search restricted to a [`Modality`], persistence to disk, and the
/// dimension/model-version/stale/dirty bookkeeping the engine relies on. Every
/// signature speaks only plain `Vec<f32>` / `&[f32]` vectors, standard-library
/// types, and [`crate::types`] domain types
/// ([`Modality`], [`VectorSearchResult`]) — no backend type (`hnsw_rs::*`) ever
/// crosses this boundary, so consumers depend on the contract rather than on
/// HNSW.
///
/// # Construction vs. persistence
///
/// Persistence is split by object-safety and ownership:
///
/// - [`VectorStore::save`] is an **instance** method (`&self`) and therefore
///   part of the port — persisting the current state is runtime behaviour a
///   store must provide.
/// - Loading is deliberately **not** on the port. The load-equivalent operation
///   (and the `new` / `new_default` constructors) return `Self`, which a
///   `&dyn`-object-safe trait cannot express, and constructing a store — reading
///   a `vectors.bin` off disk or building an empty index — is an
///   adapter/composition-root concern, not a runtime behaviour of an existing
///   store. The composition root constructs the concrete adapter (loading from
///   disk when present) and injects it behind this port.
pub trait VectorStore {
    /// Add a single vector to the index.
    ///
    /// `modality` is the coarse modality tag ("code" / "docs") used for
    /// modality-filtered search. Returns the internal ID assigned to the vector.
    fn add(
        &mut self,
        vector: &[f32],
        doc_path: &str,
        chunk_index: Option<usize>,
        is_doc_level: bool,
        modality: &str,
    ) -> Result<usize>;

    /// Add multiple vectors in batch (more efficient than individual adds).
    ///
    /// Returns the internal IDs assigned, in input order.
    fn add_batch(
        &mut self,
        vectors: &[Vec<f32>],
        doc_path: &str,
        chunk_indices: &[Option<usize>],
        is_doc_level: bool,
        modality: &str,
    ) -> Result<Vec<usize>>;

    /// Remove all vectors associated with a given document path.
    fn remove_document(&mut self, doc_path: &str);

    /// Search for the `k` nearest neighbors to a query vector.
    ///
    /// - `doc_level_only`: if true, only return document-level embeddings.
    /// - `modality`: restrict results to the given [`Modality`] (post-filter on
    ///   each vector's coarse modality tag).
    ///
    /// Returns results sorted by descending similarity score.
    fn search(
        &self,
        query: &[f32],
        k: usize,
        doc_level_only: bool,
        modality: Modality,
    ) -> Result<Vec<VectorSearchResult>>;

    /// Persist the index to disk (vectors + metadata) at the given path.
    fn save(&self, path: &Path) -> Result<()>;

    /// Get the dimensionality of vectors in this index.
    fn dimensions(&self) -> usize;

    /// Get the number of vectors currently in the index.
    fn len(&self) -> usize;

    /// Check whether the index is empty.
    fn is_empty(&self) -> bool;

    /// Get the model version stored with this index, if any.
    fn model_version(&self) -> Option<&str>;

    /// Set the model version for this index.
    fn set_model_version(&mut self, version: &str);

    /// Check whether vectors are marked as stale (model version mismatch).
    fn is_stale(&self) -> bool;

    /// Mark vectors as stale (model version mismatch detected).
    fn mark_stale(&mut self);

    /// Clear the stale flag (after re-embedding completes).
    fn clear_stale(&mut self);

    /// Check whether the index has unpersisted changes.
    fn is_dirty(&self) -> bool;

    /// Mark the index as having unpersisted changes.
    fn mark_dirty(&self);

    /// Clear the dirty flag.
    fn clear_dirty(&self);
}
