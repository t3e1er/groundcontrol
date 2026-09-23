//! Tests for dense retrieval algorithm component.

use super::*;
use groundcontrol_common::types::{Chunk, ChunkEmbedPolicy, FileFormat, Modality, ParsedArtifact};

#[test]
fn test_dense_lifecycle() {
    let vi = VectorIndex::new_default(4);
    let mut algo = DenseAlgorithm::new(vi, None);

    let doc = ParsedArtifact {
        path: "docs/architecture.md".to_string(),
        hash: "hash_arch".to_string(),
        is_code: false,
        format: FileFormat::Source,
        title: Some("Architecture".to_string()),
        doc_metadata: None,
        symbols: Vec::new(),
        grammar_semantics: Vec::new(),
        chunks: vec![Chunk::new(
            "docs/architecture.md".to_string(),
            0,
            "System architecture overview".to_string(),
            0,
            30,
        )
        .with_embed_policy(ChunkEmbedPolicy::Anchor)],
        graph_edges: Vec::new(),
        external_refs: Vec::new(),
        raw_content: None,
        projection_text: None,
    };

    // Indexing without embedder does not panic and cleanly returns Ok
    algo.index_document(&doc).unwrap();

    // Removing document cleanly returns Ok
    algo.remove_document("docs/architecture.md").unwrap();

    // Clearing index resets vector index
    algo.clear().unwrap();
    assert_eq!(algo.vector_index().len(), 0);

    // Searching without embedder returns error
    let search_res = algo.search("architecture", 5, Modality::Docs);
    assert!(search_res.is_err(), "expected error when embedder is missing");
}
