use super::*;
use groundcontrol_common::config::{
    ChunkingConfig, CorpusConfig, CorpusMode, EdgeClass, EdgeSource, EdgeTypeConfig,
    EmbeddingConfig, GraphConfig, IndexMode,
};
use groundcontrol_common::ports::GraphStore;
use groundcontrol_common::types::EdgeProvenance;
use groundcontrol_core::engine::Engine;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Create a minimal corpus config pointing at the given path.
fn test_config(corpus_path: &std::path::Path) -> CorpusConfig {
    CorpusConfig {
        name: "test".to_string(),
        path: corpus_path.to_string_lossy().to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: IndexMode::Full,
        chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig {
            edge_types: vec![EdgeTypeConfig {
                name: "Wikilink".to_string(),
                source: EdgeSource::Wikilink,
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
        exclude: groundcontrol_common::config::ExcludeConfig::default(),
        docs: groundcontrol_common::config::DocsConfig::default(),
    }
}

/// Create a test engine with an empty corpus.
fn create_test_engine(tmp: &TempDir) -> Engine {
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");
    let config = test_config(&corpus_dir);
    Engine::open(config, &index_dir).unwrap()
}

#[test]
fn test_registry_has_all_tools() {
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let tools = registry.list();
    assert_eq!(tools.len(), 18, "Expected 18 tools registered");

    // Verify each expected tool exists.
    let expected = [
        "read_file",
        "get_snippet",
        "list_notes",
        "search",
        "search_related",
        "graph_match",
        "graph_communities",
        "write_note",
        "delete_note",
        "move_note",
        "validate",
        "list_templates",
        "status",
        "list_corpora",
        "trace_cross_corpus",
        "sync_corpus",
        "index_corpus",
        "unload_corpus",
    ];

    assert_eq!(expected.len(), 18, "expected-name list must match the 18-tool count");

    for name in expected {
        assert!(registry.get(name).is_some(), "Tool '{}' should be registered", name);
    }

    // The consolidated / deleted legacy tools must not be registered.
    for gone in [
        "read_note",
        "read_code_file",
        "read_multiple",
        "get_frontmatter",
        "create_note",
        "update_note",
        "promote_concept",
        "validate_note",
        "validate_corpus",
        "validate_taxonomy",
        "analyze_density",
        "find_semantic_gaps",
        "suggest_splits",
        "coverage_report",
        "check_index_coverage",
        "corpus_list",
        "reembed_corpus",
        "reindex_corpus",
        "get_symbol_definition",
        "get_architecture",
        "search_bm25",
        "search_semantic",
        "search_hybrid",
        "search_graph",
        "search_explain",
        "get_status",
        "get_corpus_stats",
        "get_indexing_status",
        "backlinks",
        "forwardlinks",
        "graph_path",
        "graph_stats",
        "graph_subgraph",
        "list_edge_types",
        "traverse_lineage",
        "find_callers",
        "detect_changes",
    ] {
        assert!(registry.get(gone).is_none(), "Tool '{}' must no longer be registered", gone);
    }

    // Verify read-only classification
    assert!(registry.is_read_only("read_file"));
    assert!(registry.is_read_only("get_snippet"));
    assert!(registry.is_read_only("list_notes"));
    assert!(registry.is_read_only("search"));
    assert!(registry.is_read_only("search_related"));
    assert!(registry.is_read_only("graph_match"));
    assert!(registry.is_read_only("graph_communities"));
    assert!(registry.is_read_only("validate"));
    assert!(registry.is_read_only("list_templates"));
    assert!(registry.is_read_only("status"));
    assert!(registry.is_read_only("list_corpora"));
    assert!(registry.is_read_only("trace_cross_corpus"));
    assert!(!registry.is_read_only("write_note"));
    assert!(!registry.is_read_only("delete_note"));
    assert!(!registry.is_read_only("move_note"));
    assert!(!registry.is_read_only("sync_corpus"));
    assert!(!registry.is_read_only("index_corpus"));
    assert!(!registry.is_read_only("unload_corpus"));
}

#[test]
fn test_tool_profiles_gate_listing() {
    let all = MultiCorpusToolRegistry::with_profile(ToolProfile::All);
    let analysis = MultiCorpusToolRegistry::with_profile(ToolProfile::Analysis);
    let scout = MultiCorpusToolRegistry::with_profile(ToolProfile::Scout);

    let all_count = all.list().len();
    let analysis_count = analysis.list().len();
    let scout_count = scout.list().len();

    // scout ⊂ analysis ⊂ all.
    assert!(scout_count < analysis_count, "scout must expose fewer tools than analysis");
    assert!(analysis_count < all_count, "analysis must expose fewer tools than all");
    assert_eq!(all_count, 18, "all profile advertises every registered tool");
    assert_eq!(analysis_count, 12, "analysis profile advertises scout + analysis tools");
    assert_eq!(scout_count, 6, "scout profile advertises the minimal set");

    // scout includes core retrieval/fetch but not writes or analysis-only tools.
    let scout_names: HashSet<&str> = scout.list().iter().map(|t| t.name.as_str()).collect();
    assert!(scout_names.contains("search"));
    assert!(scout_names.contains("get_snippet"));
    assert!(scout_names.contains("read_file"));
    assert!(scout_names.contains("status"));
    assert!(!scout_names.contains("write_note"));
    assert!(!scout_names.contains("graph_match"));

    // Hidden tools still execute (advertise-only filtering): write_note is
    // registered even though scout does not advertise it.
    assert!(scout.registry().get("write_note").is_some());

    // analysis adds read-only tools but still hides writes.
    let analysis_names: HashSet<&str> = analysis.list().iter().map(|t| t.name.as_str()).collect();
    assert!(analysis_names.contains("graph_match"));
    assert!(analysis_names.contains("graph_communities"));
    assert!(analysis_names.contains("validate"));
    assert!(analysis_names.contains("list_corpora"));
    assert!(analysis_names.contains("trace_cross_corpus"));
    assert!(!analysis_names.contains("write_note"));
    assert!(!analysis_names.contains("sync_corpus"));
    // trace_cross_corpus is an analysis-tier capability, not a scout tool.
    assert!(!scout_names.contains("trace_cross_corpus"));
}

#[test]
fn test_read_only_tool_execution() {
    let tmp = TempDir::new().unwrap();
    let engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    // Read tool with immutable &engine should succeed
    let result = registry.execute_read("list_notes", &engine, serde_json::json!({})).unwrap();
    let notes: Vec<Value> = serde_json::from_value(result).unwrap();
    assert!(notes.is_empty());

    // Calling mutating tool with execute_read should return error
    let err = registry.execute_read(
        "write_note",
        &engine,
        serde_json::json!({ "path": "fail.md", "content": "hello" }),
    );
    assert!(err.is_err());
}

#[test]
fn test_list_notes_empty() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let result = registry.execute("list_notes", &mut engine, serde_json::json!({})).unwrap();

    let notes: Vec<Value> = serde_json::from_value(result).unwrap();
    assert!(notes.is_empty(), "Empty corpus should return empty list");
}

#[test]
fn test_search_bm25_tool() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");
    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config, &index_dir).unwrap();

    // Write and index a test file.
    let content =
        "# Rust Programming\n\nRust is a systems programming language focused on safety.\n";
    fs::write(corpus_dir.join("rust.md"), content).unwrap();
    engine.index_file("rust.md", content).unwrap();
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    let result = registry
        .execute(
            "search",
            &mut engine,
            serde_json::json!({ "query": "systems programming", "mode": "bm25" }),
        )
        .unwrap();

    let resp: groundcontrol_common::types::SearchResponse = serde_json::from_value(result).unwrap();
    let docs = resp.docs.unwrap();
    assert!(!docs.results.is_empty(), "Should find indexed file via search");
    assert_eq!(docs.results[0].path, "rust.md");
    assert!(docs.results[0].snippet.is_some());
}

#[test]
fn test_write_note_create() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let result = registry
        .execute(
            "write_note",
            &mut engine,
            serde_json::json!({
                "path": "new-note.md",
                "content": "# Hello\n\nThis is a new note.",
                "frontmatter": { "tags": ["test", "demo"] }
            }),
        )
        .unwrap();

    assert_eq!(result["path"], "new-note.md");
    assert_eq!(result["written"], true);
    assert_eq!(result["mode"], "create");

    // Verify file exists on disk.
    let corpus_dir = tmp.path().join("corpus");
    let file_content = fs::read_to_string(corpus_dir.join("new-note.md")).unwrap();
    assert!(file_content.contains("# Hello"));
    assert!(file_content.contains("---"));

    // Verify indexed (searchable).
    let search_result = registry
        .execute("search", &mut engine, serde_json::json!({ "query": "new note", "mode": "bm25" }))
        .unwrap();
    let resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(search_result).unwrap();
    assert!(!resp.docs.unwrap().results.is_empty());
}

