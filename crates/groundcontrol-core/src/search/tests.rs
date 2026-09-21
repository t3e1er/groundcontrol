//! Unit tests for search strategies.

use std::collections::HashSet;

use groundcontrol_common::types::{Chunk, EdgeProvenance, Modality, ScoreBreakdown, SearchResult};

use crate::graph::KnowledgeGraph;
use crate::index::BM25Index;

use super::bm25::search_bm25;
use super::explain::search_explain;
use super::fast::{search_explain_fast, search_fast};
use super::fusion::{path_matches_modality, rrf_fuse, rrf_fuse_cross_corpus};
use super::graph::{search_graph, search_related};
use super::hybrid::{search_hybrid, search_hybrid_full};
use super::multihop::decompose_query;
use super::semantic::search_semantic_with_embedding;

/// Helper to create a simple chunk.
fn make_chunk(doc_path: &str, index: usize, text: &str) -> Chunk {
    Chunk::new(doc_path, index, text, 0, text.len())
}

/// Helper to create a code chunk.
fn make_code_chunk(doc_path: &str, index: usize, text: &str) -> Chunk {
    Chunk::new(doc_path, index, text, 0, text.len()).with_code_metadata("rust", "", 1, 10)
}

/// An empty code-paths classifier set (all paths classify as docs).
fn no_code() -> HashSet<String> {
    HashSet::new()
}

/// Set up a BM25 index with some test documents.
/// Documents are designed so that specific queries yield predictable top results.
fn setup_bm25() -> BM25Index {
    let mut index = BM25Index::open_in_memory().unwrap();

    // rust.md mentions "systems programming" heavily — unique to this doc.
    let chunks = [
        make_chunk("notes/rust.md", 0, "Rust is a systems programming language. Systems programming in Rust focuses on safety and performance for systems-level code"),
        make_chunk("notes/async.md", 0, "Async concurrency uses futures and the tokio runtime for non-blocking IO"),
        make_chunk("notes/python.md", 0, "Python is a dynamic interpreted scripting language popular for data science and automation. Python scripting is easy to learn"),
        make_chunk("notes/ml.md", 0, "Machine learning uses neural networks and gradient descent for classification"),
        make_chunk("notes/web.md", 0, "Web development uses frameworks like actix-web and axum for HTTP servers"),
    ];

    index
        .add_document(
            "notes/rust.md",
            Some("Systems Programming"),
            &["rust".into(), "systems".into()],
            &chunks[0..1],
        )
        .unwrap();
    index
        .add_document("notes/async.md", Some("Async IO"), &["async".into()], &chunks[1..2])
        .unwrap();
    index
        .add_document(
            "notes/python.md",
            Some("Python Scripting"),
            &["python".into()],
            &chunks[2..3],
        )
        .unwrap();
    index
        .add_document("notes/ml.md", Some("Neural Networks"), &["ml".into()], &chunks[3..4])
        .unwrap();
    index
        .add_document("notes/web.md", Some("HTTP Servers"), &["web".into()], &chunks[4..5])
        .unwrap();
    index.commit().unwrap();

    index
}

/// Set up a knowledge graph with connections between test docs.
fn setup_graph() -> KnowledgeGraph {
    let mut graph = KnowledgeGraph::new();

    // Rust cluster: rust -> async, rust -> web
    graph.add_edge(
        "notes/rust.md",
        "notes/async.md",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );
    graph.add_edge(
        "notes/rust.md",
        "notes/web.md",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );
    graph.add_edge(
        "notes/async.md",
        "notes/web.md",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );

    // Python cluster: python -> ml
    graph.add_edge(
        "notes/python.md",
        "notes/ml.md",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );

    // Cross-cluster link: ml -> rust (ML uses Rust for performance)
    graph.add_edge(
        "notes/ml.md",
        "notes/rust.md",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );

    graph
}

