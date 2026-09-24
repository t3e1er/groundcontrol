//! Vector index: HNSW-based approximate nearest neighbor search.
//!
//! Wraps `hnsw_rs` to provide add/remove/search/save/load operations
//! for embedding vectors. Supports both chunk-level and document-level vectors.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use groundcontrol_common::{
    types::{Modality, VectorMeta, VectorSearchResult},
    Error, Result,
};
use hnsw_rs::prelude::*;
use serde::{Deserialize, Serialize};

/// Default number of dimensions for Jina embeddings (768).
pub const DEFAULT_DIMENSIONS: usize = 768;

/// Binary metadata trailer serialized via postcard.
#[derive(Debug, Serialize, Deserialize)]
struct BinaryMetadata {
    ids: Vec<usize>,
    meta: HashMap<usize, VectorMeta>,
    next_id: usize,
    max_nb_connection: usize,
    ef_construction: usize,
    model_version: Option<String>,
}

/// HNSW-based vector index for approximate nearest neighbor search.
pub struct VectorIndex {
    /// The HNSW graph structure.
    hnsw: Hnsw<'static, f32, DistCosine>,
    /// Mapping from external data ID to vector metadata.
    meta: HashMap<usize, VectorMeta>,
    /// Next available external ID.
    next_id: usize,
    /// Number of dimensions per vector.
    dimensions: usize,
    /// HNSW construction parameters for rebuild.
    max_nb_connection: usize,
    ef_construction: usize,
    /// Model version that produced these embeddings.
    model_version: Option<String>,
    /// Whether vectors are stale (model version mismatch detected).
    stale: bool,
    /// Whether the index has unsaved changes.
    dirty: AtomicBool,
}

impl VectorIndex {
    /// Create a new in-memory vector index.
    ///
    /// - `dimensions`: vector dimensionality (e.g., 384 for MiniLM-L6-v2)
    /// - `max_elements`: estimated maximum number of vectors (can grow)
    /// - `ef_construction`: HNSW construction parameter (higher = more accurate, slower build)
    /// - `max_nb_connection`: max neighbors per node in HNSW graph
    pub fn new(
        dimensions: usize,
        max_elements: usize,
        ef_construction: usize,
        max_nb_connection: usize,
    ) -> Self {
        let hnsw = Hnsw::<f32, DistCosine>::new(
            max_nb_connection,
            max_elements,
            16, // max_layer
            ef_construction,
            DistCosine,
        );

        Self {
            hnsw,
            meta: HashMap::new(),
            next_id: 0,
            dimensions,
            max_nb_connection,
            ef_construction,
            model_version: None,
            stale: false,
            dirty: AtomicBool::new(false),
        }
    }

    /// Create a new vector index with default parameters suitable for small-medium corpora.
    pub fn new_default(dimensions: usize) -> Self {
        Self::new(dimensions, 10_000, 200, 16)
    }