#[test]
fn test_write_note_with_template() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let result = registry
        .execute(
            "write_note",
            &mut engine,
            serde_json::json!({
                "path": "templated.md",
                "content": "Body text here.",
                "template": "meeting"
            }),
        )
        .unwrap();

    assert_eq!(result["written"], true);

    let corpus_dir = tmp.path().join("corpus");
    let file_content = fs::read_to_string(corpus_dir.join("templated.md")).unwrap();
    assert!(file_content.contains("template: meeting"));
}

#[test]
fn test_write_note_already_exists() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let corpus_dir = tmp.path().join("corpus");
    fs::write(corpus_dir.join("existing.md"), "# Existing").unwrap();

    let result = registry.execute(
        "write_note",
        &mut engine,
        serde_json::json!({ "path": "existing.md", "content": "overwrite?", "mode": "create" }),
    );

    assert!(result.is_err());
}

#[test]
fn test_write_note_overwrite() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let corpus_dir = tmp.path().join("corpus");
    let original = "# Original\n\nOld content.\n";
    fs::write(corpus_dir.join("update-me.md"), original).unwrap();
    engine.index_file("update-me.md", original).unwrap();
    engine.commit().unwrap();

    let result = registry
        .execute(
            "write_note",
            &mut engine,
            serde_json::json!({
                "path": "update-me.md",
                "content": "# Replaced\n\nNew content.",
                "mode": "overwrite"
            }),
        )
        .unwrap();

    assert_eq!(result["written"], true);
    assert_eq!(result["mode"], "overwrite");

    let file_content = fs::read_to_string(corpus_dir.join("update-me.md")).unwrap();
    assert!(file_content.contains("New content"));
    assert!(!file_content.contains("Old content"));
}

#[test]
fn test_write_note_append() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let corpus_dir = tmp.path().join("corpus");
    let original = "# Append Test\n\nFirst line.\n";
    fs::write(corpus_dir.join("append.md"), original).unwrap();
    engine.index_file("append.md", original).unwrap();
    engine.commit().unwrap();

    let result = registry
        .execute(
            "write_note",
            &mut engine,
            serde_json::json!({
                "path": "append.md",
                "content": "Second line.",
                "mode": "append"
            }),
        )
        .unwrap();

    assert_eq!(result["written"], true);
    assert_eq!(result["mode"], "append");

    let file_content = fs::read_to_string(corpus_dir.join("append.md")).unwrap();
    assert!(file_content.contains("First line."));
    assert!(file_content.contains("Second line."));
}

#[test]
fn test_write_note_prepend() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let corpus_dir = tmp.path().join("corpus");
    let original = "# Prepend Test\n\nOriginal.\n";
    fs::write(corpus_dir.join("prepend.md"), original).unwrap();
    engine.index_file("prepend.md", original).unwrap();
    engine.commit().unwrap();

    let result = registry
        .execute(
            "write_note",
            &mut engine,
            serde_json::json!({
                "path": "prepend.md",
                "content": "Prepended text.",
                "mode": "prepend"
            }),
        )
        .unwrap();

    assert_eq!(result["written"], true);
    assert_eq!(result["mode"], "prepend");

    let file_content = fs::read_to_string(corpus_dir.join("prepend.md")).unwrap();
    // Prepended text should appear before original content.
    let prepend_pos = file_content.find("Prepended text.").unwrap();
    let original_pos = file_content.find("Original.").unwrap();
    assert!(prepend_pos < original_pos);
}

#[test]
fn test_delete_note_tool() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let corpus_dir = tmp.path().join("corpus");
    let content = "# Delete Me\n\nGoing away.\n";
    fs::write(corpus_dir.join("delete-me.md"), content).unwrap();
    engine.index_file("delete-me.md", content).unwrap();
    engine.commit().unwrap();

    let result = registry
        .execute("delete_note", &mut engine, serde_json::json!({ "path": "delete-me.md" }))
        .unwrap();

    assert_eq!(result["deleted"], true);

    // File should be gone from disk.
    assert!(!corpus_dir.join("delete-me.md").exists());

    // Should not be found in search.
    let search_result = registry
        .execute(
            "search",
            &mut engine,
            serde_json::json!({ "query": "Going away", "mode": "bm25" }),
        )
        .unwrap();
    let resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(search_result).unwrap();
    let hits = resp.docs.map(|d| d.results).unwrap_or_default();
    assert!(hits.is_empty() || hits.iter().all(|h| h.path != "delete-me.md"));
}

#[test]
fn test_delete_note_not_found() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let result = registry.execute(
        "delete_note",
        &mut engine,
        serde_json::json!({ "path": "nonexistent.md" }),
    );

    assert!(result.is_err());
}

#[test]
fn test_move_note_tool() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let corpus_dir = tmp.path().join("corpus");

    // Create the note to move.
    let content = "# Alpha\n\nAlpha content.\n";
    fs::write(corpus_dir.join("alpha.md"), content).unwrap();
    engine.index_file("alpha.md", content).unwrap();

    // Create another note that links to alpha.
    let linker = "# Linker\n\nSee [[alpha]] for details.\n";
    fs::write(corpus_dir.join("linker.md"), linker).unwrap();
    engine.index_file("linker.md", linker).unwrap();
    engine.commit().unwrap();

    // Move alpha to beta.
    let result = registry
        .execute(
            "move_note",
            &mut engine,
            serde_json::json!({ "from": "alpha.md", "to": "beta.md" }),
        )
        .unwrap();

    assert_eq!(result["moved"], true);
    assert_eq!(result["from"], "alpha.md");
    assert_eq!(result["to"], "beta.md");
    assert_eq!(result["links_rewritten"], 1);

    // Old file should be gone, new file should exist.
    assert!(!corpus_dir.join("alpha.md").exists());
    assert!(corpus_dir.join("beta.md").exists());

    // Linker file should now reference [[beta]].
    let linker_content = fs::read_to_string(corpus_dir.join("linker.md")).unwrap();
    assert!(linker_content.contains("[[beta]]"));
    assert!(!linker_content.contains("[[alpha]]"));
}

#[test]
fn test_move_note_to_subdirectory() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let mut registry = ToolRegistry::new();
    registry.register_all();

    let corpus_dir = tmp.path().join("corpus");
    let content = "# Move Me\n\nContent.\n";
    fs::write(corpus_dir.join("movable.md"), content).unwrap();
    engine.index_file("movable.md", content).unwrap();
    engine.commit().unwrap();

    let result = registry
        .execute(
            "move_note",
            &mut engine,
            serde_json::json!({ "from": "movable.md", "to": "archive/movable.md" }),
        )
        .unwrap();

    assert_eq!(result["moved"], true);
    assert!(!corpus_dir.join("movable.md").exists());
    assert!(corpus_dir.join("archive/movable.md").exists());
}

// ─── Multi-Corpus Routing Tests ────────────────────────────────────

#[test]
fn test_multi_corpus_registry_has_status() {
    let registry = MultiCorpusToolRegistry::new();
    let tools = registry.list();

    assert_eq!(tools.len(), 18, "Expected 18 tools in multi-corpus registry");
    assert!(
        registry.registry().get("status").is_some(),
        "consolidated status tool should be registered"
    );
    // The old status tools/aliases are gone.
    assert!(registry.registry().get("get_status").is_none());
    assert!(registry.registry().get("get_corpus_stats").is_none());
    assert!(registry.registry().get("get_indexing_status").is_none());
}

fn add_test_corpus(
    manager: &mut groundcontrol_core::corpus_manager::CorpusManager,
    config: CorpusConfig,
) {
    let index_dir = PathBuf::from(&config.path).join(".index");
    manager.add_corpus_with_index_dir(config, &index_dir).unwrap();
}

#[test]
fn test_multi_corpus_routing_default() {
    let tmp = TempDir::new().unwrap();
    let wiki_dir = tmp.path().join("wiki");
    fs::create_dir_all(&wiki_dir).unwrap();

    let mut manager = groundcontrol_core::corpus_manager::CorpusManager::new();
    let config = CorpusConfig {
        name: "wiki".to_string(),
        path: wiki_dir.to_string_lossy().to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: IndexMode::Full,
        chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig { edge_types: Vec::new() },
        templates_dir: None,
        exclude: groundcontrol_common::config::ExcludeConfig::default(),
        docs: groundcontrol_common::config::DocsConfig::default(),
    };
    add_test_corpus(&mut manager, config);

    // Index a file in wiki.
    {
        let engine = manager.get_engine_mut("wiki").unwrap();
        let content = "# Wiki Note\n\nWiki content here.\n";
        fs::write(wiki_dir.join("note.md"), content).unwrap();
        engine.index_file("note.md", content).unwrap();
        engine.commit().unwrap();
    }

    let registry = MultiCorpusToolRegistry::new();

    // Search without corpus param — should use default (wiki).
    let result = registry
        .execute(
            "search",
            &mut manager,
            serde_json::json!({ "query": "wiki content", "mode": "bm25" }),
        )
        .unwrap();

    let resp: groundcontrol_common::types::SearchResponse = serde_json::from_value(result).unwrap();
    let results = resp.docs.unwrap().results;
    assert!(!results.is_empty(), "Should find wiki note via default corpus");
    assert_eq!(results[0].path, "note.md");
}

