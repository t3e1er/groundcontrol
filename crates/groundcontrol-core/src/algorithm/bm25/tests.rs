//! Tests for BM25 retrieval algorithm component.

use super::*;
use groundcontrol_common::types::{Chunk, FileFormat, Modality, ParsedArtifact};

#[test]
fn test_bm25_lifecycle_and_search() {
    let bm25_index = BM25Index::open_in_memory().unwrap();
    let mut algo = Bm25Algorithm::new(bm25_index);

    let doc = ParsedArtifact {
        path: "src/lib.rs".to_string(),
        hash: "hash1".to_string(),
        is_code: true,
        format: FileFormat::Source,
        title: Some("lib.rs".to_string()),
        doc_metadata: None,
        symbols: Vec::new(),
        grammar_semantics: Vec::new(),
        chunks: vec![
            Chunk::new("src/lib.rs".to_string(), 0, "fn authenticate_user() {}".to_string(), 0, 30),
            Chunk::new("src/lib.rs".to_string(), 1, "fn parse_token() {}".to_string(), 31, 60),
        ],
        graph_edges: Vec::new(),
        external_refs: Vec::new(),
        raw_content: None,
        projection_text: None,
    };

    algo.index_document(&doc).unwrap();
    algo.commit().unwrap();

    let hits = algo.search("authenticate", 5, Modality::Both).unwrap();
    assert!(!hits.is_empty(), "expected hits for 'authenticate'");
    assert_eq!(hits[0].path, "src/lib.rs");

    algo.remove_document("src/lib.rs").unwrap();
    algo.commit().unwrap();

    let hits_after = algo.search("authenticate", 5, Modality::Both).unwrap();
    assert!(hits_after.is_empty(), "expected zero hits after removal");
}