#[test]
fn test_search_bm25() {
    let index = setup_bm25();

    let results = search_bm25(&index, "systems programming", 10, Modality::Both).unwrap();
    assert!(!results.is_empty(), "Expected BM25 results for 'systems programming'");

    // rust.md is the only doc that mentions "systems programming" heavily.
    assert_eq!(results[0].path, "notes/rust.md");

    // Scores should be in descending order.
    for window in results.windows(2) {
        assert!(window[0].score >= window[1].score);
    }
}

#[test]
fn test_search_hybrid() {
    let index = setup_bm25();
    let graph = setup_graph();

    // Search for "systems programming" — hybrid should boost graph-connected nodes.
    let results = search_hybrid(
        &index,
        &graph,
        "systems programming",
        10,
        2,
        None,
        None,
        Modality::Both,
        &no_code(),
    )
    .unwrap();
    assert!(!results.is_empty());

    // rust.md should be top (only doc with BM25 match for "systems programming").
    assert_eq!(results[0].path, "notes/rust.md");

    // Graph-connected nodes (async.md, web.md) should appear in results
    // because rust.md links to them, giving them graph_boost > 0.
    let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&"notes/async.md"), "async.md should appear via graph boost");
    assert!(paths.contains(&"notes/web.md"), "web.md should appear via graph boost");

    // Check that graph-boosted results have score_components set.
    for r in &results {
        if r.path == "notes/async.md" || r.path == "notes/web.md" {
            let components = r.score_components.as_ref().unwrap();
            assert!(components.graph_boost > 0.0, "{} should have graph_boost > 0", r.path);
        }
    }
}

#[test]
fn test_search_graph() {
    let index = setup_bm25();
    let graph = setup_graph();

    // Search for "Python scripting" — graph search finds nodes reachable from python.md.
    let results = search_graph(
        &index,
        &graph,
        "Python scripting",
        10,
        3,
        None,
        None,
        Modality::Both,
        &no_code(),
    )
    .unwrap();
    assert!(!results.is_empty());

    // python.md links to ml.md, and ml.md links to rust.md.
    let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&"notes/ml.md"), "ml.md should be reachable from python.md");

    // ml.md should have higher score (1 hop) than rust.md (2 hops).
    let ml_result = results.iter().find(|r| r.path == "notes/ml.md");
    let rust_result = results.iter().find(|r| r.path == "notes/rust.md");

    if let (Some(ml), Some(rust)) = (ml_result, rust_result) {
        assert!(ml.score > rust.score, "ml.md (1 hop) should score higher than rust.md (2 hops)");
    }

    // Seeds (python.md) should NOT appear in graph search results.
    assert!(!paths.contains(&"notes/python.md"), "seed should be excluded from results");
}

#[test]
fn test_search_related() {
    let graph = setup_graph();

    // Find notes related to rust.md.
    let seeds = vec!["notes/rust.md".to_string()];
    let results = search_related(&graph, &seeds, 10, 0.85, 20, Modality::Both, &no_code()).unwrap();
    assert!(!results.is_empty());

    // rust.md links to async.md and web.md directly (1 hop).
    let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&"notes/async.md"));
    assert!(paths.contains(&"notes/web.md"));

    // Seeds should not appear in results.
    assert!(!paths.contains(&"notes/rust.md"));

    // Direct neighbors (1 hop) should score higher than 2-hop neighbors.
    let async_result = results.iter().find(|r| r.path == "notes/async.md").unwrap();
    assert_eq!(async_result.score_components.as_ref().unwrap().graph_hops, Some(1));
}

#[test]
fn test_search_related_multi_seed() {
    let graph = setup_graph();

    // Multiple seeds: rust.md and python.md.
    let seeds = vec!["notes/rust.md".to_string(), "notes/python.md".to_string()];
    let results = search_related(&graph, &seeds, 10, 0.85, 20, Modality::Both, &no_code()).unwrap();
    assert!(!results.is_empty());

    // ml.md is reachable from python.md (1 hop) and from rust.md via longer path.
    let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&"notes/ml.md"));
    assert!(paths.contains(&"notes/async.md"));
    assert!(paths.contains(&"notes/web.md"));

    // Neither seed should appear.
    assert!(!paths.contains(&"notes/rust.md"));
    assert!(!paths.contains(&"notes/python.md"));
}

