//! Unit tests for Engine indexing, lifecycle, delta scanning, and polyglot search.

use std::fs;
use std::path::Path;
use tempfile::TempDir;

use ctxvault_common::config::CorpusConfig;
use ctxvault_common::ports::{GraphStore, MetadataCatalog};
use ctxvault_common::types::{CodeSymbolType, IndexingStatus};

use crate::engine::Engine;
use crate::vector_index::VectorIndex;

/// Create a minimal corpus config pointing at the given path.
fn test_config(corpus_path: &Path) -> CorpusConfig {
    CorpusConfig {
        name: "test".to_string(),
        path: corpus_path.to_string_lossy().to_string(),
        mode: ctxvault_common::config::CorpusMode::ReadWrite,
        index_mode: ctxvault_common::config::IndexMode::Full,
        chunking: ctxvault_common::config::ChunkingConfig {
            min_chunk_tokens: 1, // very low for tests
            ..Default::default()
        },
        embedding: ctxvault_common::config::EmbeddingConfig::default(),
        graph: ctxvault_common::config::GraphConfig {
            edge_types: vec![ctxvault_common::config::EdgeTypeConfig {
                name: "Wikilink".to_string(),
                source: ctxvault_common::config::EdgeSource::Wikilink,
                weight: 1.0,
                bidirectional: false,
                field: None,
                direction: None,
                max_frequency: None,
                class: None,
                description: None,
                allowed_source_templates: None,
                allowed_target_templates: None,
            }],
        },
        templates_dir: None,
        exclude: ctxvault_common::config::ExcludeConfig::default(),
        docs: ctxvault_common::config::DocsConfig::default(),
    }
}

#[test]
fn test_fast_mode_skips_vectors_and_embedder() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    fs::write(corpus_dir.join("file1.md"), "# File 1\nSome test markdown content").unwrap();

    let mut config = test_config(&corpus_dir);
    config.index_mode = ctxvault_common::config::IndexMode::Fast;

    let index_dir = tmp.path().join("index");
    let mut engine = Engine::open(config, &index_dir).unwrap();
    assert!(engine.is_fast_mode());
    assert!(!engine.has_vector_index());
    assert_eq!(engine.ensure_embedder().unwrap(), false);

    let files = engine.full_reindex_paginated(10, false).unwrap();
    assert_eq!(files, 1);
    assert!(!engine.has_vector_index());
    assert_eq!(engine.store().list_files().unwrap().len(), 1);
}

#[test]
fn test_full_mode_docs_vector_code_hamming() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();

    fs::write(
        corpus_dir.join("guide.md"),
        "# Architecture Guide\n\n## Overview\n\nFull mode indexes docs into dense vector store and code into binary hamming.\n",
    )
    .unwrap();

    fs::write(
        corpus_dir.join("lib.rs"),
        "pub struct EngineConfig {\n    pub name: String,\n}\n\npub fn run_engine() {}\n",
    )
    .unwrap();

    let mut config = test_config(&corpus_dir);
    config.index_mode = ctxvault_common::config::IndexMode::Full;

    let index_dir = tmp.path().join("index");
    let mut engine = Engine::open(config, &index_dir).unwrap();

    assert!(!engine.is_fast_mode());
    assert!(engine.has_vector_index());

    let files_indexed = engine.full_reindex_paginated(10, false).unwrap();
    assert_eq!(files_indexed, 2);

    // Verify SQLite store contains both files, chunks, and symbols
    assert_eq!(engine.store().list_files().unwrap().len(), 2);
    let code_symbols = engine.store().find_symbols_by_name("EngineConfig").unwrap();
    assert!(!code_symbols.is_empty());

    // Verify BM25 indexed both files
    let bm25_doc = engine.bm25.search("Architecture", 10).unwrap();
    assert!(!bm25_doc.is_empty());
    let bm25_code = engine.bm25.search("EngineConfig", 10).unwrap();
    assert!(!bm25_code.is_empty());

    // Verify Graph indexed both
    assert!(engine.graph().node_count() >= 2);

    // Staged chunk generation: code chunk pending vector queue must be empty
    let (code_pending, _) = engine.index_file_staged("src/main.rs", "pub struct Foo;").unwrap();
    assert!(code_pending.is_empty(), "Code chunks must not be staged for vector embedding");

    // Staged chunk generation: doc chunk pending vector queue must contain chunks
    let (doc_pending, _) = engine
        .index_file_staged("readme.md", "# Readme\n\n## Overview\n\nAnchor content.")
        .unwrap();
    let has_anchor = doc_pending
        .iter()
        .any(|c| c.embed_policy == ctxvault_common::types::ChunkEmbedPolicy::Anchor);
    assert!(has_anchor, "Doc chunks must be staged for vector embedding in Full mode");

    // Verify binary index has entries for code
    assert!(!engine.binary_index().is_empty());
}

