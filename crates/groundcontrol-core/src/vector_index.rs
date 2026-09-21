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

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a deterministic vector with a specific seed pattern, L2-normalized.
    fn make_vector(seed: usize, dims: usize) -> Vec<f32> {
        let v: Vec<f32> = (0..dims).map(|i| ((seed * 7 + i * 13) % 100) as f32 / 100.0).collect();
        // L2-normalize for cosine distance.
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            v.iter().map(|x| x / norm).collect()
        } else {
            v
        }
    }

    /// Generate a vector that's similar to another (small perturbation).
    fn make_similar_vector(base: &[f32], offset: f32) -> Vec<f32> {
        let v: Vec<f32> = base.iter().map(|&val| val + offset * 0.01).collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            v.iter().map(|x| x / norm).collect()
        } else {
            v
        }
    }

    #[test]
    fn test_create_empty_index() {
        let index = VectorIndex::new_default(384);
        assert_eq!(index.len(), 0);
        assert!(index.is_empty());
        assert_eq!(index.dimensions(), 384);
    }

    #[test]
    fn test_add_single_vector() {
        let mut index = VectorIndex::new_default(384);
        let vec = make_vector(1, 384);

        let id = index.add(&vec, "notes/test.md", Some(0), false, "docs").unwrap();
        assert_eq!(id, 0);
        assert_eq!(index.len(), 1);
        assert!(!index.is_empty());
    }

    #[test]
    fn test_add_batch() {
        let mut index = VectorIndex::new_default(384);

        let vectors: Vec<Vec<f32>> = (0..5).map(|i| make_vector(i, 384)).collect();
        let chunk_indices: Vec<Option<usize>> = (0..5).map(Some).collect();

        let ids =
            index.add_batch(&vectors, "notes/multi.md", &chunk_indices, false, "docs").unwrap();

        assert_eq!(ids.len(), 5);
        assert_eq!(index.len(), 5);
    }

    #[test]
    fn test_dimension_mismatch_rejected() {
        let mut index = VectorIndex::new_default(384);
        let wrong_vec = make_vector(1, 256); // Wrong dimension.

        let result = index.add(&wrong_vec, "notes/bad.md", Some(0), false, "docs");
        assert!(result.is_err());
    }

    #[test]
    fn test_search_finds_similar() {
        let mut index = VectorIndex::new(384, 100, 200, 16);

        // Add a base vector and some others.
        let base = make_vector(42, 384);
        let similar = make_similar_vector(&base, 1.0);
        let different = make_vector(999, 384);

        index.add(&base, "notes/base.md", Some(0), false, "docs").unwrap();
        index.add(&similar, "notes/similar.md", Some(0), false, "docs").unwrap();
        index.add(&different, "notes/different.md", Some(0), false, "docs").unwrap();

        // Search with the base vector — should find itself and similar.
        let results = index.search(&base, 3, false, Modality::Both).unwrap();
        assert!(!results.is_empty());

        // The base vector should be the top result (exact match = highest similarity).
        assert_eq!(results[0].doc_path, "notes/base.md");

        // Scores should be in descending order.
        for window in results.windows(2) {
            assert!(window[0].score >= window[1].score);
        }
    }

    #[test]
    fn test_search_empty_index() {
        let index = VectorIndex::new_default(384);
        let query = make_vector(1, 384);

        let results = index.search(&query, 10, false, Modality::Both).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_dimension_mismatch() {
        let mut index = VectorIndex::new_default(384);
        let vec = make_vector(1, 384);
        index.add(&vec, "notes/a.md", Some(0), false, "docs").unwrap();

        let bad_query = make_vector(1, 256);
        let result = index.search(&bad_query, 10, false, Modality::Both);
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_document() {
        let mut index = VectorIndex::new(384, 100, 200, 16);

        let v1 = make_vector(1, 384);
        let v2 = make_vector(2, 384);
        let v3 = make_vector(3, 384);

        index.add(&v1, "notes/keep.md", Some(0), false, "docs").unwrap();
        index.add(&v2, "notes/remove.md", Some(0), false, "docs").unwrap();
        index.add(&v3, "notes/remove.md", Some(1), false, "docs").unwrap();

        assert_eq!(index.len(), 3);

        index.remove_document("notes/remove.md");
        assert_eq!(index.len(), 1);

        // Search should not return removed documents.
        let results = index.search(&v2, 10, false, Modality::Both).unwrap();
        for r in &results {
            assert_ne!(r.doc_path, "notes/remove.md");
        }
    }

    #[test]
    fn test_doc_level_filter() {
        let mut index = VectorIndex::new(384, 100, 200, 16);

        let v1 = make_vector(1, 384);
        let v2 = make_vector(2, 384);

        // Add chunk-level and doc-level vectors.
        index.add(&v1, "notes/a.md", Some(0), false, "docs").unwrap();
        index.add(&v2, "notes/a.md", None, true, "docs").unwrap();

        // Search with doc_level_only = true should only return doc-level.
        let results = index.search(&v2, 10, true, Modality::Both).unwrap();
        for r in &results {
            assert!(r.is_doc_level);
        }
    }

    #[test]
    fn test_save_and_load() {
        let tmp = tempfile::TempDir::new().unwrap();
        let index_path = tmp.path().join("vectors.bin");

        // Create and populate an index.
        let mut index = VectorIndex::new(384, 100, 200, 16);
        let v1 = make_vector(10, 384);
        let v2 = make_vector(20, 384);
        let v3 = make_vector(30, 384);

        index.add(&v1, "notes/alpha.md", Some(0), false, "docs").unwrap();
        index.add(&v2, "notes/beta.md", Some(0), false, "docs").unwrap();
        index.add(&v3, "notes/alpha.md", None, true, "docs").unwrap();

        // Save to disk.
        index.save(&index_path).unwrap();

        // Load from disk.
        let loaded = VectorIndex::load(&index_path).unwrap();

        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded.dimensions(), 384);

        // Search should work on loaded index.
        let results = loaded.search(&v1, 3, false, Modality::Both).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].doc_path, "notes/alpha.md");
    }

    #[test]
    fn test_search_document_deduplication() {
        let mut index = VectorIndex::new(384, 100, 200, 16);
        let base = make_vector(1, 384);
        let chunk0 = make_similar_vector(&base, 1.0);
        let chunk1 = make_similar_vector(&base, 0.1); // Closer to base
        let other = make_vector(20, 384);

        // Add 2 chunks for doc A and 1 for doc B
        index.add(&chunk0, "notes/doc_a.md", Some(0), false, "docs").unwrap();
        index.add(&chunk1, "notes/doc_a.md", Some(1), false, "docs").unwrap();
        index.add(&other, "notes/doc_b.md", Some(0), false, "docs").unwrap();

        let results = index.search(&base, 5, false, Modality::Both).unwrap();

        // doc_a should appear only once (with chunk 1 which is closer)
        let doc_a_results: Vec<_> =
            results.iter().filter(|r| r.doc_path == "notes/doc_a.md").collect();
        assert_eq!(doc_a_results.len(), 1);
        assert_eq!(doc_a_results[0].chunk_index, Some(1));
    }

    #[test]
    fn test_modality_filter() {
        let mut index = VectorIndex::new_default(384);

        // Two similar vectors, one tagged docs, one tagged code, symmetrically offset from base.
        let base = make_vector(7, 384);
        let doc_vec = make_similar_vector(&base, 0.2);
        let code_vec = make_similar_vector(&base, -0.2);

        index.add(&doc_vec, "notes/guide.md", Some(0), false, "docs").unwrap();
        index.add(&code_vec, "src/engine.rs", Some(0), false, "code").unwrap();

        // Code-only returns only the code vector.
        let code_results = index.search(&code_vec, 10, false, Modality::Code).unwrap();
        assert!(!code_results.is_empty());
        assert!(code_results.iter().all(|r| r.modality == "code"));
        assert!(code_results.iter().all(|r| r.doc_path == "src/engine.rs"));

        // Docs-only returns only the doc vector.
        let doc_results = index.search(&doc_vec, 10, false, Modality::Docs).unwrap();
        assert!(!doc_results.is_empty());
        assert!(doc_results.iter().all(|r| r.modality == "docs"));
        assert!(doc_results.iter().all(|r| r.doc_path == "notes/guide.md"));

        // Both returns both.
        let both_results = index.search(&code_vec, 10, false, Modality::Both).unwrap();
        let paths: Vec<&str> = both_results.iter().map(|r| r.doc_path.as_str()).collect();
        assert!(paths.contains(&"notes/guide.md"));
        assert!(paths.contains(&"src/engine.rs"));
    }

    #[test]
    fn test_modality_survives_save_load() {
        let tmp = tempfile::TempDir::new().unwrap();
        let index_path = tmp.path().join("vectors.bin");

        let mut index = VectorIndex::new_default(384);
        let base = make_vector(9, 384);
        index.add(&make_similar_vector(&base, 0.4), "src/lib.rs", Some(0), false, "code").unwrap();
        index.save(&index_path).unwrap();

        let loaded = VectorIndex::load(&index_path).unwrap();
        let results = loaded.search(&base, 10, false, Modality::Code).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.modality == "code"));
    }

    #[test]
    fn test_dirty_tracking_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let index_path = dir.path().join("vectors.bin");

        let mut index = VectorIndex::new_default(384);
        assert!(!index.is_dirty());

        let v1 = make_vector(1, 384);
        index.add(&v1, "notes/doc.md", Some(0), false, "docs").unwrap();
        assert!(index.is_dirty());

        index.save(&index_path).unwrap();
        assert!(!index.is_dirty());

        // Modification marks dirty again
        index.remove_document("notes/doc.md");
        assert!(index.is_dirty());

        let loaded = VectorIndex::load(&index_path).unwrap();
        assert!(!loaded.is_dirty());
    }

    #[test]
    fn test_binary_header_and_format() {
        let tmp = tempfile::TempDir::new().unwrap();
        let index_path = tmp.path().join("vectors.bin");

        let mut index = VectorIndex::new_default(768);
        index.set_model_version("test-model-v1");
        let v = make_vector(42, 768);
        index.add(&v, "src/lib.rs", Some(0), false, "code").unwrap();

        index.save_binary(&index_path).unwrap();

        // Verify raw bytes on disk
        let bytes = fs::read(&index_path).unwrap();
        assert!(bytes.len() >= 32);
        assert_eq!(&bytes[0..4], b"CTXV");
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 1);
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), 768);
        assert_eq!(u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]), 1);

        // Load and verify model_version preserved
        let loaded = VectorIndex::load_binary(&index_path).unwrap();
        assert_eq!(loaded.model_version(), Some("test-model-v1"));
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.dimensions(), 768);
    }
}
