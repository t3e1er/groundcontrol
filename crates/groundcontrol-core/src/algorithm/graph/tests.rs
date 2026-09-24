//! Tests for graph / PPR retrieval algorithm component.

use super::*;
use groundcontrol_common::types::{Edge, EdgeProvenance, FileFormat, Modality, ParsedArtifact};

#[test]
fn test_graph_lifecycle_and_ppr() {
    let graph = KnowledgeGraph::new();
    let mut algo = GraphAlgorithm::new(graph);

    let doc = ParsedArtifact {
        path: "src/auth.rs".to_string(),
        hash: "hash_auth".to_string(),
        is_code: true,
        format: FileFormat::Source,
        title: Some("auth.rs".to_string()),
        doc_metadata: None,
        symbols: Vec::new(),
        grammar_semantics: Vec::new(),
        chunks: Vec::new(),
        graph_edges: vec![Edge::new(
            "src/auth.rs",
            "src/token.rs",
            "calls",
            1.0,
            EdgeProvenance::CodeCalls,
        )],
        external_refs: Vec::new(),
        raw_content: None,
        projection_text: None,
    };

    algo.index_document(&doc).unwrap();

    let hits = algo.search("auth", 5, Modality::Both).unwrap();
    assert!(!hits.is_empty(), "expected graph/ppr hits for 'auth'");

    algo.remove_document("src/auth.rs").unwrap();
    let hits_after = algo.search("auth", 5, Modality::Both).unwrap();
    assert!(hits_after.is_empty(), "expected zero hits after removal");
}