#[test]
fn test_path_matches_modality_classifier() {
    let mut code_paths = HashSet::new();
    let _ = code_paths.insert("src/engine.rs".to_string());
    let _ = code_paths.insert("crate::search::Engine".to_string());

    // Both accepts everything.
    assert!(path_matches_modality("src/engine.rs", Modality::Both, &code_paths));
    assert!(path_matches_modality("notes/guide.md", Modality::Both, &code_paths));

    // Code keeps only code-set paths.
    assert!(path_matches_modality("src/engine.rs", Modality::Code, &code_paths));
    assert!(path_matches_modality("crate::search::Engine", Modality::Code, &code_paths));
    assert!(!path_matches_modality("notes/guide.md", Modality::Code, &code_paths));

    // Docs keeps only non-code-set paths.
    assert!(path_matches_modality("notes/guide.md", Modality::Docs, &code_paths));
    assert!(!path_matches_modality("src/engine.rs", Modality::Docs, &code_paths));
}

#[test]
fn test_search_graph_modality_post_filter() {
    // Build a small graph: a doc seed linking to one doc node and one code node.
    let mut graph = KnowledgeGraph::new();
    graph.add_edge(
        "notes/design.md",
        "notes/related.md",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );
    graph.add_edge(
        "notes/design.md",
        "src/engine.rs",
        "documents",
        1.0,
        EdgeProvenance::DocumentsCode,
        groundcontrol_common::config::EdgeClass::Structural,
    );

    // Seed matches the design doc via BM25.
    let mut index = BM25Index::open_in_memory().unwrap();
    let chunks = vec![make_chunk("notes/design.md", 0, "design document about the engine")];
    index.add_document("notes/design.md", Some("Design"), &[], &chunks).unwrap();
    index.commit().unwrap();

    // Classifier: only src/engine.rs is code.
    let mut code_paths = HashSet::new();
    let _ = code_paths.insert("src/engine.rs".to_string());

    // Code modality keeps only the code node.
    let code_results =
        search_graph(&index, &graph, "design", 10, 3, None, None, Modality::Code, &code_paths)
            .unwrap();
    let code_paths_out: Vec<&str> = code_results.iter().map(|r| r.path.as_str()).collect();
    assert!(code_paths_out.contains(&"src/engine.rs"));
    assert!(!code_paths_out.contains(&"notes/related.md"));

    // Docs modality keeps only the doc node.
    let doc_results =
        search_graph(&index, &graph, "design", 10, 3, None, None, Modality::Docs, &code_paths)
            .unwrap();
    let doc_paths_out: Vec<&str> = doc_results.iter().map(|r| r.path.as_str()).collect();
    assert!(doc_paths_out.contains(&"notes/related.md"));
    assert!(!doc_paths_out.contains(&"src/engine.rs"));

    // Both keeps everything discovered.
    let both_results =
        search_graph(&index, &graph, "design", 10, 3, None, None, Modality::Both, &code_paths)
            .unwrap();
    let both_paths_out: Vec<&str> = both_results.iter().map(|r| r.path.as_str()).collect();
    assert!(both_paths_out.contains(&"src/engine.rs"));
    assert!(both_paths_out.contains(&"notes/related.md"));
}

