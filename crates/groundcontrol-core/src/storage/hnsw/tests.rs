use std::fs;

use groundcontrol_common::types::Modality;

use super::store::VectorIndex;

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

    let ids = index.add_batch(&vectors, "notes/multi.md", &chunk_indices, false, "docs").unwrap();

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
    let doc_a_results: Vec<_> = results.iter().filter(|r| r.doc_path == "notes/doc_a.md").collect();
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