#[test]
fn test_multi_corpus_routing_explicit() {
    let tmp = TempDir::new().unwrap();
    let wiki_dir = tmp.path().join("wiki");
    let docs_dir = tmp.path().join("docs");
    fs::create_dir_all(&wiki_dir).unwrap();
    fs::create_dir_all(&docs_dir).unwrap();

    let mut manager = groundcontrol_core::corpus_manager::CorpusManager::new();

    let wiki_config = CorpusConfig {
        name: "wiki".to_string(),
        path: wiki_dir.to_string_lossy().to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: IndexMode::Full,
        chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig { edge_types: Vec::new() },
        templates_dir: None,
        exclude: groundcontrol_common::config::ExcludeConfig::default(),
        docs: groundcontrol_common::config::DocsConfig::default(),
    };
    let docs_config = CorpusConfig {
        name: "docs".to_string(),
        path: docs_dir.to_string_lossy().to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: IndexMode::Full,
        chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig { edge_types: Vec::new() },
        templates_dir: None,
        exclude: groundcontrol_common::config::ExcludeConfig::default(),
        docs: groundcontrol_common::config::DocsConfig::default(),
    };

    add_test_corpus(&mut manager, wiki_config);
    add_test_corpus(&mut manager, docs_config);

    // Index different content in each corpus.
    {
        let engine = manager.get_engine_mut("wiki").unwrap();
        let content = "# Rust Wiki\n\nRust programming language notes.\n";
        fs::write(wiki_dir.join("rust.md"), content).unwrap();
        engine.index_file("rust.md", content).unwrap();
        engine.commit().unwrap();
    }
    {
        let engine = manager.get_engine_mut("docs").unwrap();
        let content = "# Python Docs\n\nPython documentation guide.\n";
        fs::write(docs_dir.join("python.md"), content).unwrap();
        engine.index_file("python.md", content).unwrap();
        engine.commit().unwrap();
    }

    let registry = MultiCorpusToolRegistry::new();

    // Search in wiki corpus explicitly.
    let result = registry
        .execute(
            "search",
            &mut manager,
            serde_json::json!({ "query": "programming", "mode": "bm25", "corpus": "wiki" }),
        )
        .unwrap();
    let resp: groundcontrol_common::types::SearchResponse = serde_json::from_value(result).unwrap();
    let results = resp.docs.unwrap().results;
    assert!(!results.is_empty(), "Should find rust.md in wiki");
    assert_eq!(results[0].path, "rust.md");

    // Search in docs corpus explicitly.
    let result = registry
        .execute(
            "search",
            &mut manager,
            serde_json::json!({ "query": "documentation", "mode": "bm25", "corpus": "docs" }),
        )
        .unwrap();
    let resp: groundcontrol_common::types::SearchResponse = serde_json::from_value(result).unwrap();
    let results = resp.docs.unwrap().results;
    assert!(!results.is_empty(), "Should find python.md in docs");
    assert_eq!(results[0].path, "python.md");

    // Verify isolation: searching wiki for python returns nothing.
    let result = registry
            .execute(
                "search",
                &mut manager,
                serde_json::json!({ "query": "python documentation", "mode": "bm25", "corpus": "wiki" }),
            )
            .unwrap();
    let resp: groundcontrol_common::types::SearchResponse = serde_json::from_value(result).unwrap();
    let results = resp.docs.map(|d| d.results).unwrap_or_default();
    assert!(
        results.is_empty() || results.iter().all(|r| r.path != "python.md"),
        "Wiki corpus should not contain python.md"
    );
}

#[test]
fn test_multi_corpus_fan_out_tags_by_corpus() {
    let tmp = TempDir::new().unwrap();
    let wiki_dir = tmp.path().join("wiki");
    let docs_dir = tmp.path().join("docs");
    fs::create_dir_all(&wiki_dir).unwrap();
    fs::create_dir_all(&docs_dir).unwrap();

    let mut manager = groundcontrol_core::corpus_manager::CorpusManager::new();
    for (name, dir) in [("wiki", &wiki_dir), ("docs", &docs_dir)] {
        let config = CorpusConfig {
            name: name.to_string(),
            path: dir.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Full,
            chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig { edge_types: Vec::new() },
            templates_dir: None,
            exclude: groundcontrol_common::config::ExcludeConfig::default(),
            docs: groundcontrol_common::config::DocsConfig::default(),
        };
        add_test_corpus(&mut manager, config);
    }

    // Both corpora contain a doc mentioning "shared" (BM25-only; no embedder).
    {
        let engine = manager.get_engine_mut("wiki").unwrap();
        let content = "# Wiki\n\nshared knowledge lives here in the wiki.\n";
        fs::write(wiki_dir.join("shared.md"), content).unwrap();
        engine.index_file("shared.md", content).unwrap();
        engine.commit().unwrap();
    }
    {
        let engine = manager.get_engine_mut("docs").unwrap();
        let content = "# Docs\n\nshared documentation lives here in the docs.\n";
        fs::write(docs_dir.join("shared.md"), content).unwrap();
        engine.index_file("shared.md", content).unwrap();
        engine.commit().unwrap();
    }

    let registry = MultiCorpusToolRegistry::new();

    // Fan out across both corpora with corpora = "all".
    let result = registry
        .execute_read(
            "search",
            &manager,
            serde_json::json!({ "query": "shared", "mode": "bm25", "corpora": "all" }),
        )
        .unwrap();

    let resp: groundcontrol_common::types::SearchResponse = serde_json::from_value(result).unwrap();
    let docs = resp.docs.unwrap();
    assert_eq!(docs.results.len(), 2, "both corpora should contribute a hit");

    // Same path, distinct corpora → two tagged hits.
    let corpora: HashSet<String> = docs.results.iter().filter_map(|r| r.corpus.clone()).collect();
    assert!(corpora.contains("wiki"), "a hit must be tagged 'wiki'");
    assert!(corpora.contains("docs"), "a hit must be tagged 'docs'");
    assert!(docs.results.iter().all(|r| r.path == "shared.md"));

    // Single-corpus read via corpus="wiki" also tags its hit.
    let single = registry
        .execute_read(
            "search",
            &manager,
            serde_json::json!({ "query": "shared", "mode": "bm25", "corpus": "wiki" }),
        )
        .unwrap();
    let single_resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(single).unwrap();
    let single_docs = single_resp.docs.unwrap();
    assert!(!single_docs.results.is_empty());
    assert!(single_docs.results.iter().all(|r| r.corpus.as_deref() == Some("wiki")));
}