#[test]
fn test_search_hybrid_empty_query() {
    let index = setup_bm25();
    let graph = setup_graph();

    // An empty or non-matching query should return empty results gracefully.
    // Note: tantivy may error on empty queries, so we test a non-matching term.
    let results = search_hybrid(
        &index,
        &graph,
        "xyznonexistent",
        10,
        2,
        None,
        None,
        Modality::Both,
        &no_code(),
    )
    .unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_search_hybrid_full_with_vectors() {
    use crate::vector_index::VectorIndex;

    let index = setup_bm25();
    let graph = setup_graph();

    // Set up a vector index with vectors for some documents.
    let mut vi = VectorIndex::new(384, 100, 200, 16);

    let make_vec = |seed: usize| -> Vec<f32> {
        let v: Vec<f32> = (0..384).map(|i| ((seed * 7 + i * 13) % 100) as f32 / 100.0).collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / norm).collect()
    };

    // Assign vectors: rust and python have similar vectors.
    let v_rust = make_vec(42);
    let v_python = make_vec(43); // very similar to rust
    let v_web = make_vec(200); // different

    vi.add(&v_rust, "notes/rust.md", Some(0), false, "docs").unwrap();
    vi.add(&v_python, "notes/python.md", Some(0), false, "docs").unwrap();
    vi.add(&v_web, "notes/web.md", Some(0), false, "docs").unwrap();

    // Search with the rust vector as query embedding.
    let results = search_hybrid_full(
        &index,
        &vi,
        &graph,
        "systems programming",
        Some(&v_rust),
        10,
        2,
        None,
        None,
        Modality::Both,
        &no_code(),
    )
    .unwrap();

    assert!(!results.is_empty());

    // rust.md should be top (has both BM25 and vector signal).
    assert_eq!(results[0].path, "notes/rust.md");

    // Results should have non-zero scores.
    for r in &results {
        assert!(r.score > 0.0, "{} should have score > 0", r.path);
    }

    // Graph-connected nodes should still appear.
    let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
    assert!(
        paths.contains(&"notes/async.md") || paths.contains(&"notes/web.md"),
        "Graph-connected nodes should appear"
    );
}

#[test]
fn test_search_hybrid_full_without_vectors() {
    use crate::vector_index::VectorIndex;

    let index = setup_bm25();
    let graph = setup_graph();
    let vi = VectorIndex::new_default(384); // empty vector index

    // Without query embedding, should still work (BM25+graph only).
    let results = search_hybrid_full(
        &index,
        &vi,
        &graph,
        "systems programming",
        None,
        10,
        2,
        None,
        None,
        Modality::Both,
        &no_code(),
    )
    .unwrap();

    assert!(!results.is_empty());

    // rust.md should appear (it's the BM25 match for "systems programming").
    let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&"notes/rust.md"), "rust.md should appear in results");

    // Results should be sorted by descending score.
    for window in results.windows(2) {
        assert!(window[0].score >= window[1].score);
    }
}

#[test]
fn test_rrf_fusion_merges_bm25_and_vector_for_same_file() {
    use crate::vector_index::VectorIndex;

    let mut bm25 = BM25Index::open_in_memory().unwrap();
    let chunk = make_code_chunk("src/auth/service.rs", 5, "pub fn authenticate() -> bool { true }");
    bm25.add_document("src/auth/service.rs", None, &[], &[chunk]).unwrap();
    bm25.commit().unwrap();

    let graph = KnowledgeGraph::new();

    let mut vi = VectorIndex::new(384, 100, 200, 16);
    let make_vec = |seed: usize| -> Vec<f32> {
        let v: Vec<f32> = (0..384).map(|i| ((seed * 7 + i * 13) % 100) as f32 / 100.0).collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / norm).collect()
    };
    let q_vec = make_vec(42);
    // Vector stored with chunk_index = 0 (file skeleton aggregate)
    vi.add(&q_vec, "src/auth/service.rs", Some(0), false, "code").unwrap();

    let mut code_paths = HashSet::new();
    code_paths.insert("src/auth/service.rs".to_string());

    let results = search_hybrid_full(
        &bm25,
        &vi,
        &graph,
        "authenticate",
        Some(&q_vec),
        10,
        2,
        None,
        None,
        Modality::Code,
        &code_paths,
    )
    .unwrap();

    assert_eq!(results.len(), 1, "BM25 and vector results for same file must fuse into one entry");
    assert_eq!(results[0].path, "src/auth/service.rs");
    assert_eq!(
        results[0].chunk_index,
        Some(5),
        "Representative chunk should be the BM25 matched symbol"
    );
    let components = results[0].score_components.as_ref().unwrap();
    assert!(components.bm25 > 0.0, "BM25 score must be non-zero");
    assert!(components.vector > 0.0, "Vector score must be non-zero");
}