#[test]
fn test_open_creates_index_dir() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    let config = test_config(&corpus_dir);
    let _engine = Engine::open(config, &index_dir).unwrap();

    // Verify index directory structure was created.
    assert!(index_dir.exists());
    assert!(index_dir.join("meta.db").exists());
    assert!(index_dir.join("tantivy").exists());
}

#[test]
fn test_index_and_search() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config, &index_dir).unwrap();

    let content =
        "# Rust Programming\n\nRust is a systems programming language focused on safety.\n";
    engine.index_file("rust.md", content).unwrap();
    engine.commit().unwrap();

    // Verify searchable via BM25.
    let results = engine.bm25.search("systems programming", 10).unwrap();
    assert!(!results.is_empty(), "Should find indexed file via search");
    assert_eq!(results[0].path, "rust.md");

    // Verify stored in persistence.
    let file = engine.store().get_file("rust.md").unwrap();
    assert!(file.is_some());
    assert_eq!(file.unwrap().title.as_deref(), Some("Rust Programming"));
}

#[test]
fn test_remove_file() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config, &index_dir).unwrap();

    let content = "# To Remove\n\nThis note will be removed.\n";
    engine.index_file("remove_me.md", content).unwrap();
    engine.commit().unwrap();

    // Confirm it's indexed.
    assert!(engine.store().get_file("remove_me.md").unwrap().is_some());

    // Remove it.
    engine.remove_file("remove_me.md").unwrap();
    engine.commit().unwrap();

    // Verify gone from all stores.
    assert!(engine.store().get_file("remove_me.md").unwrap().is_none());
    let results = engine.bm25.search("removed", 10).unwrap();
    assert!(results.iter().all(|r| r.path != "remove_me.md"), "File should be gone from BM25");
    assert!(!engine.graph().contains_node("remove_me.md"));
}

#[test]
fn test_delta_scan() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    // Create initial files.
    fs::write(corpus_dir.join("existing.md"), "# Existing\n\nOriginal content.\n").unwrap();
    fs::write(corpus_dir.join("will_modify.md"), "# Will Modify\n\nOriginal.\n").unwrap();
    fs::write(corpus_dir.join("will_delete.md"), "# Will Delete\n\nGoing away.\n").unwrap();

    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config, &index_dir).unwrap();

    // Initial full index.
    let count = engine.full_reindex().unwrap();
    assert_eq!(count, 3);

    // Now modify one file, delete one, and add a new one.
    fs::write(
        corpus_dir.join("will_modify.md"),
        "# Will Modify\n\nUpdated content that is different.\n",
    )
    .unwrap();
    fs::remove_file(corpus_dir.join("will_delete.md")).unwrap();
    fs::write(corpus_dir.join("new_file.md"), "# New File\n\nBrand new.\n").unwrap();

    // Run delta scan.
    let result = engine.delta_scan().unwrap();

    assert_eq!(result.new_files, vec!["new_file.md"]);
    assert_eq!(result.modified_files, vec!["will_modify.md"]);
    assert_eq!(result.deleted_files, vec!["will_delete.md"]);

    // Verify the new file is searchable.
    let search = engine.bm25.search("brand new", 10).unwrap();
    assert!(search.iter().any(|r| r.path == "new_file.md"));

    // Verify deleted file is gone.
    assert!(engine.store().get_file("will_delete.md").unwrap().is_none());
}