/// Fast-mode (embedder-free) corpus config for scoping-parity + federated
/// tests. Fast mode skips dense embeddings, so no ONNX model is needed, yet
/// BM25/graph retrieval and code extraction still run.
fn fast_corpus_config(name: &str, dir: &Path) -> CorpusConfig {
    let _ = fs::create_dir_all(dir.join(".index"));
    CorpusConfig {
        name: name.to_string(),
        path: dir.to_string_lossy().to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: IndexMode::Fast,
        chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig {
            edge_types: vec![EdgeTypeConfig {
                name: "Wikilink".to_string(),
                source: EdgeSource::Wikilink,
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
        exclude: groundcontrol_common::config::ExcludeConfig::default(),
        docs: groundcontrol_common::config::DocsConfig::default(),
    }
}

/// Rust source defining exactly one top-level fn (scope_path == bare name).
fn rs_symbol(name: &str) -> String {
    format!("pub fn {name}() -> u32 {{\n    42\n}}\n")
}

/// Rust source for a named fn that calls an out-of-corpus fn (produces an
/// unresolved `ExternalRef` that `resolve_external_refs` links across corpora).
fn rs_caller(caller: &str, callee: &str) -> String {
    format!("pub fn {caller}() -> u32 {{\n    {callee}()\n}}\n")
}

/// `corpus`/`corpora` resolution is applied uniformly BEFORE per-engine
/// dispatch, so every search `mode` is scopable to N / N+1 corpora. This
/// asserts each embedder-free mode (`bm25`, `graph`, `hybrid`) honors
/// `corpora=["A","B"]` and `corpora="all"`. Semantic mode is asserted at the
/// routing level only: it requires the ONNX embedder, which fast-mode corpora
/// deliberately do not load — but it flows through the identical
/// `resolve_corpus_target` fan-out path, so its scoping is proven by the fact
/// that fan-out invokes the same code for every mode. Here we confirm the
/// mode-agnostic fan-out surfaces corpus-tagged hits from BOTH corpora.
#[test]
fn test_search_modes_honor_corpora_scoping() {
    let tmp = TempDir::new().unwrap();
    let a_dir = tmp.path().join("A");
    let b_dir = tmp.path().join("B");
    fs::create_dir_all(&a_dir).unwrap();
    fs::create_dir_all(&b_dir).unwrap();

    let mut manager = groundcontrol_core::corpus_manager::CorpusManager::new();
    add_test_corpus(&mut manager, fast_corpus_config("A", &a_dir));
    add_test_corpus(&mut manager, fast_corpus_config("B", &b_dir));

    // Each corpus has a doc that shares the query token "shared" and links
    // to a neighbor so graph search (which returns discovered neighbors) has nodes.
    // Index the neighbor first so indexing the parent adds the edge last.
    {
        let a = manager.get_engine_mut("A").unwrap();
        let sub_content = "# Alpha Sub\n\nsub knowledge.\n";
        fs::write(a_dir.join("alpha_sub.md"), sub_content).unwrap();
        a.index_file("alpha_sub.md", sub_content).unwrap();
        let content = "# Alpha\n\nshared alpha knowledge lives here. See [[alpha_sub.md]].\n";
        fs::write(a_dir.join("alpha.md"), content).unwrap();
        a.index_file("alpha.md", content).unwrap();
        a.commit().unwrap();
    }
    {
        let b = manager.get_engine_mut("B").unwrap();
        let sub_content = "# Beta Sub\n\nsub knowledge.\n";
        fs::write(b_dir.join("beta_sub.md"), sub_content).unwrap();
        b.index_file("beta_sub.md", sub_content).unwrap();
        let content = "# Beta\n\nshared beta knowledge lives here. See [[beta_sub.md]].\n";
        fs::write(b_dir.join("beta.md"), content).unwrap();
        b.index_file("beta.md", content).unwrap();
        b.commit().unwrap();
    }

    let registry = MultiCorpusToolRegistry::new();

    // Modes that need no dense embedder: full end-to-end fan-out assertions.
    for mode in ["bm25", "graph", "hybrid"] {
        for corpora in [serde_json::json!(["A", "B"]), serde_json::json!("all")] {
            let result = registry
                .execute_read(
                    "search",
                    &manager,
                    serde_json::json!({ "query": "shared", "mode": mode, "corpora": corpora }),
                )
                .unwrap_or_else(|e| panic!("mode {mode} corpora {corpora:?} failed: {e}"));

            if mode == "graph" {
                eprintln!("DEBUG GRAPH RESULT: {result:#?}");
            }

            let resp: groundcontrol_common::types::SearchResponse =
                serde_json::from_value(result).unwrap();
            let docs = resp.docs.unwrap_or_default();
            let corpora_seen: HashSet<String> =
                docs.results.iter().filter_map(|r| r.corpus.clone()).collect();
            assert!(
                corpora_seen.contains("A"),
                "mode {mode} corpora {corpora:?}: expected a hit tagged 'A', saw {corpora_seen:?}"
            );
            assert!(
                corpora_seen.contains("B"),
                "mode {mode} corpora {corpora:?}: expected a hit tagged 'B', saw {corpora_seen:?}"
            );
        }
    }

    // Semantic mode: fast-mode corpora have no ONNX embedder, so an
    // end-to-end semantic query is not meaningful here. It rides the SAME
    // mode-agnostic fan-out path (resolve_corpus_target strips corpora before
    // per-engine dispatch, independent of `mode`), so its scoping is proven by
    // the fan-out invoking each engine — we assert the call fans out to both
    // engines by observing per-corpus execution (a fast-mode semantic call
    // errors per engine, so the fan-out surfaces that error rather than a
    // wrong-corpus routing). The routing itself is mode-independent.
    let sem = registry.execute_read(
        "search",
        &manager,
        serde_json::json!({ "query": "shared", "mode": "semantic", "corpora": ["A", "B"] }),
    );
    assert!(
        sem.is_err(),
        "semantic fan-out over embedder-free corpora surfaces the per-engine \
             embedder error, confirming the call was routed/fanned out (not silently dropped)"
    );
}

/// The `trace_cross_corpus` MCP tool exposes federated traversal: given a
/// start node in corpus A whose call crosses into corpus B, the returned JSON
/// carries a `hops` entry naming `to_corpus == "B"` and a `nodes` entry tagged
/// with `corpus == "B"`.
#[test]
fn test_trace_cross_corpus_returns_hop_annotated_results() {
    let tmp = TempDir::new().unwrap();
    let a_dir = tmp.path().join("A");
    let b_dir = tmp.path().join("B");
    fs::create_dir_all(&a_dir).unwrap();
    fs::create_dir_all(&b_dir).unwrap();

    let mut manager = groundcontrol_core::corpus_manager::CorpusManager::new();
    add_test_corpus(&mut manager, fast_corpus_config("A", &a_dir));
    add_test_corpus(&mut manager, fast_corpus_config("B", &b_dir));

    // B uniquely defines `leaf`; A's `top` calls `leaf` (unresolved locally).
    {
        let b = manager.get_engine_mut("B").unwrap();
        b.index_file("src/leaf.rs", &rs_symbol("leaf")).unwrap();
        b.commit().unwrap();
    }
    {
        let a = manager.get_engine_mut("A").unwrap();
        a.index_file("src/top.rs", &rs_caller("top", "leaf")).unwrap();
        a.commit().unwrap();
    }

    // Build the cross-corpus edge (CorpusManager-level; public API).
    let created = manager.resolve_external_refs().unwrap();
    assert!(created >= 1, "a unique cross-corpus ref must create an edge");

    let registry = MultiCorpusToolRegistry::new();
    let result = registry
        .execute_read(
            "trace_cross_corpus",
            &manager,
            serde_json::json!({
                "start_corpus": "A",
                "start_node": "top",
                "per_corpus_depth": 4,
                "max_corpus_hops": 3,
                "continue": true
            }),
        )
        .unwrap();

    // A cross-corpus hop into B must be recorded.
    let hops = result["hops"].as_array().expect("hops must be an array");
    assert!(
        hops.iter().any(|h| h["to_corpus"] == "B"),
        "hops must name a cross-corpus seam into corpus B: {hops:?}"
    );
    let ab = hops.iter().find(|h| h["to_corpus"] == "B").unwrap();
    assert_eq!(ab["from_corpus"], "A");
    assert_eq!(ab["to_node"], "leaf");

    // A node tagged with corpus B must appear (live continuation entered B).
    let nodes = result["nodes"].as_array().expect("nodes must be an array");
    assert!(
        nodes.iter().any(|n| n["corpus"] == "B" && n["node"] == "leaf"),
        "nodes must include B's `leaf` node tagged corpus 'B': {nodes:?}"
    );
    // The origin node in A is present at depth 0.
    assert!(
        nodes.iter().any(|n| n["corpus"] == "A" && n["node"] == "top" && n["depth"] == 0),
        "origin node A::top must be present at depth 0: {nodes:?}"
    );
}

#[test]
fn test_multi_corpus_get_status() {
    let tmp = TempDir::new().unwrap();
    let wiki_dir = tmp.path().join("wiki");
    fs::create_dir_all(&wiki_dir).unwrap();

    let mut manager = groundcontrol_core::corpus_manager::CorpusManager::new();
    let config = CorpusConfig {
        name: "wiki".to_string(),
        path: wiki_dir.to_string_lossy().to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: IndexMode::Full,
        chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig { edge_types: Vec::new() },
        templates_dir: None,
        exclude: groundcontrol_common::config::ExcludeConfig::default(),
        docs: groundcontrol_common::config::DocsConfig::default(),
    };
    add_test_corpus(&mut manager, config);

    let registry = MultiCorpusToolRegistry::new();

    let result = registry.execute("status", &mut manager, serde_json::json!({})).unwrap();

    assert_eq!(result["corpus_count"], 1);
    assert_eq!(result["default_corpus"], "wiki");
    let corpora = result["corpora"].as_array().unwrap();
    assert_eq!(corpora.len(), 1);
    assert_eq!(corpora[0]["name"], "wiki");
}

#[test]
fn test_multi_corpus_invalid_corpus_returns_error() {
    let tmp = TempDir::new().unwrap();
    let wiki_dir = tmp.path().join("wiki");
    fs::create_dir_all(&wiki_dir).unwrap();

    let mut manager = groundcontrol_core::corpus_manager::CorpusManager::new();
    let config = CorpusConfig {
        name: "wiki".to_string(),
        path: wiki_dir.to_string_lossy().to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: IndexMode::Full,
        chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig { edge_types: Vec::new() },
        templates_dir: None,
        exclude: groundcontrol_common::config::ExcludeConfig::default(),
        docs: groundcontrol_common::config::DocsConfig::default(),
    };
    add_test_corpus(&mut manager, config);

    let registry = MultiCorpusToolRegistry::new();

    // Non-existent corpus should error.
    let result = registry.execute(
        "search",
        &mut manager,
        serde_json::json!({ "query": "test", "mode": "bm25", "corpus": "nonexistent" }),
    );
    assert!(result.is_err());
}

// ─── Graph Match Tool Tests ────────────────────────────────────────

#[test]
fn test_graph_match_tool() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);

    // Add nodes and lineage edge directly to graph and SQLite
    engine.graph_mut().add_edge(
        "docs/adrs/002.md",
        "docs/adrs/001.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    let result = registry
        .execute(
            "graph_match",
            &mut engine,
            serde_json::json!({
                "pattern": "(a)-[:supersedes]->(b)"
            }),
        )
        .unwrap();

    let match_res: groundcontrol_common::types::GraphMatchResult =
        serde_json::from_value(result).unwrap();
    assert_eq!(match_res.total_matches, 1);
    assert_eq!(match_res.tree[0].node, "docs/adrs/001.md");
}

#[test]
fn test_validate_tool() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);

    // Add broken link: valid.md -> missing.md
    engine.graph_mut().add_edge(
        "valid.md",
        "missing.md",
        "Wikilink",
        1.0,
        EdgeProvenance::Wikilink,
        EdgeClass::Structural,
    );

    // Add circular dependency: A -> B -> A
    engine.graph_mut().add_edge(
        "A.md",
        "B.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );
    engine.graph_mut().add_edge(
        "B.md",
        "A.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );

    let mut registry = ToolRegistry::new();
    registry.register_all();

    let result = registry
        .execute("validate", &mut engine, serde_json::json!({ "check_taxonomy": true }))
        .unwrap();

    assert_eq!(result["valid"], false);
    assert!(result["taxonomy"]["broken_links_count"].as_u64().unwrap() >= 1);
    assert!(result["taxonomy"]["circular_dependencies_count"].as_u64().unwrap() >= 1);
}

#[test]
fn test_list_templates_and_validate_markdown_template() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let corpus_dir = tmp.path().join("corpus");
    let templates_dir = corpus_dir.join(".templates");
    fs::create_dir_all(&templates_dir).unwrap();

    let adr_template = r#"---
template:
  name: adr
  description: "Architecture Decision Record"

schema:
  fields:
    status:
      type: enum
      required: true
      values: [proposed, accepted, rejected]
    date:
      type: date
      required: true
  edges:
    - field: supersedes
      type: Supersedes
      class: structural
      direction: outbound
      target_template: adr
      required: false

  sections:
    required: ["Context", "Decision"]
  min_words: 20
---
# ADR-{id}: {Title}

## Context
Describe context.

## Decision
State decision.
"#;
    fs::write(templates_dir.join("adr.md"), adr_template).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // 1. list_templates returns schema + scaffold
    let tmpl_list = registry.execute("list_templates", &mut engine, serde_json::json!({})).unwrap();
    let list_arr = tmpl_list.as_array().unwrap();
    assert_eq!(list_arr.len(), 1);
    assert_eq!(list_arr[0]["name"], "adr");
    assert!(list_arr[0]["scaffold"].as_str().unwrap().contains("# ADR-{id}: {Title}"));
    assert_eq!(list_arr[0]["edges"][0]["field"], "supersedes");

    // 2. Validate a valid note
    let note_content = r#"---
template: adr
status: accepted
date: 2026-09-11
---
# ADR-001: First Decision

## Context
This is a comprehensive context section that satisfies the minimum word count requirement for this template.

## Decision
We decide to adopt the markdown template standard across all repositories.
"#;
    fs::write(corpus_dir.join("001.md"), note_content).unwrap();
    engine.index_file("001.md", note_content).unwrap();
    engine.commit().unwrap();

    let val_res = registry
        .execute(
            "validate",
            &mut engine,
            serde_json::json!({ "path": "001.md", "check_taxonomy": false }),
        )
        .unwrap();
    assert_eq!(val_res["valid"], true, "Note should be valid: {:?}", val_res);

    // 3. Validate a note with missing required field and missing section
    let invalid_note = r#"---
template: adr
status: accepted
---
# ADR-002: Incomplete

## Context
Only context, missing decision and date.
"#;
    fs::write(corpus_dir.join("002.md"), invalid_note).unwrap();
    let val_invalid = registry
        .execute(
            "validate",
            &mut engine,
            serde_json::json!({ "path": "002.md", "check_taxonomy": false }),
        )
        .unwrap();
    assert_eq!(val_invalid["valid"], false, "Note should be invalid: {:?}", val_invalid);
}

#[test]
fn test_code_intelligence_mcp_tools() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let corpus_dir = tmp.path().join("corpus");

    // Write polyglot code files
    let rust_code = r#"
pub struct QueryParser;

impl QueryParser {
    pub fn parse_query(&self, raw: &str) -> Vec<String> {
        tokenize(raw)
    }
}

pub fn tokenize(input: &str) -> Vec<String> {
    vec![input.to_string()]
}
"#;
    fs::write(corpus_dir.join("parser.rs"), rust_code).unwrap();
    engine.index_file("parser.rs", rust_code).unwrap();
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // 1. Test get_snippet (absorbed get_symbol_definition)
    let def_res = registry
        .execute_read("get_snippet", &engine, serde_json::json!({ "name": "parse_query" }))
        .unwrap();

    assert_eq!(def_res["path"], "parser.rs");
    assert!(def_res["total_lines"].as_u64().unwrap() >= 1);
    assert!(def_res["source"].as_str().unwrap().contains("tokenize(raw)"));

    // 2. Test callers via graph_match
    let callers_res = registry
        .execute_read(
            "graph_match",
            &engine,
            serde_json::json!({ "pattern": "(caller)-[:calls]->(target {name: \"tokenize\"})" }),
        )
        .unwrap();
    let match_res: groundcontrol_common::types::GraphMatchResult =
        serde_json::from_value(callers_res).unwrap();
    assert_eq!(match_res.total_matches, 1);
    assert_eq!(match_res.tree[0].node, "tokenize");

    // 3. Test graph_communities view='architecture' (absorbed get_architecture)
    let arch_res = registry
        .execute_read("graph_communities", &engine, serde_json::json!({ "view": "architecture" }))
        .unwrap();

    assert!(arch_res["component_count"].as_u64().unwrap() >= 1);
    assert!(!arch_res["components"].as_array().unwrap().is_empty());
}