#[test]
fn test_rrf_best_bm25_chunk_selected() {
    use crate::vector_index::VectorIndex;

    let mut bm25 = BM25Index::open_in_memory().unwrap();
    let chunks = vec![
        make_code_chunk("src/auth/service.rs", 1, "pub fn login() { auth(); }"),
        make_code_chunk(
            "src/auth/service.rs",
            5,
            "pub fn authenticate_token_fast() { auth(); auth(); auth(); }",
        ),
    ];
    bm25.add_document("src/auth/service.rs", None, &[], &chunks).unwrap();
    bm25.commit().unwrap();

    let graph = KnowledgeGraph::new();
    let vi = VectorIndex::new_default(384);

    let mut code_paths = HashSet::new();
    code_paths.insert("src/auth/service.rs".to_string());

    let results = search_hybrid_full(
        &bm25,
        &vi,
        &graph,
        "auth",
        None,
        10,
        2,
        None,
        None,
        Modality::Code,
        &code_paths,
    )
    .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, "src/auth/service.rs");
    assert_eq!(results[0].chunk_index, Some(5), "Best BM25 scoring chunk must be selected");
}

#[test]
fn test_search_related_empty_seeds() {
    let graph = setup_graph();

    let results = search_related(&graph, &[], 10, 0.85, 20, Modality::Both, &no_code()).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_search_explain_returns_breakdowns() {
    use crate::vector_index::VectorIndex;

    let index = setup_bm25();
    let graph = setup_graph();
    let vi = VectorIndex::new_default(384); // no vectors, tests BM25+graph explain

    let explanations = search_explain(
        &index,
        &vi,
        &graph,
        "systems programming",
        None,
        10,
        2,
        None,
        None,
        Modality::Both,
        &no_code(),
    )
    .unwrap();

    assert!(!explanations.is_empty());

    // rust.md should appear since it matches "systems programming" in BM25.
    let rust_exp = explanations.iter().find(|e| e.path == "notes/rust.md");
    assert!(rust_exp.is_some(), "rust.md should appear in explanations");

    let rust = rust_exp.unwrap();
    // Should have BM25 signal.
    assert!(rust.bm25.raw_score > 0.0, "rust.md should have BM25 score");
    assert!(rust.bm25.rank > 0, "rust.md should have BM25 rank");
    assert!(rust.bm25.rrf_contribution > 0.0, "rust.md should have BM25 RRF contribution");

    // Vector signal should be zero (no embeddings).
    assert_eq!(rust.vector.raw_score, 0.0);
    assert_eq!(rust.vector.rank, 0);

    // Final score should be sum of RRF contributions.
    let expected_score =
        rust.bm25.rrf_contribution + rust.vector.rrf_contribution + rust.graph.rrf_contribution;
    assert!((rust.final_score - expected_score).abs() < 1e-10);

    // Results should be sorted by descending final_score.
    for window in explanations.windows(2) {
        assert!(window[0].final_score >= window[1].final_score);
    }
}

#[test]
fn test_search_graph_with_edge_filter() {
    let mut graph = KnowledgeGraph::new();

    // Set up two types of edges.
    graph.add_edge(
        "A",
        "B",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );
    graph.add_edge(
        "A",
        "C",
        "SharedTag",
        0.5,
        EdgeProvenance::SharedTag,
        groundcontrol_common::config::EdgeClass::Semantic,
    );
    graph.add_edge(
        "B",
        "D",
        "Link",
        1.0,
        EdgeProvenance::Wikilink,
        groundcontrol_common::config::EdgeClass::Structural,
    );

    // Create a BM25 index that matches "A".
    let mut index = BM25Index::open_in_memory().unwrap();
    let chunks = vec![make_chunk("A", 0, "Alpha document about testing")];
    index.add_document("A", Some("Alpha"), &[], &chunks).unwrap();
    index.commit().unwrap();

    // With edge filter for "Link" only, should find B and D but not C.
    let filter = vec!["Link".to_string()];
    let results = search_graph(
        &index,
        &graph,
        "Alpha",
        10,
        3,
        Some(&filter),
        None,
        Modality::Both,
        &no_code(),
    )
    .unwrap();
    let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();

    assert!(paths.contains(&"B"));
    assert!(paths.contains(&"D"));
    assert!(!paths.contains(&"C"), "C should be excluded by edge type filter");
}

#[test]
fn test_search_semantic_with_embedding() {
    use crate::vector_index::VectorIndex;

    // Create a vector index with some test vectors.
    let mut vi = VectorIndex::new(384, 100, 200, 16);

    // Helper to make a deterministic normalized vector.
    let make_vec = |seed: usize| -> Vec<f32> {
        let v: Vec<f32> = (0..384).map(|i| ((seed * 7 + i * 13) % 100) as f32 / 100.0).collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / norm).collect()
    };

    let v_rust = make_vec(42);
    let v_python = make_vec(99);
    let v_java = make_vec(200);

    vi.add(&v_rust, "notes/rust.md", Some(0), false, "docs").unwrap();
    vi.add(&v_python, "notes/python.md", Some(0), false, "docs").unwrap();
    vi.add(&v_java, "notes/java.md", Some(0), false, "docs").unwrap();

    // Search with the rust vector — should find rust.md first.
    let results = search_semantic_with_embedding(&vi, &v_rust, 3, false, Modality::Both).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].path, "notes/rust.md");
    // Should have vector score > 0 in components.
    assert!(results[0].score_components.as_ref().unwrap().vector > 0.0);

    // BM25 and graph should be 0.
    assert_eq!(results[0].score_components.as_ref().unwrap().bm25, 0.0);
    assert_eq!(results[0].score_components.as_ref().unwrap().graph_boost, 0.0);
}

