use super::*;
use groundcontrol_common::types::EdgeProvenance;

use crate::graph::KnowledgeGraph;
use crate::index::BM25Index;

#[test]
fn test_analyze_density_empty_graph() {
    let graph = KnowledgeGraph::new();
    let report = analyze_density(&graph, 5);
    assert_eq!(report.total_nodes, 0);
    assert_eq!(report.total_edges, 0);
}

#[test]
fn test_analyze_density_with_data() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge(
        "A",
        "B",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );
    graph.add_edge(
        "B",
        "C",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );
    graph.add_edge(
        "A",
        "C",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );
    // D is orphan (no edges to/from).
    graph.ensure_node("D");

    let report = analyze_density(&graph, 3);
    assert_eq!(report.total_nodes, 4);
    assert_eq!(report.total_edges, 3);
    assert!(report.density > 0.0);

    // D should be an orphan.
    assert!(report.orphans.contains(&"D".to_string()));

    // A should be a hub (highest degree: 2 outbound).
    assert!(!report.hubs.is_empty());
    assert_eq!(report.hubs[0].path, "A");
}

#[test]
fn test_find_semantic_gaps_disjoint() {
    use crate::vector_index::VectorIndex;
    use groundcontrol_common::types::Chunk;

    let mut bm25 = BM25Index::open_in_memory().unwrap();
    let mut vi = VectorIndex::new(4, 100, 200, 16);

    // BM25 has doc A, vector has doc B.
    let chunks = vec![Chunk::new("A", 0, "alpha beta gamma", 0, 16)];
    bm25.add_document("A", Some("Alpha"), &[], &chunks).unwrap();
    bm25.commit().unwrap();

    // Add B to vector index only.
    let vec_b: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0];
    vi.add(&vec_b, "B", Some(0), false, "docs").unwrap();

    let query_emb = vec![1.0, 0.0, 0.0, 0.0];
    let gaps = find_semantic_gaps(&bm25, &vi, &["alpha"], &[query_emb], 5).unwrap();

    assert_eq!(gaps.len(), 1);
    // BM25 finds A, vector finds B — no overlap.
    assert!(gaps[0].bm25_only.contains(&"A".to_string()));
    assert!(gaps[0].vector_only.contains(&"B".to_string()));
    assert_eq!(gaps[0].overlap_ratio, 0.0);
}

#[test]
fn test_coverage_report_identifies_dead_zones() {
    use groundcontrol_common::types::Chunk;

    let mut bm25 = BM25Index::open_in_memory().unwrap();

    let chunks_a = vec![Chunk::new("A", 0, "rust systems programming", 0, 24)];
    let chunks_b = vec![Chunk::new("B", 0, "python data science", 0, 19)];
    bm25.add_document("A", Some("Rust"), &[], &chunks_a).unwrap();
    bm25.add_document("B", Some("Python"), &[], &chunks_b).unwrap();
    bm25.commit().unwrap();

    let all_paths = vec!["A".to_string(), "B".to_string(), "C".to_string()];
    let queries = &["rust programming"];

    let report = coverage_report(&bm25, queries, &all_paths, 10).unwrap();

    assert_eq!(report.total_notes, 3);
    // Only A should be retrieved.
    assert!(report.covered_notes >= 1);
    // C is never indexed, so it's uncovered.
    assert!(report.uncovered_notes.contains(&"C".to_string()));
    // B might not be retrieved for "rust programming".
    assert!(report.coverage_ratio < 1.0);
}