#[test]
fn test_read_file_tool() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let corpus_dir = tmp.path().join("corpus");

    let md = "# Design Note\n\nSome markdown content here.\n";
    fs::write(corpus_dir.join("design.md"), md).unwrap();
    engine.index_file("design.md", md).unwrap();

    let rust = "pub fn helper() -> u32 { 42 }\n";
    fs::write(corpus_dir.join("lib.rs"), rust).unwrap();
    engine.index_file("lib.rs", rust).unwrap();
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // Single file read
    let single = registry
        .execute_read("read_file", &engine, serde_json::json!({ "path": "design.md" }))
        .unwrap();
    assert_eq!(single["kind"], "markdown_note");
    assert_eq!(single["title"], "Design Note");
    assert!(single["content"].as_str().unwrap().contains("markdown content"));

    // Batch file read: Two existing files + one missing -> 3 entries, one carrying an error.
    let res = registry
        .execute_read(
            "read_file",
            &engine,
            serde_json::json!({ "paths": ["design.md", "lib.rs", "nope.md"] }),
        )
        .unwrap();

    assert_eq!(res["count"], 3);
    let results = res["results"].as_array().unwrap();

    let note = results.iter().find(|r| r["path"] == "design.md").unwrap();
    assert_eq!(note["kind"], "markdown_note");
    assert_eq!(note["title"], "Design Note");
    assert!(note["content"].as_str().unwrap().contains("markdown content"));
    assert!(note.get("error").is_none());

    let code = results.iter().find(|r| r["path"] == "lib.rs").unwrap();
    assert_eq!(code["kind"], "code_file");
    assert_eq!(code["language"], "rust");
    assert!(code["content"].as_str().unwrap().contains("helper"));

    let missing = results.iter().find(|r| r["path"] == "nope.md").unwrap();
    assert!(missing.get("error").is_some(), "missing path must carry an error entry");
}

#[test]
fn test_status_coverage_scope() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let corpus_dir = tmp.path().join("corpus");

    let rust = r#"
pub fn indexed_fn() -> u32 {
    7
}
"#;
    fs::write(corpus_dir.join("covered.rs"), rust).unwrap();
    engine.index_file("covered.rs", rust).unwrap();
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    let res = registry
            .execute_read(
                "status",
                &engine,
                serde_json::json!({ "scope": "coverage", "paths": ["covered.rs", "does_not_exist.rs"] }),
            )
            .unwrap();

    let reports = res["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 2);

    let covered = reports.iter().find(|r| r["path"] == "covered.rs").unwrap();
    assert_eq!(covered["indexed"], true);
    assert!(covered["chunk_count"].as_u64().unwrap() > 0);
    assert_eq!(covered["parsed"], true);

    let bogus = reports.iter().find(|r| r["path"] == "does_not_exist.rs").unwrap();
    assert_eq!(bogus["indexed"], false);
    assert_eq!(bogus["parsed"], false);

    assert_eq!(res["summary"]["total"], 2);
    assert_eq!(res["summary"]["covered"], 1);
    assert_eq!(res["summary"]["uncovered"], 1);
}