#[test]
fn test_search_semantic_empty_index() {
    use crate::vector_index::VectorIndex;

    let vi = VectorIndex::new_default(384);
    let query_vec = vec![0.1_f32; 384];

    let results =
        search_semantic_with_embedding(&vi, &query_vec, 10, false, Modality::Both).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_rrf_fuse_merges_lists() {
    // Test the RRF fusion logic directly.
    let list1 = vec![
        SearchResult::new("A", 0.9).with_chunk_index(Some(0)).with_score_components(
            ScoreBreakdown { bm25: 0.0, vector: 0.9, graph_boost: 0.0, graph_hops: None },
        ),
        SearchResult::new("B", 0.7).with_chunk_index(Some(0)).with_score_components(
            ScoreBreakdown { bm25: 0.0, vector: 0.7, graph_boost: 0.0, graph_hops: None },
        ),
    ];
    let list2 = vec![
        SearchResult::new("B", 0.95).with_score_components(ScoreBreakdown {
            bm25: 0.0,
            vector: 0.95,
            graph_boost: 0.0,
            graph_hops: None,
        }),
        SearchResult::new("C", 0.8).with_score_components(ScoreBreakdown {
            bm25: 0.0,
            vector: 0.8,
            graph_boost: 0.0,
            graph_hops: None,
        }),
    ];

    let fused = rrf_fuse(&[&list1, &list2], 5);

    // B appears in both lists, so it should have the highest RRF score.
    assert!(!fused.is_empty());
    assert_eq!(fused[0].path, "B", "B should be top since it appears in both lists");

    // All three docs should appear.
    let paths: Vec<&str> = fused.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&"A"));
    assert!(paths.contains(&"B"));
    assert!(paths.contains(&"C"));

    // RRF scores should be in descending order.
    for window in fused.windows(2) {
        assert!(window[0].score >= window[1].score);
    }
}

#[test]
fn test_search_depth_precise_only_chunks() {
    use crate::vector_index::VectorIndex;

    let mut vi = VectorIndex::new(384, 100, 200, 16);

    let make_vec = |seed: usize| -> Vec<f32> {
        let v: Vec<f32> = (0..384).map(|i| ((seed * 7 + i * 13) % 100) as f32 / 100.0).collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / norm).collect()
    };

    // Add chunk-level and doc-level vectors for the same document.
    let v_chunk = make_vec(10);
    let v_doc = make_vec(20);

    vi.add(&v_chunk, "notes/a.md", Some(0), false, "docs").unwrap();
    vi.add(&v_doc, "notes/a.md", None, true, "docs").unwrap();

    // Search with precise (chunk only) — should not return doc-level.
    let results = search_semantic_with_embedding(&vi, &v_chunk, 10, false, Modality::Both).unwrap();
    // Should find both since doc_level_only=false doesn't exclude chunks
    assert!(!results.is_empty());

    // Search with broad (doc only) — should only return doc-level.
    let results_broad =
        search_semantic_with_embedding(&vi, &v_doc, 10, true, Modality::Both).unwrap();
    for r in &results_broad {
        // All results from doc_level_only=true should not have chunk_index
        assert!(r.chunk_index.is_none(), "broad mode should return doc-level only");
    }
}