    /// Check whether the index has unpersisted changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    /// Mark the index as having unpersisted changes.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Clear the dirty flag.
    pub fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Relaxed);
    }

    /// Get the number of vectors currently in the index.
    pub fn len(&self) -> usize {
        self.meta.len()
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.meta.is_empty()
    }

    /// Get the dimensionality of vectors in this index.
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Get the model version stored with this index.
    pub fn model_version(&self) -> Option<&str> {
        self.model_version.as_deref()
    }

    /// Set the model version for this index.
    pub fn set_model_version(&mut self, version: &str) {
        self.model_version = Some(version.to_string());
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Check whether vectors are marked as stale (model version mismatch).
    pub fn is_stale(&self) -> bool {
        self.stale
    }

    /// Mark vectors as stale (model version mismatch detected).
    pub fn mark_stale(&mut self) {
        self.stale = true;
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Clear the stale flag (after re-embedding completes).
    pub fn clear_stale(&mut self) {
        self.stale = false;
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Add a single vector to the index.
    ///
    /// `modality` is the coarse modality tag ("code" / "docs") used for
    /// modality-filtered search.
    ///
    /// Returns the internal ID assigned to this vector.
    pub fn add(
        &mut self,
        vector: &[f32],
        doc_path: &str,
        chunk_index: Option<usize>,
        is_doc_level: bool,
        modality: &str,
    ) -> Result<usize> {
        if vector.len() != self.dimensions {
            return Err(Error::Index(format!(
                "vector dimension mismatch: expected {}, got {}",
                self.dimensions,
                vector.len()
            )));
        }

        let id = self.next_id;
        self.next_id += 1;

        // Insert into HNSW. The insert_slice method takes a tuple (&[T], DataId).
        self.hnsw.insert_slice((&vector, id));

        let meta = VectorMeta {
            doc_path: doc_path.to_string(),
            chunk_index,
            is_doc_level,
            modality: modality.to_string(),
        };

        // Store metadata.
        let _ = self.meta.insert(id, meta);
        self.dirty.store(true, Ordering::Relaxed);

        Ok(id)
    }

    /// Add multiple vectors in batch (more efficient than individual adds).
    ///
    /// Returns the internal IDs assigned.
    pub fn add_batch(
        &mut self,
        vectors: &[Vec<f32>],
        doc_path: &str,
        chunk_indices: &[Option<usize>],
        is_doc_level: bool,
        modality: &str,
    ) -> Result<Vec<usize>> {
        if vectors.len() != chunk_indices.len() {
            return Err(Error::Index(
                "vectors and chunk_indices must have same length".to_string(),
            ));
        }

        let mut ids = Vec::with_capacity(vectors.len());

        for (vec, &chunk_idx) in vectors.iter().zip(chunk_indices.iter()) {
            let id = self.add(vec, doc_path, chunk_idx, is_doc_level, modality)?;
            ids.push(id);
        }

        Ok(ids)
    }

    /// Remove all vectors for a given document path.
    ///
    /// Note: HNSW doesn't support true deletion, so we remove from metadata.
    /// The HNSW graph entries become stale but are filtered out during search.
    /// A rebuild (save+load) compacts the index.
    pub fn remove_document(&mut self, doc_path: &str) {
        let ids_to_remove: Vec<usize> =
            self.meta.iter().filter(|(_, m)| m.doc_path == doc_path).map(|(&id, _)| id).collect();

        if !ids_to_remove.is_empty() {
            self.dirty.store(true, Ordering::Relaxed);
        }

        for id in ids_to_remove {
            let _ = self.meta.remove(&id);
        }
    }

    /// Search for the K nearest neighbors to a query vector.
    ///
    /// - `query`: the query embedding vector
    /// - `k`: number of results to return
    /// - `doc_level_only`: if true, only return document-level embeddings
    /// - `modality`: restrict results to the given modality (post-filter on
    ///   each vector's coarse modality tag)
    ///
    /// Returns results sorted by descending similarity score.
    pub fn search(
        &self,
        query: &[f32],
        k: usize,
        doc_level_only: bool,
        modality: Modality,
    ) -> Result<Vec<VectorSearchResult>> {
        if query.len() != self.dimensions {
            return Err(Error::Index(format!(
                "query dimension mismatch: expected {}, got {}",
                self.dimensions,
                query.len()
            )));
        }

        if self.meta.is_empty() || k == 0 {
            return Ok(Vec::new());
        }

        // Over-fetch neighbors to account for multiple chunks per document and filtered entries.
        let fetch_k = (k * 10).max(64);
        let ef_search = (k * 10).max(64);

        let neighbours = self.hnsw.search(query, fetch_k, ef_search);

        let mut candidate_results: Vec<VectorSearchResult> = neighbours
            .into_iter()
            .filter_map(|neighbour| {
                let id = neighbour.d_id;
                let meta = self.meta.get(&id)?;

                // Filter by level if requested.
                if doc_level_only && !meta.is_doc_level {
                    return None;
                }

                // Filter by modality (post-filter on the coarse tag).
                if !modality.matches_tag(&meta.modality) {
                    return None;
                }

                // DistCosine returns 1 - cos(a,b), so similarity = 1 - distance.
                let similarity = 1.0 - neighbour.distance as f64;

                Some(VectorSearchResult {
                    doc_path: meta.doc_path.clone(),
                    chunk_index: meta.chunk_index,
                    score: similarity,
                    is_doc_level: meta.is_doc_level,
                    modality: meta.modality.clone(),
                })
            })
            .collect();

        // Sort by descending score (highest similarity first).
        candidate_results
            .sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Document-level max pooling: retain only the highest scoring chunk per document.
        let mut seen_docs = HashSet::new();
        let mut results = Vec::with_capacity(k);
        for item in candidate_results {
            if seen_docs.insert(item.doc_path.clone()) {
                results.push(item);
                if results.len() >= k {
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Save the index to disk using the packed IEEE 754 binary format (`vectors.bin`)
    /// with atomic OS replacement.
    ///
    /// Header: Magic `b"CTXV"` (4B), version `1u16` (2B), dimensions `u16` (2B),
    /// vector count `u32` (4B), reserved `[0u8; 20]` (20B).
    /// Body: Contiguous raw float bytes (`count * dimensions * 4` bytes).
    /// Tail: Length-prefixed postcard-serialized `BinaryMetadata`.
    pub fn save_binary(&self, path: &Path) -> Result<()> {
        let parent = path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|e| Error::Index(format!("cannot create vector index dir: {}", e)))?;

        let tmp_path = path.with_extension("bin.tmp");
        let file = fs::File::create(&tmp_path).map_err(|e| {
            Error::Index(format!("cannot create tmp vector index {}: {}", tmp_path.display(), e))
        })?;
        let mut writer = std::io::BufWriter::new(file);

        // Gather active points from PointIndexation
        let pi = self.hnsw.get_point_indexation();
        let mut points_to_write = Vec::with_capacity(self.meta.len());
        for point in pi {
            let id = point.get_origin_id();
            if self.meta.contains_key(&id) {
                points_to_write.push((id, point));
            }
        }
        let count = points_to_write.len();

        // 1. Write Header (32 bytes)
        writer
            .write_all(b"CTXV")
            .map_err(|e| Error::Index(format!("failed to write magic: {e}")))?;
        writer
            .write_all(&1u16.to_le_bytes())
            .map_err(|e| Error::Index(format!("failed to write version: {e}")))?;
        writer
            .write_all(&(self.dimensions as u16).to_le_bytes())
            .map_err(|e| Error::Index(format!("failed to write dimensions: {e}")))?;
        writer
            .write_all(&(count as u32).to_le_bytes())
            .map_err(|e| Error::Index(format!("failed to write count: {e}")))?;
        writer
            .write_all(&[0u8; 20])
            .map_err(|e| Error::Index(format!("failed to write reserved padding: {e}")))?;

        // 2. Write Body: contiguous raw float bytes from active points
        let mut written_ids = Vec::with_capacity(count);
        for (id, point) in points_to_write {
            written_ids.push(id);
            let bytes: &[u8] = bytemuck::cast_slice(point.get_v());
            writer
                .write_all(bytes)
                .map_err(|e| Error::Index(format!("failed to write vector {id}: {e}")))?;
        }

        // 3. Write Tail: postcard-serialized metadata
        let metadata = BinaryMetadata {
            ids: written_ids,
            meta: self.meta.clone(),
            next_id: self.next_id,
            max_nb_connection: self.max_nb_connection,
            ef_construction: self.ef_construction,
            model_version: self.model_version.clone(),
        };

        let meta_bytes = postcard::to_allocvec(&metadata)
            .map_err(|e| Error::Index(format!("failed to serialize vector metadata: {e}")))?;
        writer
            .write_all(&(meta_bytes.len() as u64).to_le_bytes())
            .map_err(|e| Error::Index(format!("failed to write meta len: {e}")))?;
        writer
            .write_all(&meta_bytes)
            .map_err(|e| Error::Index(format!("failed to write meta bytes: {e}")))?;

        writer.flush().map_err(|e| Error::Index(format!("failed to flush vector index: {e}")))?;
        let file = writer
            .into_inner()
            .map_err(|e| Error::Index(format!("failed to unwrap writer: {e}")))?;
        file.sync_all().map_err(|e| Error::Index(format!("failed to sync vector index: {e}")))?;
        drop(file);

        fs::rename(&tmp_path, path)
            .map_err(|e| Error::Index(format!("failed to atomically rename vector index: {e}")))?;

        self.dirty.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// Load a previously saved index from a packed binary file (`vectors.bin`)
    /// and rebuild the HNSW graph.
    pub fn load_binary(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(Error::Index(format!("vector index file not found at {}", path.display())));
        }

        let file = fs::File::open(path).map_err(|e| {
            Error::Index(format!("cannot open vector index {}: {}", path.display(), e))
        })?;
        let mut reader = std::io::BufReader::new(file);

        // 1. Read Header (32 bytes)
        let mut magic = [0u8; 4];
        reader
            .read_exact(&mut magic)
            .map_err(|e| Error::Index(format!("failed to read magic: {e}")))?;
        if &magic != b"CTXV" {
            return Err(Error::Index(format!(
                "invalid vector index magic: expected b\"CTXV\", got {:?}",
                magic
            )));
        }

        let mut version_bytes = [0u8; 2];
        reader
            .read_exact(&mut version_bytes)
            .map_err(|e| Error::Index(format!("failed to read version: {e}")))?;
        let version = u16::from_le_bytes(version_bytes);
        if version != 1 {
            return Err(Error::Index(format!(
                "unsupported vector index version: expected 1, got {version}"
            )));
        }

        let mut dim_bytes = [0u8; 2];
        reader
            .read_exact(&mut dim_bytes)
            .map_err(|e| Error::Index(format!("failed to read dimensions: {e}")))?;
        let dimensions = u16::from_le_bytes(dim_bytes) as usize;

        let mut count_bytes = [0u8; 4];
        reader
            .read_exact(&mut count_bytes)
            .map_err(|e| Error::Index(format!("failed to read count: {e}")))?;
        let count = u32::from_le_bytes(count_bytes) as usize;

        let mut reserved = [0u8; 20];
        reader
            .read_exact(&mut reserved)
            .map_err(|e| Error::Index(format!("failed to read reserved bytes: {e}")))?;

        // 2. Read Body: contiguous raw float bytes into an aligned buffer
        let total_floats = count
            .checked_mul(dimensions)
            .ok_or_else(|| Error::Index("vector float buffer size overflow".to_string()))?;
        let mut float_buf = vec![0.0f32; total_floats];
        let byte_slice: &mut [u8] = bytemuck::cast_slice_mut(&mut float_buf);
        reader
            .read_exact(byte_slice)
            .map_err(|e| Error::Index(format!("failed to read vector body: {e}")))?;

        // 3. Read Tail: metadata length + postcard payload
        let mut meta_len_bytes = [0u8; 8];
        reader
            .read_exact(&mut meta_len_bytes)
            .map_err(|e| Error::Index(format!("failed to read meta length: {e}")))?;
        let meta_len = u64::from_le_bytes(meta_len_bytes) as usize;

        let mut meta_bytes = vec![0u8; meta_len];
        reader
            .read_exact(&mut meta_bytes)
            .map_err(|e| Error::Index(format!("failed to read meta bytes: {e}")))?;

        let metadata: BinaryMetadata = postcard::from_bytes(&meta_bytes)
            .map_err(|e| Error::Index(format!("failed to deserialize vector metadata: {e}")))?;

        if metadata.ids.len() != count {
            return Err(Error::Index(format!(
                "vector count mismatch: header indicates {count}, metadata has {} ids",
                metadata.ids.len()
            )));
        }

        // 4. Rebuild HNSW
        let max_elements = count.max(100);
        let hnsw = Hnsw::<f32, DistCosine>::new(
            metadata.max_nb_connection,
            max_elements,
            16,
            metadata.ef_construction,
            DistCosine,
        );

        for (i, &id) in metadata.ids.iter().enumerate() {
            let slice = &float_buf[i * dimensions..(i + 1) * dimensions];
            hnsw.insert_slice((slice, id));
        }

        Ok(Self {
            hnsw,
            meta: metadata.meta,
            next_id: metadata.next_id,
            dimensions,
            max_nb_connection: metadata.max_nb_connection,
            ef_construction: metadata.ef_construction,
            model_version: metadata.model_version,
            stale: false,
            dirty: AtomicBool::new(false),
        })
    }

    /// Save the index to disk using the packed binary format (`vectors.bin`).
    pub fn save(&self, path: &Path) -> Result<()> {
        self.save_binary(path)
    }

    /// Load a previously saved index from disk using the packed binary format.
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_binary(path)
    }
}

// ---------------------------------------------------------------------------
// Port adapter: VectorStore
// ---------------------------------------------------------------------------

impl groundcontrol_common::ports::VectorStore for VectorIndex {
    fn add(
        &mut self,
        vector: &[f32],
        doc_path: &str,
        chunk_index: Option<usize>,
        is_doc_level: bool,
        modality: &str,
    ) -> Result<usize> {
        VectorIndex::add(self, vector, doc_path, chunk_index, is_doc_level, modality)
    }

    fn add_batch(
        &mut self,
        vectors: &[Vec<f32>],
        doc_path: &str,
        chunk_indices: &[Option<usize>],
        is_doc_level: bool,
        modality: &str,
    ) -> Result<Vec<usize>> {
        VectorIndex::add_batch(self, vectors, doc_path, chunk_indices, is_doc_level, modality)
    }

    fn remove_document(&mut self, doc_path: &str) {
        VectorIndex::remove_document(self, doc_path)
    }

    fn search(
        &self,
        query: &[f32],
        k: usize,
        doc_level_only: bool,
        modality: Modality,
    ) -> Result<Vec<VectorSearchResult>> {
        VectorIndex::search(self, query, k, doc_level_only, modality)
    }

    fn save(&self, path: &Path) -> Result<()> {
        VectorIndex::save(self, path)
    }

    fn dimensions(&self) -> usize {
        VectorIndex::dimensions(self)
    }

    fn len(&self) -> usize {
        VectorIndex::len(self)
    }

    fn is_empty(&self) -> bool {
        VectorIndex::is_empty(self)
    }

    fn model_version(&self) -> Option<&str> {
        VectorIndex::model_version(self)
    }

    fn set_model_version(&mut self, version: &str) {
        VectorIndex::set_model_version(self, version)
    }

    fn is_stale(&self) -> bool {
        VectorIndex::is_stale(self)
    }

    fn mark_stale(&mut self) {
        VectorIndex::mark_stale(self)
    }

    fn clear_stale(&mut self) {
        VectorIndex::clear_stale(self)
    }

    fn is_dirty(&self) -> bool {
        VectorIndex::is_dirty(self)
    }

    fn mark_dirty(&self) {
        VectorIndex::mark_dirty(self)
    }

    fn clear_dirty(&self) {
        VectorIndex::clear_dirty(self)
    }
}