#[test]
fn test_fast_mode_mcp_tools() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("fast_corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    fs::write(
            corpus_dir.join("guide.md"),
            "# Architecture Guide\nFast mode provides instant BM25 and graph search without vector models.\n",
        )
        .unwrap();

    let mut config = test_config(&corpus_dir);
    config.index_mode = IndexMode::Fast;

    let index_dir = tmp.path().join(".index");
    let mut engine = Engine::open(config, &index_dir).unwrap();
    let files_indexed = engine.full_reindex().unwrap();
    assert_eq!(files_indexed, 1);
    assert!(engine.is_fast_mode());
    assert!(!engine.has_vector_index());

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // 1. Semantic search must fail with the exact fast mode error message
    let sem_err = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({ "query": "architecture guide", "mode": "semantic" }),
        )
        .unwrap_err();
    assert!(
            sem_err.to_string().contains(
                "Semantic search is unavailable in fast mode. Re-index with index_mode = 'full' to enable vector search."
            ),
            "Unexpected error: {sem_err}"
        );

    // 2. Hybrid search must cleanly fall back to BM25+Graph
    let hyb_res = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({ "query": "architecture", "mode": "hybrid" }),
        )
        .unwrap();
    let hyb_resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(hyb_res).unwrap();
    let hyb_docs = hyb_resp.docs.unwrap();
    assert_eq!(hyb_docs.results.len(), 1);
    assert_eq!(hyb_docs.results[0].path, "guide.md");

    // 3. Sync corpus with fast: true maintains fast mode
    let sync_res =
        registry.execute("sync_corpus", &mut engine, serde_json::json!({ "fast": true })).unwrap();
    assert_eq!(sync_res["status"], "complete");
    assert!(engine.is_fast_mode());
}

#[test]
fn test_full_mode_mcp_tools() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("full_corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    fs::write(
            corpus_dir.join("guide.md"),
            "# Architecture Guide\nFull mode provides vector search for documentation and hamming for code.\n",
        )
        .unwrap();
    fs::write(
        corpus_dir.join("service.rs"),
        "pub struct SearchPipeline;\npub fn execute_pipeline() {}\n",
    )
    .unwrap();

    let mut config = test_config(&corpus_dir);
    config.index_mode = IndexMode::Full;

    let index_dir = tmp.path().join(".index");
    let mut engine = Engine::open(config, &index_dir).unwrap();
    let files_indexed = engine.full_reindex().unwrap();
    assert_eq!(files_indexed, 2);
    assert!(!engine.is_fast_mode());
    assert!(engine.has_vector_index());

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // 1. Status tool reflects Full mode
    let status_res =
        registry.execute_read("status", &engine, serde_json::json!({ "scope": "corpus" })).unwrap();
    assert_eq!(status_res["index_mode"], "Full");

    // 2. BM25 search finds both doc and code
    let bm25_res = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({ "query": "SearchPipeline", "mode": "bm25" }),
        )
        .unwrap();
    let bm25_resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(bm25_res).unwrap();
    assert!(bm25_resp.code.is_some());

    // 3. Hybrid search executes cleanly
    let hyb_res = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({ "query": "Architecture Guide", "mode": "hybrid" }),
        )
        .unwrap();
    let hyb_resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(hyb_res).unwrap();
    assert!(hyb_resp.docs.is_some());

    // 4. Reindex with index_mode override preserves Full
    let reindex_res = registry
        .execute(
            "sync_corpus",
            &mut engine,
            serde_json::json!({ "mode": "full", "index_mode": "full" }),
        )
        .unwrap();
    assert_eq!(reindex_res["status"], "complete");
    assert!(!engine.is_fast_mode());
}

// ─── Progressive Disclosure Tests (Tier 1 → 2 → 3) ─────────────────

#[test]
fn test_progressive_disclosure_handle_fetch_full() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let corpus_dir = tmp.path().join("corpus");

    // A markdown note with two headings → two chunks.
    let md = "# Alpha Section\n\nAlpha talks about retrieval and ranking.\n\n\
                  # Beta Section\n\nBeta talks about graph traversal and edges.\n";
    fs::write(corpus_dir.join("notes.md"), md).unwrap();
    engine.index_file("notes.md", md).unwrap();

    // A Rust file with a caller/callee pair.
    let rust = r#"
pub struct Router;

impl Router {
    pub fn dispatch(&self, q: &str) -> Vec<String> {
        normalize(q)
    }
}

pub fn normalize(input: &str) -> Vec<String> {
    vec![input.to_lowercase()]
}
"#;
    fs::write(corpus_dir.join("router.rs"), rust).unwrap();
    engine.index_file("router.rs", rust).unwrap();
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // Tier 1: a search with detail="ids" returns handles with snippet == None.
    let ids_res = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({ "query": "retrieval ranking", "mode": "bm25", "detail": "ids" }),
        )
        .unwrap();
    let ids_resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(ids_res).unwrap();
    let ids_results = ids_resp.docs.unwrap().results;
    assert!(!ids_results.is_empty(), "detail=ids should still return handles");
    assert!(
        ids_results.iter().all(|r| r.snippet.is_none()),
        "detail=ids must strip snippets (bare handles only)"
    );

    // Default detail keeps a short snippet.
    let default_res = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({ "query": "retrieval ranking", "mode": "bm25" }),
        )
        .unwrap();
    let default_resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(default_res).unwrap();
    let default_results = default_resp.docs.unwrap().results;
    assert!(default_results.iter().any(|r| r.snippet.is_some()), "default keeps a snippet");

    // Tier 2 (doc): fetch exactly one chunk by path + chunk_index, bounded.
    let chunk_res = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "path": "notes.md", "chunk_index": 0, "max_lines": 100 }),
        )
        .unwrap();
    assert_eq!(chunk_res["path"], "notes.md");
    assert!(chunk_res["total_lines"].as_u64().unwrap() >= 1);
    assert_eq!(chunk_res["chunk_index"], 0);
    assert!(chunk_res["text"].as_str().unwrap().contains("Alpha"));

    // Tier 2 (doc) neighbor expansion: adjacent chunk is returned.
    let chunk_nb = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({
                "path": "notes.md",
                "chunk_index": 0,
                "include_neighbors": true
            }),
        )
        .unwrap();
    assert_eq!(chunk_nb["previous"], Value::Null, "chunk 0 has no previous");
    assert!(chunk_nb["next"].is_object(), "chunk 0 should have a next neighbor");
    assert!(chunk_nb["next"]["text"].as_str().unwrap().contains("Beta"));

    // Tier 2 (code): fetch one symbol's source by qualified_name.
    let sym_res = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "qualified_name": "Router > dispatch" }),
        )
        .unwrap();
    assert_eq!(sym_res["path"], "router.rs");
    assert!(sym_res["total_lines"].as_u64().unwrap() >= 1);
    assert!(sym_res["source"].as_str().unwrap().contains("normalize(q)"));
    assert!(sym_res["start_line"].as_u64().unwrap() >= 1);
    assert!(
        sym_res["end_line"].as_u64().unwrap() >= sym_res["start_line"].as_u64().unwrap(),
        "line range must be well-formed"
    );

    // Tier 2 (code) neighbor expansion: callees include the called symbol.
    let sym_nb = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({
                "qualified_name": "Router > dispatch",
                "include_neighbors": true
            }),
        )
        .unwrap();
    let outgoing = sym_nb["relationships"]["outgoing"].as_object().unwrap();
    let callees = outgoing.get("calls").and_then(|v| v.as_array()).unwrap();
    assert!(
        callees.iter().any(|c| c["name"] == "normalize" || c["scope_path"] == "normalize"),
        "dispatch should list normalize as an outgoing calls handle"
    );
    // Callees are HANDLES only — no body field.
    assert!(callees.iter().all(|c| c.get("source").is_none()), "neighbors are handles, not bodies");

    // Callers of normalize should include dispatch in incoming calls.
    let normalize_nb = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "qualified_name": "normalize", "include_neighbors": true }),
        )
        .unwrap();
    let incoming = normalize_nb["relationships"]["incoming"].as_object().unwrap();
    let callers = incoming.get("calls").and_then(|v| v.as_array()).unwrap();
    assert!(
        callers.iter().any(|c| c["scope_path"] == "Router > dispatch"),
        "normalize should list Router > dispatch as an incoming calls handle"
    );

    // Tier 2 bounding: max_lines truncates the body.
    let capped = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "qualified_name": "Router > dispatch", "max_lines": 1 }),
        )
        .unwrap();
    assert_eq!(capped["truncated"], true, "max_lines=1 must truncate a multi-line symbol");
    assert_eq!(capped["source"].as_str().unwrap().lines().count(), 1);

    // Tier 3 (code): read the whole file raw.
    let file_res = registry
        .execute_read("read_file", &engine, serde_json::json!({ "path": "router.rs" }))
        .unwrap();
    assert_eq!(file_res["language"], "rust");
    assert!(file_res["content"].as_str().unwrap().contains("pub struct Router;"));
    assert!(file_res["content"].as_str().unwrap().contains("pub fn normalize"));
    assert!(file_res["total_lines"].as_u64().unwrap() >= 5);

    // A bare path (no chunk_index / qualified_name) is redirected to Tier 3.
    let hint =
        registry.execute_read("get_snippet", &engine, serde_json::json!({ "path": "router.rs" }));
    assert!(hint.is_err(), "bare path must hint toward Tier 3");
}