#[test]
fn test_decompose_query_multi_concepts() {
    // "How does prompt engineering relate to agent architecture and tool use?"
    let result =
        decompose_query("How does prompt engineering relate to agent architecture and tool use?");
    assert_eq!(
        result,
        vec![
            "prompt engineering".to_string(),
            "agent architecture".to_string(),
            "tool use?".to_string(),
        ]
    );
}

#[test]
fn test_decompose_query_connect_pattern() {
    // "How do embeddings connect RAG to knowledge graphs?"
    let result = decompose_query("How do embeddings connect RAG to knowledge graphs?");
    assert_eq!(
        result,
        vec!["embeddings".to_string(), "RAG".to_string(), "knowledge graphs?".to_string(),]
    );
}

#[test]
fn test_decompose_query_single_concept() {
    // A simple query with no connecting words should return one concept.
    let result = decompose_query("rust programming language");
    assert_eq!(result, vec!["rust programming language".to_string()]);
}

#[test]
fn test_decompose_query_strips_prefix() {
    // "What is machine learning?" should strip "What is" prefix.
    let result = decompose_query("What is machine learning?");
    assert_eq!(result, vec!["machine learning?".to_string()]);
}

#[test]
fn test_lineage_enrichment_in_search() {
    let index = setup_bm25();
    let mut graph = setup_graph();

    // Add structural edge: notes/async.md supersedes notes/rust.md
    graph.add_edge(
        "notes/async.md",
        "notes/rust.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        groundcontrol_common::config::EdgeClass::Structural,
    );

    let results = search_hybrid(
        &index,
        &graph,
        "systems programming",
        5,
        2,
        None,
        None,
        Modality::Both,
        &no_code(),
    )
    .unwrap();
    assert!(!results.is_empty());

    let rust_result = results.iter().find(|r| r.path == "notes/rust.md").unwrap();
    assert!(rust_result.lineage.is_some(), "SearchResult for rust.md should have lineage");
    let lineage = rust_result.lineage.as_ref().unwrap();
    assert_eq!(lineage.superseded_by, vec!["notes/async.md"]);
}

#[test]
fn test_rrf_fuse_cross_corpus_merges_ranks_and_tags() {
    // Two per-corpus ranked lists, built by hand (no embedder, no manager).
    // Corpus "docs": shared.md is rank 1 (strongest), a.md rank 2.
    // Corpus "code": shared.md is rank 1, b.md rank 2.
    let docs = vec![
        SearchResult::new("shared.md", 9.0).with_snippet(Some("docs shared".into())),
        SearchResult::new("a.md", 8.0),
    ];
    let code = vec![
        SearchResult::new("shared.md", 7.0).with_language("rust").with_chunk_index(Some(3)),
        SearchResult::new("b.md", 6.0),
    ];

    let tagged = vec![("docs".to_string(), docs), ("code".to_string(), code)];
    let fused = rrf_fuse_cross_corpus(&tagged, 10);

    // (b) same path in two corpora stays as two distinct hits.
    let shared_hits: Vec<&SearchResult> = fused.iter().filter(|r| r.path == "shared.md").collect();
    assert_eq!(shared_hits.len(), 2, "shared.md must appear once per corpus");

    // (c) every result is tagged with its origin corpus.
    assert!(fused.iter().all(|r| r.corpus.is_some()), "all hits must be corpus-tagged");
    let docs_shared =
        fused.iter().find(|r| r.path == "shared.md" && r.corpus.as_deref() == Some("docs"));
    let code_shared =
        fused.iter().find(|r| r.path == "shared.md" && r.corpus.as_deref() == Some("code"));
    assert!(docs_shared.is_some(), "docs/shared.md must be present and tagged 'docs'");
    assert!(code_shared.is_some(), "code/shared.md must be present and tagged 'code'");

    // Rich fields are preserved from the source result.
    assert_eq!(docs_shared.unwrap().snippet.as_deref(), Some("docs shared"));
    assert_eq!(code_shared.unwrap().language.as_deref(), Some("rust"));
    assert_eq!(code_shared.unwrap().chunk_index, Some(3));

    // (a) RRF ranks: both shared.md hits are rank-1 in their lists → identical RRF
    // score, and each strictly beats the rank-2 hit from the same corpus.
    let a_hit = fused.iter().find(|r| r.path == "a.md").unwrap();
    assert!(docs_shared.unwrap().score > a_hit.score, "rank-1 shared.md must outrank rank-2 a.md");
    // Top result overall is a shared.md hit.
    assert_eq!(fused[0].path, "shared.md");
}