#[test]
fn test_full_reindex() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    fs::write(corpus_dir.join("alpha.md"), "# Alpha\n\nFirst note.\n").unwrap();
    fs::write(corpus_dir.join("beta.md"), "# Beta\n\nSecond note.\n").unwrap();
    fs::write(corpus_dir.join("gamma.md"), "# Gamma\n\nThird note with [[alpha]] link.\n").unwrap();

    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config, &index_dir).unwrap();

    let count = engine.full_reindex().unwrap();
    assert_eq!(count, 3);

    // All files should be in the store.
    let files = engine.store().list_files().unwrap();
    assert_eq!(files.len(), 3);

    // Graph should have the wikilink edge from gamma to alpha.
    let fwd = engine.graph().forwardlinks("gamma.md", None);
    let targets = fwd.get("Wikilink").unwrap_or(&Vec::new()).clone();
    assert!(targets.contains(&"alpha".to_string()), "Expected wikilink edge from gamma to alpha");

    // Verify search works.
    let results = engine.bm25.search("Second note", 10).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].path, "beta.md");
}

#[test]
fn test_model_version_set_on_new_index() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    let config = test_config(&corpus_dir);
    let engine = Engine::open(config, &index_dir).unwrap();

    // New empty vector index should not be stale.
    assert!(!engine.vectors_stale());
}

#[test]
fn test_model_version_mismatch_marks_stale() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    // Create a vector index file with a different model version.
    fs::create_dir_all(&index_dir).unwrap();
    let mut vi = VectorIndex::new_default(768);
    vi.set_model_version("some-other-model-v99");
    vi.add(&vec![0.1f32; 768], "test.md", Some(0), false, "text").unwrap();
    vi.save_binary(&index_dir.join("vectors.bin")).unwrap();

    let config = test_config(&corpus_dir);
    let engine = Engine::open(config, &index_dir).unwrap();

    // Should be marked stale due to version mismatch.
    assert!(engine.vectors_stale());
    assert_eq!(engine.stored_model_version(), Some("some-other-model-v99"));
}

#[test]
fn test_model_version_no_version_marks_stale() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    // Create a vector index file WITHOUT model_version.
    fs::create_dir_all(&index_dir).unwrap();
    let mut vi = VectorIndex::new_default(768);
    vi.add(&vec![0.1f32; 768], "test.md", Some(0), false, "text").unwrap();
    vi.save_binary(&index_dir.join("vectors.bin")).unwrap();

    let config = test_config(&corpus_dir);
    let engine = Engine::open(config, &index_dir).unwrap();

    // Vectors with no model_version with data should be marked stale.
    assert!(engine.vectors_stale());
}

#[test]
fn test_corpus_config_persistence() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    let config = test_config(&corpus_dir);
    let engine = Engine::open(config, &index_dir).unwrap();

    // Set and get config.
    engine.store().set_config("embedding_model", "all-minilm-l6-v2").unwrap();
    let value = engine.store().get_config("embedding_model").unwrap();
    assert_eq!(value, Some("all-minilm-l6-v2".to_string()));

    // Non-existent key returns None.
    let missing = engine.store().get_config("nonexistent").unwrap();
    assert_eq!(missing, None);
}

#[test]
fn test_paginated_reindex_and_status() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    // Write 15 files
    for i in 0..15 {
        fs::write(
            corpus_dir.join(format!("doc_{:02}.md", i)),
            format!("# Document {}\n\nContent for note {}\n", i, i),
        )
        .unwrap();
    }

    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config, &index_dir).unwrap();

    // Index in batches of 5
    let count = engine.full_reindex_paginated(5, false).unwrap();
    assert_eq!(count, 15);

    // Check indexing status
    let status = engine.get_indexing_status().unwrap();
    assert_eq!(status.corpus_id, "test");
    assert_eq!(status.status, IndexingStatus::Completed);
    assert_eq!(status.total_files, 15);
    assert_eq!(status.indexed_files, 15);
    assert_eq!(status.progress_percent, 100.0);
}