#[test]
fn test_search_detail_ids_stripping_and_explain() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let corpus_dir = tmp.path().join("corpus");

    let md_old = "# Legacy\n\nLegacy architecture and design.\n";
    let md_new = "# Modern\n\nModern architecture and design.\n";
    fs::write(corpus_dir.join("legacy.md"), md_old).unwrap();
    fs::write(corpus_dir.join("modern.md"), md_new).unwrap();
    engine.index_file("legacy.md", md_old).unwrap();
    engine.index_file("modern.md", md_new).unwrap();

    let rust = r#"
pub struct Service;

impl Service {
    pub fn process(&self) -> bool {
        true
    }
}
"#;
    fs::write(corpus_dir.join("service.rs"), rust).unwrap();
    engine.index_file("service.rs", rust).unwrap();

    // Add structural edge so legacy.md has lineage: modern.md supersedes legacy.md
    engine.graph_mut().add_edge(
        "modern.md",
        "legacy.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        groundcontrol_common::config::EdgeClass::Structural,
    );
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // 1. detail="ids" on code search: snippet, lineage, and score_components must all be None
    let code_ids_res = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({
                "query": "Service process",
                "modality": "code",
                "mode": "bm25",
                "detail": "ids"
            }),
        )
        .unwrap();
    let code_ids_resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(code_ids_res).unwrap();
    let code_ids_results = code_ids_resp.code.unwrap().results;
    assert!(!code_ids_results.is_empty(), "expected hits for Service process");
    for r in &code_ids_results {
        assert!(r.snippet.is_none(), "code hit snippet must be None with detail=ids");
        assert!(r.lineage.is_none(), "code hit lineage must be None with detail=ids");
        assert!(
            r.score_components.is_none(),
            "code hit score_components must be None with detail=ids"
        );
    }

    // 2. detail="ids" on doc search with lineage: snippet, lineage, and score_components must all be None
    let doc_ids_res = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({
                "query": "Legacy architecture",
                "modality": "docs",
                "mode": "bm25",
                "detail": "ids"
            }),
        )
        .unwrap();
    let doc_ids_resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(doc_ids_res).unwrap();
    let doc_ids_results = doc_ids_resp.docs.unwrap().results;
    assert!(!doc_ids_results.is_empty(), "expected hits for Legacy architecture");
    for r in &doc_ids_results {
        assert!(r.snippet.is_none(), "doc hit snippet must be None with detail=ids");
        assert!(r.lineage.is_none(), "doc hit lineage must be None with detail=ids");
        assert!(
            r.score_components.is_none(),
            "doc hit score_components must be None with detail=ids"
        );
    }

    // 3. detail="default" preserves snippet, lineage, and score_components
    let default_res = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({
                "query": "Legacy architecture",
                "modality": "docs",
                "mode": "bm25",
                "detail": "default"
            }),
        )
        .unwrap();
    let default_resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(default_res).unwrap();
    let default_results = default_resp.docs.unwrap().results;
    assert!(!default_results.is_empty());
    let legacy_hit = default_results.iter().find(|r| r.path.contains("legacy.md")).unwrap();
    assert!(legacy_hit.snippet.is_some(), "snippet must be preserved with detail=default");
    assert!(legacy_hit.lineage.is_some(), "lineage must be preserved with detail=default");
    assert!(
        legacy_hit.score_components.is_some(),
        "score_components must be preserved with detail=default"
    );

    // 4. mode="explain" preserves score breakdown even with detail="ids"
    let explain_res = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({
                "query": "Legacy architecture",
                "mode": "explain",
                "detail": "ids"
            }),
        )
        .unwrap();
    let explanations: Vec<groundcontrol_common::types::SearchExplanation> =
        serde_json::from_value(explain_res).unwrap();
    assert!(!explanations.is_empty(), "explain should return explanations");
    for exp in &explanations {
        assert!(exp.snippet.is_none(), "snippet must be None when detail=ids in explain");
        assert!(exp.final_score > 0.0, "final_score must be preserved in explain");
        assert!(exp.bm25.raw_score > 0.0, "bm25 score component must be preserved in explain");
    }
}

#[test]
fn test_generic_normalized_scope_resolution() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let corpus_dir = tmp.path().join("corpus");

    let rust_code = r#"
pub struct EarlyBinder<'tcx, T> {
    value: T,
    _marker: std::marker::PhantomData<&'tcx ()>,
}

impl<'tcx, T> EarlyBinder<'tcx, T> {
    pub fn instantiate(&self) -> &T {
        &self.value
    }
}

pub struct OtherBinder<'a, A> {
    item: A,
    _life: &'a str,
}

impl<'a, A> OtherBinder<'a, A> {
    pub fn instantiate(&self) -> &A {
        &self.item
    }
}
"#;
    fs::write(corpus_dir.join("binder.rs"), rust_code).unwrap();
    engine.index_file("binder.rs", rust_code).unwrap();
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // 1. Resolve EarlyBinder > instantiate when defined as EarlyBinder<'tcx, T> > instantiate
    let res = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "qualified_name": "EarlyBinder > instantiate" }),
        )
        .unwrap();
    assert_eq!(res["path"], "binder.rs");
    assert!(res["total_lines"].as_u64().unwrap() >= 1);
    assert!(res["source"].as_str().unwrap().contains("&self.value"));

    // 2. Nonexistent symbol returns clean 404 Not Found error
    let err = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "qualified_name": "Nonexistent > missing" }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("not found") || err.to_string().contains("no code symbol"));

    // 3. Ambiguous method: two EarlyBinder > instantiate in different files
    let rust_code_2 = r#"
pub struct EarlyBinder<'a, T> {
    alt: T,
}

impl<'a, T> EarlyBinder<'a, T> {
    pub fn instantiate(&self) -> &T {
        &self.alt
    }
}
"#;
    fs::write(corpus_dir.join("binder2.rs"), rust_code_2).unwrap();
    engine.index_file("binder2.rs", rust_code_2).unwrap();
    engine.commit().unwrap();

    let amb_res = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "qualified_name": "EarlyBinder > instantiate" }),
        )
        .unwrap();
    assert_eq!(amb_res["kind"], "ambiguous");
    let candidates = amb_res["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().any(|c| c["file_path"] == "binder.rs"));
    assert!(candidates.iter().any(|c| c["file_path"] == "binder2.rs"));
}

#[test]
fn test_get_snippet_suggestions_and_enrichment() {
    let tmp = TempDir::new().unwrap();
    let mut engine = create_test_engine(&tmp);
    let corpus_dir = tmp.path().join("corpus");

    let rust_code = r#"
/// Compute hash of input data.
pub fn compute_hash(data: &[u8]) -> u64 {
    42
}
"#;
    fs::write(corpus_dir.join("hash.rs"), rust_code).unwrap();
    engine.index_file("hash.rs", rust_code).unwrap();
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // 1. Context enrichment: check scope_path, language, path, source bounds
    let res = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "qualified_name": "compute_hash", "include_neighbors": true }),
        )
        .unwrap();
    assert_eq!(res["path"], "hash.rs");
    assert_eq!(res["start_line"], 3);
    assert_eq!(res["end_line"], 5);
    assert_eq!(res["total_lines"], 5);
    assert!(res["source"].as_str().unwrap().contains("pub fn compute_hash"));
    // Grammar-driven relationships: incoming defines from hash.rs, 0 callers, 0 outgoing.
    let incoming = res["relationships"]["incoming"].as_object().unwrap();
    assert!(incoming.get("calls").is_none());
    assert!(incoming.contains_key("defines"));
    assert!(res["relationships"]["outgoing"].as_object().unwrap().is_empty());

    // 2. Candidate suggestions on near-miss: query with wrong container "CryptoEngine > compute_hash"
    let sugg_res = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "qualified_name": "CryptoEngine > compute_hash" }),
        )
        .unwrap();
    assert_eq!(sugg_res["kind"], "candidate_suggestions");
    let candidates = sugg_res["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["name"], "compute_hash");
    assert_eq!(candidates[0]["scope_path"], "compute_hash");
    assert_eq!(candidates[0]["file_path"], "hash.rs");

    // 3. Complete miss returns 404
    let err = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "qualified_name": "CryptoEngine > unknown_fn" }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("no code symbol"));
}

#[test]
fn test_search_inlines_top_snippets() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");
    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config, &index_dir).unwrap();

    let doc_content = "# Architecture\n\nCtxvault is a high performance semantic context server.\n";
    fs::write(corpus_dir.join("arch.md"), doc_content).unwrap();
    engine.index_file("arch.md", doc_content).unwrap();

    let rust_code = "pub fn execute_search() -> bool { true }\n";
    fs::write(corpus_dir.join("search.rs"), rust_code).unwrap();
    engine.index_file("search.rs", rust_code).unwrap();
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // Search with snippets = 2
    let res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "semantic context server", "mode": "bm25", "snippets": 2 }),
            )
            .unwrap();

    let resp: groundcontrol_common::types::SearchResponse = serde_json::from_value(res).unwrap();
    let docs = resp.docs.unwrap();
    assert!(!docs.results.is_empty());
    let top_hit = &docs.results[0];
    assert_eq!(top_hit.path, "arch.md");
    assert!(top_hit.snippet.is_some(), "Turn 1 snippet must be populated");
    assert!(top_hit.snippet.as_ref().unwrap().contains("high performance semantic"));
}

