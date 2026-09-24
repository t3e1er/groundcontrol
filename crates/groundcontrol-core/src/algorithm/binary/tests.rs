//! Tests for binary retrieval algorithm component.

use super::*;
use groundcontrol_common::types::{Chunk, FileFormat, Modality, ParsedArtifact};

#[test]
fn test_binary_lifecycle_and_search() {
    let binary_index = BinarySearchIndex::new();
    let mut algo = BinaryAlgorithm::new(binary_index);

    let doc = ParsedArtifact {
        path: "src/crypto.rs".to_string(),
        hash: "hash_crypto".to_string(),
        is_code: true,
        format: FileFormat::Source,
        title: Some("crypto.rs".to_string()),
        doc_metadata: None,
        symbols: Vec::new(),
        grammar_semantics: Vec::new(),
        chunks: vec![Chunk::new(
            "src/crypto.rs".to_string(),
            0,
            "fn hash_sha256(data: &[u8]) -> Vec<u8>".to_string(),
            0,
            40,
        )],
        graph_edges: Vec::new(),
        external_refs: Vec::new(),
        raw_content: Some(
            "fn hash_sha256(data: &[u8]) -> Vec<u8> { sha2::Sha256::digest(data) }".to_string(),
        ),
        projection_text: None,
    };

    algo.index_document(&doc).unwrap();

    let hits = algo.search("sha256 hash", 5, Modality::Both).unwrap();
    assert!(!hits.is_empty(), "expected binary hits for 'sha256 hash'");
    assert_eq!(hits[0].path, "src/crypto.rs");

    algo.remove_document("src/crypto.rs").unwrap();
    let hits_after = algo.search("sha256 hash", 5, Modality::Both).unwrap();
    assert!(hits_after.is_empty(), "expected zero hits after removal");
}