#[test]
fn test_indexing_resumption() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");

    // Write 10 files
    for i in 0..10 {
        fs::write(
            corpus_dir.join(format!("doc_{:02}.md", i)),
            format!("# Document {}\n\nContent for note {}\n", i, i),
        )
        .unwrap();
    }

    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config.clone(), &index_dir).unwrap();

    // 1. First index all 10 files
    let count = engine.full_reindex_paginated(4, false).unwrap();
    assert_eq!(count, 10);

    // 2. Add 5 more files to corpus
    for i in 10..15 {
        fs::write(
            corpus_dir.join(format!("doc_{:02}.md", i)),
            format!("# Document {}\n\nContent for note {}\n", i, i),
        )
        .unwrap();
    }

    // 3. Open fresh engine instance and resume indexing
    let mut resumed_engine = Engine::open(config, &index_dir).unwrap();
    let resumed_count = resumed_engine.full_reindex_paginated(4, true).unwrap();
    assert_eq!(resumed_count, 15);

    let files = resumed_engine.store().list_files().unwrap();
    assert_eq!(files.len(), 15);
}

#[test]
fn test_polyglot_codebase_indexing_and_cross_modal_search() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(corpus_dir.join("docs/adr")).unwrap();
    fs::create_dir_all(corpus_dir.join("src")).unwrap();
    fs::create_dir_all(corpus_dir.join("scripts")).unwrap();
    let index_dir = tmp.path().join("index");

    // 1. Write markdown ADR
    let adr_content = r#"---
title: ADR-0001 Hybrid Search
tags: [search, rrf, architecture]
---
# ADR-0001: Reciprocal Rank Fusion Search

We implement 4-way RRF hybrid search combining BM25, embeddings, and graph traversal.
"#;
    fs::write(corpus_dir.join("docs/adr/0001-hybrid-search.md"), adr_content).unwrap();

    // 2. Write Rust file
    let rust_code = r#"
/// Search engine implementation
pub struct Engine;

impl Engine {
    /// Execute hybrid search across all modalities
    pub fn search_hybrid(&self, query: &str) -> Vec<String> {
        let results = execute_rrf(query);
        results
    }
}

pub fn execute_rrf(q: &str) -> Vec<String> {
    vec![q.to_string()]
}
"#;
    fs::write(corpus_dir.join("src/search.rs"), rust_code).unwrap();

    // 3. Write TypeScript file
    let ts_code = r#"
export interface UserProfile {
    id: string;
    email: string;
}

export class UserService {
    /** Fetch user by ID */
    async getUser(id: string): Promise<UserProfile> {
        return { id, email: "user@example.com" };
    }
}
"#;
    fs::write(corpus_dir.join("src/user.ts"), ts_code).unwrap();

    // 4. Write Python script
    let py_code = r#"
class DataIngest:
    """Batch data ingestion pipeline."""
    def run_pipeline(self, batch):
        return len(batch)