#[test]
fn test_search_fast_and_explain() {
    use crate::search::binary::BinarySearchIndex;
    use groundcontrol_common::types::BinaryFingerprint;

    let mut bm25 = BM25Index::open_in_memory().unwrap();
    let chunks = [
        make_code_chunk("src/auth/service.rs", 1, "pub fn authenticate_token() { verify_jwt(); }"),
        make_code_chunk("src/auth/helper.rs", 1, "pub fn helper_call() { debug(); }"),
    ];
    bm25.add_document("src/auth/service.rs", None, &[], &chunks[0..1]).unwrap();
    bm25.add_document("src/auth/helper.rs", None, &[], &chunks[1..2]).unwrap();
    bm25.commit().unwrap();

    let mut binary = BinarySearchIndex::new();
    // Insert binary fingerprints
    let fp_service = BinaryFingerprint([0xF0F0_0000_0000_0000, 0, 0, 0]);
    let fp_helper = BinaryFingerprint([0x0000_0000_0000_0000, 0, 0, 0]);
    let records = vec![
        groundcontrol_common::types::FingerprintRecord {
            id: "src/auth/service.rs".to_string(),
            fingerprint: fp_service,
            modality: Modality::Code,
        },
        groundcontrol_common::types::FingerprintRecord {
            id: "src/auth/helper.rs".to_string(),
            fingerprint: fp_helper,
            modality: Modality::Code,
        },
    ];
    binary.index_fingerprints(&records).unwrap();

    let mut graph = KnowledgeGraph::new();
    graph.add_node("src/auth/service.rs", None);
    graph.add_node("src/auth/helper.rs", None);
    graph.add_edge(
        "src/auth/service.rs",
        "src/auth/helper.rs",
        "calls",
        1.0,
        EdgeProvenance::CodeCalls,
        groundcontrol_common::config::EdgeClass::Code,
    );

    let mut code_paths = HashSet::new();
    code_paths.insert("src/auth/service.rs".to_string());
    code_paths.insert("src/auth/helper.rs".to_string());

    let results = search_fast(
        &bm25,
        &binary,
        &graph,
        "authenticate_token",
        5,
        Modality::Code,
        None,
        &code_paths,
    )
    .unwrap();

    assert!(!results.is_empty(), "Fast search should return results");
    assert_eq!(results[0].path, "src/auth/service.rs");
    let comp = results[0].score_components.as_ref().unwrap();
    assert!(comp.bm25 > 0.0);

    // Test search_explain_fast
    let explanations = search_explain_fast(
        &bm25,
        &binary,
        &graph,
        "authenticate_token",
        5,
        Modality::Code,
        None,
        &code_paths,
    )
    .unwrap();

    assert!(!explanations.is_empty());
    assert_eq!(explanations[0].path, "src/auth/service.rs");
    assert!(explanations[0].bm25.raw_score > 0.0);
    assert!(explanations[0].bm25.rrf_contribution > 0.0);
}
