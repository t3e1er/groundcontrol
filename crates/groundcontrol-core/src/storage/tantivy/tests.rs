use super::*;
use groundcontrol_common::types::{Chunk, Modality};

/// Helper to create a simple chunk.
fn make_chunk(doc_path: &str, index: usize, text: &str) -> Chunk {
    Chunk::new(doc_path, index, text, 0, text.len())
}

#[test]
fn test_create_in_memory() {
    let index = BM25Index::open_in_memory();
    assert!(index.is_ok());
}

#[test]
fn test_add_and_search() {
    let mut index = BM25Index::open_in_memory().unwrap();

    let chunks1 = vec![make_chunk(
        "notes/rust.md",
        0,
        "Rust is a systems programming language focused on safety and performance",
    )];
    let chunks2 = vec![make_chunk(
        "notes/python.md",
        0,
        "Python is a dynamic interpreted language popular for data science",
    )];
    let chunks3 = vec![make_chunk(
        "notes/java.md",
        0,
        "Java is an object-oriented language that runs on the JVM",
    )];

    index
        .add_document(
            "notes/rust.md",
            Some("Rust Language"),
            &["rust".to_string(), "systems".to_string()],
            &chunks1,
        )
        .unwrap();
    index
        .add_document("notes/python.md", Some("Python Language"), &["python".to_string()], &chunks2)
        .unwrap();
    index
        .add_document("notes/java.md", Some("Java Language"), &["java".to_string()], &chunks3)
        .unwrap();
    index.commit().unwrap();

    let results = index.search("systems programming safety", 10).unwrap();
    assert!(!results.is_empty(), "Expected at least one result");
    assert_eq!(results[0].path, "notes/rust.md");
}

#[test]
fn test_remove_document() {
    let mut index = BM25Index::open_in_memory().unwrap();

    let chunks =
        vec![make_chunk("notes/remove_me.md", 0, "This document should be removed from the index")];
    index.add_document("notes/remove_me.md", Some("Remove Me"), &[], &chunks).unwrap();
    index.commit().unwrap();

    // Verify it's searchable first.
    let results = index.search("removed from the index", 10).unwrap();
    assert!(!results.is_empty());

    // Remove and commit.
    index.remove_document("notes/remove_me.md").unwrap();
    index.commit().unwrap();

    // Should no longer appear in results.
    let results = index.search("removed from the index", 10).unwrap();
    assert!(results.is_empty(), "Document should have been removed");
}

#[test]
fn test_search_by_title() {
    let mut index = BM25Index::open_in_memory().unwrap();

    let chunks = vec![make_chunk(
        "notes/kubernetes.md",
        0,
        "Container orchestration platform for deploying applications",
    )];
    index
        .add_document(
            "notes/kubernetes.md",
            Some("Kubernetes Deep Dive"),
            &["k8s".to_string()],
            &chunks,
        )
        .unwrap();
    index.commit().unwrap();

    // Search using title text — should find the document.
    let results = index.search("Kubernetes Deep Dive", 10).unwrap();
    assert!(!results.is_empty(), "Should find document by title");
    assert_eq!(results[0].path, "notes/kubernetes.md");
}

#[test]
fn test_search_multiple_results() {
    let mut index = BM25Index::open_in_memory().unwrap();

    let chunks1 = vec![make_chunk(
        "notes/ml_intro.md",
        0,
        "Machine learning is a subset of artificial intelligence that learns from data",
    )];
    let chunks2 = vec![make_chunk(
        "notes/ml_advanced.md",
        0,
        "Advanced machine learning covers deep learning neural networks and transformers",
    )];
    let chunks3 = vec![make_chunk(
        "notes/cooking.md",
        0,
        "This recipe explains how to make a perfect sourdough bread",
    )];

    index
        .add_document("notes/ml_intro.md", Some("ML Introduction"), &["ml".to_string()], &chunks1)
        .unwrap();
    index
        .add_document(
            "notes/ml_advanced.md",
            Some("Advanced ML"),
            &["ml".to_string(), "deep-learning".to_string()],
            &chunks2,
        )
        .unwrap();
    index
        .add_document(
            "notes/cooking.md",
            Some("Sourdough Recipe"),
            &["cooking".to_string()],
            &chunks3,
        )
        .unwrap();
    index.commit().unwrap();

    let results = index.search("machine learning", 10).unwrap();
    assert!(
        results.len() >= 2,
        "Expected at least 2 results for 'machine learning', got {}",
        results.len()
    );

    // Both ML documents should appear, cooking should not.
    let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&"notes/ml_intro.md"));
    assert!(paths.contains(&"notes/ml_advanced.md"));
    assert!(!paths.contains(&"notes/cooking.md"));

    // Results should be ordered by relevance (descending score).
    for window in results.windows(2) {
        assert!(window[0].score >= window[1].score, "Results should be in descending score order");
    }
}

#[test]
fn test_search_with_modality_filters() {
    let mut index = BM25Index::open_in_memory().unwrap();

    // A documentation chunk (default entity_kind = Documentation).
    let doc_chunk = make_chunk(
        "notes/guide.md",
        0,
        "The retrieval engine performs hybrid search over the corpus",
    );
    // A code chunk (entity_kind = CodeChunk via with_code_metadata).
    let code_chunk = Chunk::new(
        "src/engine.rs",
        0,
        "fn search_hybrid() performs hybrid retrieval over the corpus",
        0,
        60,
    )
    .with_code_metadata("rust", "crate::engine", 1, 3);

    index.add_document("notes/guide.md", Some("Guide"), &[], &[doc_chunk]).unwrap();
    index.add_document("src/engine.rs", Some("engine.rs"), &[], &[code_chunk]).unwrap();
    index.commit().unwrap();

    // Code modality returns only the code chunk.
    let code_results =
        index.search_with_modality("hybrid retrieval corpus", 10, Modality::Code).unwrap();
    assert!(!code_results.is_empty(), "expected code results");
    assert!(code_results.iter().all(|r| r.path == "src/engine.rs"));

    // Docs modality returns only the doc chunk.
    let doc_results =
        index.search_with_modality("hybrid search corpus", 10, Modality::Docs).unwrap();
    assert!(!doc_results.is_empty(), "expected doc results");
    assert!(doc_results.iter().all(|r| r.path == "notes/guide.md"));

    // Both returns both.
    let both_results = index.search_with_modality("hybrid corpus", 10, Modality::Both).unwrap();
    let paths: Vec<&str> = both_results.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&"notes/guide.md"));
    assert!(paths.contains(&"src/engine.rs"));

    // Default `search` is equivalent to Modality::Both.
    let default_results = index.search("hybrid corpus", 10).unwrap();
    assert_eq!(default_results.len(), both_results.len());
}

#[test]
fn test_lockfile_self_healing() {
    let temp = tempfile::TempDir::new().unwrap();
    let index_dir = temp.path().join("tantivy");
    std::fs::create_dir_all(&index_dir).unwrap();

    // Simulate an orphaned stale lock file left behind by a killed process
    let stale_lock = index_dir.join(".tantivy-writer.lock");
    std::fs::write(&stale_lock, b"stale lock content").unwrap();
    assert!(stale_lock.exists());

    // Opening BM25Index should detect and remove the stale lockfile
    let mut index = BM25Index::open(&index_dir).expect("should heal and open");

    // Verify we can write and commit
    let chunks = vec![make_chunk("doc.md", 0, "Test content")];
    index.add_document("doc.md", Some("Title"), &[], &chunks).unwrap();
    index.commit().unwrap();

    let res = index.search("content", 5).unwrap();
    assert_eq!(res.len(), 1);
}