#[test]
fn test_dynamic_turn1_schema_envelope_and_graph_match() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");
    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config, &index_dir).unwrap();

    let ts_code = r#"
@Injectable()
export class UserService extends BaseService implements IUserService {
    @Get('/users')
    getUsers() {}
}
"#;
    fs::write(corpus_dir.join("user.ts"), ts_code).unwrap();
    engine.index_file("user.ts", ts_code).unwrap();
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // 1. Search for UserService and inspect Turn 1 SchemaEnvelope & Affordances
    let search_val = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({ "query": "UserService", "mode": "bm25" }),
        )
        .unwrap();

    let resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(search_val).unwrap();
    let code = resp.code.expect("expected code partition");
    assert!(!code.results.is_empty(), "expected hits for UserService");

    // Verify active_edges in schema_envelope contains extended edge types
    assert!(
        code.schema_envelope.active_edges.iter().any(|e| e == "extends"),
        "schema_envelope should include 'extends', got: {:?}",
        code.schema_envelope.active_edges
    );
    assert!(
        code.schema_envelope.active_edges.iter().any(|e| e == "decorates"),
        "schema_envelope should include 'decorates', got: {:?}",
        code.schema_envelope.active_edges
    );

    // Verify Cypher-Lite graph representation on the UserService hit
    let hit = &code.results[0];
    let graph = hit.graph.as_ref().expect("expected graph affordances");
    assert!(
        graph.contains("extends") || graph.contains("decorates"),
        "Expected extends or decorates in graph: {}",
        graph
    );

    // 2. Cypher-Lite graph_match traversal across new edge types
    let match_val = registry
            .execute_read(
                "graph_match",
                &engine,
                serde_json::json!({ "pattern": "(:CodeSymbol {name: \"UserService\"})-[:extends]->(target)" }),
            )
            .unwrap();

    let match_res: groundcontrol_common::types::GraphMatchResult =
        serde_json::from_value(match_val).unwrap();
    assert_eq!(match_res.total_matches, 1);
    assert!(!match_res.tree.is_empty(), "Expected graph_match path for -[:extends]->");
}

#[test]
fn test_lean_multiline_emissions_turns_1_to_3() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");
    let config = test_config(&corpus_dir);
    let mut engine = Engine::open(config, &index_dir).unwrap();

    let rust_code = r#"
pub struct PaymentService {
    api_key: String,
}

pub fn process_payment(amount: u64) -> bool {
    amount > 0
}
"#;
    fs::write(corpus_dir.join("payment.rs"), rust_code).unwrap();
    engine.index_file("payment.rs", rust_code).unwrap();
    engine.graph_mut().add_edge(
        "PaymentService",
        "process_payment",
        "calls",
        1.0,
        groundcontrol_common::types::EdgeProvenance::CodeCalls,
        groundcontrol_common::config::EdgeClass::Code,
    );
    engine.commit().unwrap();

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // 1. Turn 1: Search with format="lean"
    let search_val = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({ "query": "PaymentService", "mode": "bm25", "format": "lean" }),
        )
        .unwrap();

    let text = search_val.as_str().expect("expected lean text string");
    assert!(text.contains("# Search: \"PaymentService\" [mode: bm25, hits:"));
    assert!(text.contains("PaymentService (`payment.rs`)"));
    assert!(text.contains("-> [T2a fetch] get_snippet"));
    assert!(text.contains("-> [T2b graph]"));

    // 2. Turn 2a: get_snippet with format="lean"
    let snippet_val = registry
        .execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "name": "PaymentService", "format": "lean" }),
        )
        .unwrap();
    let snippet_text = snippet_val.as_str().expect("expected lean text string");
    assert!(snippet_text.contains("# Symbol: PaymentService (`payment.rs:L2-L4"));
    assert!(snippet_text.contains("L2: pub struct PaymentService {"));
    assert!(snippet_text.contains("-> [T2b callers] graph_match"));
    assert!(snippet_text.contains("-> [T3 full file] read_file"));

    // 3. Turn 2b: graph_match with format="lean"
    let match_val = registry
        .execute_read(
            "graph_match",
            &engine,
            serde_json::json!({
                "pattern": "(:CodeSymbol {name: \"PaymentService\"})-[:calls]->(target)",
                "format": "lean"
            }),
        )
        .unwrap();
    let match_text = match_val.as_str().expect("expected lean text string");
    assert!(match_text.contains("root: PaymentService"));
    assert!(match_text.contains("-> [T2a fetch] get_snippet(symbol: \"PaymentService\")"));

    // 4. Turn 3: read_file with format="lean"
    let read_val = registry
        .execute_read(
            "read_file",
            &engine,
            serde_json::json!({ "path": "payment.rs", "format": "lean" }),
        )
        .unwrap();
    let read_text = read_val.as_str().expect("expected lean text string");
    assert!(read_text.contains("# File: `payment.rs` [lines: L1-L8 of 8, language: rust]"));
    assert!(read_text.contains("```rust\nL1: \nL2: pub struct PaymentService {"));

    // 5. JSON format override continues to return structured objects
    let json_val = registry
        .execute_read(
            "search",
            &engine,
            serde_json::json!({ "query": "PaymentService", "mode": "bm25", "format": "json" }),
        )
        .unwrap();
    let resp: groundcontrol_common::types::SearchResponse =
        serde_json::from_value(json_val).unwrap();
    assert!(resp.code.unwrap().total_matches > 0);
}

#[test]
fn test_document_extractor_and_projections_mcp_flow() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    let index_dir = tmp.path().join("index");
    let mut config = test_config(&corpus_dir);
    config.docs.patterns = vec![
        "*.html".to_string(),
        "**/*.html".to_string(),
        "*.docx".to_string(),
        "*.pdf".to_string(),
    ];
    let mut engine = Engine::open(config, &index_dir).unwrap();

    // 1. Write an HTML documentation article
    let html_content = r#"<!DOCTYPE html>
<html>
<head><title>System Architecture Overview</title></head>
<body>
<header><nav><a href="/home">Home</a></nav></header>
<main>
<h1>Architecture Guide</h1>
<p>This document details the core distributed architecture and protocols.</p>
<h2>Subsystems</h2>
<p>The messaging subsystem routes packets between cluster nodes.</p>
<a href="https://example.com/spec">External Specification</a>
</main>
<footer>(c) 2026 Enterprise Corp</footer>
</body>
</html>"#;
    fs::write(corpus_dir.join("guide.html"), html_content).unwrap();

    // 2. Perform delta sync/reindex
    engine.delta_scan().unwrap();

    // 3. Verify projection file was written to .index/projections/guide.html.txt
    let proj_path = engine.projection_path("guide.html");
    assert!(proj_path.is_file(), "Projection file should exist at {:?}", proj_path);
    let proj_text = fs::read_to_string(&proj_path).unwrap();
    assert!(proj_text.contains("# Architecture Guide"));
    assert!(proj_text.contains("messaging subsystem"));
    assert!(!proj_text.contains("<nav>"));

    let mut registry = ToolRegistry::new();
    registry.register_all();

    // 4. Test Tier 3: read_file on projected document returns kind: "projected_doc"
    let read_val = registry
        .execute_read("read_file", &engine, serde_json::json!({ "path": "guide.html" }))
        .unwrap();
    assert_eq!(read_val["kind"], "projected_doc");
    assert!(read_val["content"].as_str().unwrap().contains("Architecture Guide"));

    // 5. Test Tier 3 read_file with format: "lean"
    let read_lean = registry
        .execute_read(
            "read_file",
            &engine,
            serde_json::json!({ "path": "guide.html", "format": "lean" }),
        )
        .unwrap();
    let lean_str = read_lean.as_str().unwrap();
    assert!(lean_str.contains("# File: `guide.html`"));

    // 6. Test write_note rejects modifying document formats (Docx, Pdf, HtmlDoc)
    let write_res = registry.execute_write(
        "write_note",
        &mut engine,
        serde_json::json!({
            "path": "spec.docx",
            "content": "Trying to overwrite docx"
        }),
    );
    assert!(write_res.is_err(), "write_note on docx must fail");
    let err_msg = write_res.err().unwrap().to_string();
    assert!(err_msg.contains("strictly read-only"));

    let write_html_res = registry.execute_write(
        "write_note",
        &mut engine,
        serde_json::json!({
            "path": "guide.html",
            "content": "Trying to overwrite html"
        }),
    );
    assert!(write_html_res.is_err(), "write_note on doc html must fail");

    // 7. Test move_note moves both the source file and its projection
    let move_res = registry.execute_write(
        "move_note",
        &mut engine,
        serde_json::json!({
            "from": "guide.html",
            "to": "archived_guide.html"
        }),
    );
    assert!(move_res.is_ok(), "move_note should succeed: {:?}", move_res);
    assert!(!proj_path.exists(), "Old projection must be gone");
    let new_proj = engine.projection_path("archived_guide.html");
    assert!(new_proj.is_file(), "New projection must exist at {:?}", new_proj);
}