"#;
    fs::write(corpus_dir.join("scripts/process.py"), py_code).unwrap();

    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config, &index_dir).unwrap();

    // Perform full reindex
    let count = engine.full_reindex().unwrap();
    assert_eq!(count, 4, "Should index 1 markdown file + 3 polyglot code files");

    // Verify BM25 search across modalities
    let adr_hits = engine.bm25.search("Reciprocal Rank Fusion", 5).unwrap();
    assert!(!adr_hits.is_empty());
    assert_eq!(adr_hits[0].path, "docs/adr/0001-hybrid-search.md");

    let rust_hits = engine.bm25.search("search_hybrid modalities", 5).unwrap();
    assert!(!rust_hits.is_empty());
    assert_eq!(rust_hits[0].path, "src/search.rs");

    let ts_hits = engine.bm25.search("UserProfile getUser", 5).unwrap();
    assert!(!ts_hits.is_empty());
    assert_eq!(ts_hits[0].path, "src/user.ts");

    // Verify SQLite code_symbols catalog
    let rust_symbols = engine.store().get_code_symbols_for_file("src/search.rs").unwrap();
    assert!(rust_symbols
        .iter()
        .any(|s| s.name == "Engine" && s.symbol_type == CodeSymbolType::Struct));
    assert!(rust_symbols
        .iter()
        .any(|s| s.name == "search_hybrid" && s.symbol_type == CodeSymbolType::Function));
    assert!(rust_symbols
        .iter()
        .any(|s| s.name == "execute_rrf" && s.symbol_type == CodeSymbolType::Function));

    let ts_symbols = engine.store().get_code_symbols_for_file("src/user.ts").unwrap();
    assert!(ts_symbols
        .iter()
        .any(|s| s.name == "UserService" && s.symbol_type == CodeSymbolType::Class));
    assert!(ts_symbols
        .iter()
        .any(|s| s.name == "getUser" && s.symbol_type == CodeSymbolType::Method));

    // Verify graph edges (defines and calls)
    let edges = engine.graph().get_all_edges();
    assert!(edges
        .iter()
        .any(|e| e.edge_type == "defines" && e.source == "src/search.rs" && e.target == "Engine"));
    assert!(edges.iter().any(|e| e.edge_type == "calls"
        && e.source == "Engine > search_hybrid"
        && e.target == "execute_rrf"));
}

#[test]
fn test_indexing_excludes_test_and_node_modules() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("repo");
    fs::create_dir_all(corpus_dir.join("src")).unwrap();
    fs::create_dir_all(corpus_dir.join("tests")).unwrap();
    fs::create_dir_all(corpus_dir.join("node_modules").join("pkg")).unwrap();
    fs::create_dir_all(corpus_dir.join("target").join("debug")).unwrap();

    // Valid source file
    fs::write(corpus_dir.join("src").join("main.rs"), "fn main() { println!(\"hello\"); }")
        .unwrap();
    // Excluded test file in tests/
    fs::write(corpus_dir.join("tests").join("integration_test.rs"), "fn test_it() {}").unwrap();
    // Excluded test file in src/
    fs::write(corpus_dir.join("src").join("app.test.rs"), "fn app_test() {}").unwrap();
    // Excluded dependency
    fs::write(corpus_dir.join("node_modules").join("pkg").join("index.js"), "module.exports = {};")
        .unwrap();
    // Excluded build artifact
    fs::write(corpus_dir.join("target").join("debug").join("out.rs"), "fn out() {}").unwrap();
    // .gitignore rule migrated into config
    fs::write(corpus_dir.join(".gitignore"), "secrets.rs\n").unwrap();
    fs::write(corpus_dir.join("secrets.rs"), "fn secret() {}").unwrap();

    let mut config = test_config(&corpus_dir);
    config.exclude.import_gitignore(&corpus_dir.join(".gitignore"));
    config.index_mode = ctxvault_common::config::IndexMode::Fast;

    let index_dir = tmp.path().join("index");
    let mut engine = Engine::open(config, &index_dir).unwrap();
    let sync_res = engine.delta_scan_paginated(500).unwrap();

    // Only src/main.rs should be indexed!
    assert_eq!(sync_res.new_files, vec!["src/main.rs"]);
    assert!(engine.store().get_file("src/main.rs").unwrap().is_some());
    assert!(engine.store().get_file("tests/integration_test.rs").unwrap().is_none());
    assert!(engine.store().get_file("src/app.test.rs").unwrap().is_none());
    assert!(engine.store().get_file("node_modules/pkg/index.js").unwrap().is_none());
    assert!(engine.store().get_file("secrets.rs").unwrap().is_none());
}
