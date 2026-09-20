//! Unit tests for persistence layer.

use ctxvault_common::types::{
    ChunkRecord, EdgeRecord, EdgeTypeRecord, FileFormat, IndexingState, IndexingStatus,
};

use super::Store;

#[test]
fn open_in_memory_creates_tables() {
    let store = Store::open_in_memory().expect("should open in-memory db");

    // Verify that all expected tables exist by querying sqlite_master.
    let tables: Vec<String> = {
        let conn = store.conn();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect()
    };

    assert!(tables.contains(&"files".to_string()));
    assert!(tables.contains(&"chunks".to_string()));
    assert!(tables.contains(&"edge_types".to_string()));
    assert!(tables.contains(&"templates".to_string()));
    assert!(tables.contains(&"validation_issues".to_string()));
}

#[test]
fn file_crud() {
    let store = Store::open_in_memory().unwrap();

    // Insert
    store
        .insert_file(
            "notes/hello.md",
            "abc123",
            1700000000,
            Some("daily"),
            Some("Hello"),
            FileFormat::Source,
        )
        .unwrap();

    // Get
    let record = store.get_file("notes/hello.md").unwrap().expect("should find file");
    assert_eq!(record.path, "notes/hello.md");
    assert_eq!(record.content_hash, "abc123");
    assert_eq!(record.modified_at, 1700000000);
    assert_eq!(record.template.as_deref(), Some("daily"));
    assert_eq!(record.title.as_deref(), Some("Hello"));
    assert_eq!(record.format, FileFormat::Source);
    assert!(record.indexed_at > 0);

    // List
    store
        .insert_file("notes/world.md", "def456", 1700000001, None, None, FileFormat::Source)
        .unwrap();
    let files = store.list_files().unwrap();
    assert_eq!(files.len(), 2);

    // Delete
    store.delete_file("notes/hello.md").unwrap();
    assert!(store.get_file("notes/hello.md").unwrap().is_none());
    assert_eq!(store.list_files().unwrap().len(), 1);
}

#[test]
fn chunk_storage_round_trip() {
    let store = Store::open_in_memory().unwrap();

    // Must have a parent file due to foreign key constraint.
    store.insert_file("doc.md", "hash1", 1700000000, None, None, FileFormat::Source).unwrap();

    let chunks = vec![
        ChunkRecord { chunk_index: 0, start_byte: 0, end_byte: 100, start_line: 1, end_line: 5 },
        ChunkRecord { chunk_index: 1, start_byte: 100, end_byte: 250, start_line: 6, end_line: 12 },
        ChunkRecord {
            chunk_index: 2,
            start_byte: 250,
            end_byte: 400,
            start_line: 13,
            end_line: 20,
        },
    ];

    store.insert_chunks("doc.md", &chunks).unwrap();

    let retrieved = store.get_chunks_for_file("doc.md").unwrap();
    assert_eq!(retrieved.len(), 3);
    assert_eq!(retrieved[0].chunk_index, 0);
    assert_eq!(retrieved[0].start_line, 1);
    assert_eq!(retrieved[0].end_line, 5);
    assert_eq!(retrieved[1].start_byte, 100);
    assert_eq!(retrieved[2].end_byte, 400);

    let single = store.get_chunk("doc.md", 1).unwrap().unwrap();
    assert_eq!(single.chunk_index, 1);
    assert_eq!(single.start_byte, 100);
    assert_eq!(single.end_byte, 250);

    // Delete chunks
    store.delete_chunks_for_file("doc.md").unwrap();
    assert!(store.get_chunks_for_file("doc.md").unwrap().is_empty());
}

#[test]
fn edge_type_storage_round_trip() {
    let store = Store::open_in_memory().unwrap();

    let edge_types = vec![
        EdgeTypeRecord {
            name: "Wikilink".to_string(),
            source: "wikilink".to_string(),
            weight: 1.0,
            bidirectional: false,
            field: None,
            config: None,
        },
        EdgeTypeRecord {
            name: "SharedTag".to_string(),
            source: "tag".to_string(),
            weight: 0.5,
            bidirectional: true,
            field: None,
            config: Some(r#"{"max_frequency": 100}"#.to_string()),
        },
        EdgeTypeRecord {
            name: "Implements".to_string(),
            source: "frontmatter".to_string(),
            weight: 0.8,
            bidirectional: false,
            field: Some("implements".to_string()),
            config: None,
        },
    ];

    store.insert_edge_types(&edge_types).unwrap();

    let retrieved = store.list_edge_types().unwrap();
    assert_eq!(retrieved.len(), 3);

    // Sorted by name: Implements, SharedTag, Wikilink
    assert_eq!(retrieved[0].name, "Implements");
    assert_eq!(retrieved[0].source, "frontmatter");
    assert_eq!(retrieved[0].field.as_deref(), Some("implements"));
    assert!(!retrieved[0].bidirectional);

    assert_eq!(retrieved[1].name, "SharedTag");
    assert!(retrieved[1].bidirectional);
    assert!((retrieved[1].weight - 0.5).abs() < f32::EPSILON);
    assert_eq!(retrieved[1].config.as_deref(), Some(r#"{"max_frequency": 100}"#));

    assert_eq!(retrieved[2].name, "Wikilink");
    assert!((retrieved[2].weight - 1.0).abs() < f32::EPSILON);
}

#[test]
fn cascade_delete_removes_chunks() {
    let store = Store::open_in_memory().unwrap();

    store.insert_file("cascade.md", "h1", 1700000000, None, None, FileFormat::Source).unwrap();
    store
        .insert_chunks(
            "cascade.md",
            &[ChunkRecord {
                chunk_index: 0,
                start_byte: 0,
                end_byte: 50,
                start_line: 1,
                end_line: 3,
            }],
        )
        .unwrap();

    // Deleting the file should cascade-delete its chunks.
    store.delete_file("cascade.md").unwrap();
    let chunks = store.get_chunks_for_file("cascade.md").unwrap();
    assert!(chunks.is_empty());
}

#[test]
fn insert_file_upsert_on_conflict() {
    let store = Store::open_in_memory().unwrap();

    store
        .insert_file("upsert.md", "hash_v1", 1000, None, Some("Title v1"), FileFormat::Source)
        .unwrap();
    store
        .insert_file(
            "upsert.md",
            "hash_v2",
            2000,
            Some("note"),
            Some("Title v2"),
            FileFormat::Source,
        )
        .unwrap();

    let record = store.get_file("upsert.md").unwrap().unwrap();
    assert_eq!(record.content_hash, "hash_v2");
    assert_eq!(record.modified_at, 2000);
    assert_eq!(record.template.as_deref(), Some("note"));
    assert_eq!(record.title.as_deref(), Some("Title v2"));

    // Should still be only one row.
    assert_eq!(store.list_files().unwrap().len(), 1);
}

#[test]
fn test_indexing_state_round_trip() {
    let store = Store::open_in_memory().unwrap();

    // Initially None
    assert!(store.get_indexing_state("corpus_a").unwrap().is_none());

    let state = IndexingState {
        corpus_id: "corpus_a".to_string(),
        status: IndexingStatus::Indexing,
        total_files: 100,
        indexed_files: 45,
        last_processed_path: Some("docs/intro.md".to_string()),
        started_at: 1700000000,
        updated_at: 1700000050,
        error_message: None,
    };

    store.update_indexing_state(&state).unwrap();

    let retrieved = store.get_indexing_state("corpus_a").unwrap().expect("should find state");
    assert_eq!(retrieved.corpus_id, "corpus_a");
    assert_eq!(retrieved.status, IndexingStatus::Indexing);
    assert_eq!(retrieved.total_files, 100);
    assert_eq!(retrieved.indexed_files, 45);
    assert_eq!(retrieved.last_processed_path.as_deref(), Some("docs/intro.md"));
    assert_eq!(retrieved.started_at, 1700000000);
    assert_eq!(retrieved.updated_at, 1700000050);
    assert!(retrieved.error_message.is_none());

    // Update to Completed
    let mut completed = state.clone();
    completed.status = IndexingStatus::Completed;
    completed.indexed_files = 100;
    completed.updated_at = 1700000100;
    store.update_indexing_state(&completed).unwrap();

    let updated = store.get_indexing_state("corpus_a").unwrap().unwrap();
    assert_eq!(updated.status, IndexingStatus::Completed);
    assert_eq!(updated.indexed_files, 100);

    // Reset
    store.reset_indexing_state("corpus_a").unwrap();
    assert!(store.get_indexing_state("corpus_a").unwrap().is_none());
}

#[test]
fn test_find_symbols_by_normalized_scope() {
    let store = Store::open_in_memory().unwrap();

    let sym1 = ctxvault_common::types::CodeSymbol {
        file_path: "binder.rs".to_string(),
        name: "instantiate".to_string(),
        scope_path: "EarlyBinder<'tcx, T> > instantiate".to_string(),
        symbol_type: ctxvault_common::types::CodeSymbolType::Function,
        language: "rust".to_string(),
        signature: "pub fn instantiate(&self) -> T".to_string(),
        docstring: None,
        start_line: 10,
        end_line: 20,
    };
    let sym2 = ctxvault_common::types::CodeSymbol {
        file_path: "binder.rs".to_string(),
        name: "peek".to_string(),
        scope_path: "EarlyBinder<'tcx, T> > peek".to_string(),
        symbol_type: ctxvault_common::types::CodeSymbolType::Function,
        language: "rust".to_string(),
        signature: "pub fn peek(&self)".to_string(),
        docstring: None,
        start_line: 22,
        end_line: 30,
    };
    let sym3 = ctxvault_common::types::CodeSymbol {
        file_path: "other.rs".to_string(),
        name: "instantiate".to_string(),
        scope_path: "OtherBinder<'a> > instantiate".to_string(),
        symbol_type: ctxvault_common::types::CodeSymbolType::Function,
        language: "rust".to_string(),
        signature: "pub fn instantiate(&self)".to_string(),
        docstring: None,
        start_line: 5,
        end_line: 15,
    };

    store.insert_file("binder.rs", "hash1", 1000, None, None, FileFormat::Source).unwrap();
    store.insert_file("other.rs", "hash2", 1000, None, None, FileFormat::Source).unwrap();
    store.insert_file("binder2.rs", "hash3", 1000, None, None, FileFormat::Source).unwrap();

    store.save_code_symbols("binder.rs", &[sym1, sym2]).unwrap();
    store.save_code_symbols("other.rs", &[sym3]).unwrap();

    // 1. Exact match works
    let exact = store.find_symbols_by_qualified_name("EarlyBinder<'tcx, T> > instantiate").unwrap();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].scope_path, "EarlyBinder<'tcx, T> > instantiate");

    // 2. Normalized scope query resolves generic scope path
    let normalized = store.find_symbols_by_qualified_name("EarlyBinder > instantiate").unwrap();
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].scope_path, "EarlyBinder<'tcx, T> > instantiate");

    // Direct call to find_symbols_by_normalized_scope
    let direct_norm = store.find_symbols_by_normalized_scope("EarlyBinder > instantiate").unwrap();
    assert_eq!(direct_norm.len(), 1);
    assert_eq!(direct_norm[0].scope_path, "EarlyBinder<'tcx, T> > instantiate");

    // 3. Nonexistent returns empty vec
    let missing = store.find_symbols_by_qualified_name("Nonexistent > instantiate").unwrap();
    assert!(missing.is_empty());

    // 4. Ambiguous methods across different types with same normalized name
    let sym4 = ctxvault_common::types::CodeSymbol {
        file_path: "binder2.rs".to_string(),
        name: "instantiate".to_string(),
        scope_path: "EarlyBinder<'a, A> > instantiate".to_string(),
        symbol_type: ctxvault_common::types::CodeSymbolType::Function,
        language: "rust".to_string(),
        signature: "pub fn instantiate(&self) -> A".to_string(),
        docstring: None,
        start_line: 1,
        end_line: 10,
    };
    store.save_code_symbols("binder2.rs", &[sym4]).unwrap();

    let ambiguous = store.find_symbols_by_qualified_name("EarlyBinder > instantiate").unwrap();
    assert_eq!(ambiguous.len(), 2, "should return both candidates for disambiguation");
    assert!(ambiguous.iter().any(|s| s.scope_path == "EarlyBinder<'tcx, T> > instantiate"));
    assert!(ambiguous.iter().any(|s| s.scope_path == "EarlyBinder<'a, A> > instantiate"));
}

#[test]
fn test_edges_table_crud_and_degrees() {
    let store = Store::open_in_memory().unwrap();

    let edges = vec![
        EdgeRecord {
            id: None,
            source: "file_a.rs > func_a".to_string(),
            target: "file_b.rs > func_b".to_string(),
            edge_type: "calls".to_string(),
            edge_class: "structural".to_string(),
            weight: 1.0,
            confidence: 1.0,
            metadata: None,
        },
        EdgeRecord {
            id: None,
            source: "file_c.rs > func_c".to_string(),
            target: "file_b.rs > func_b".to_string(),
            edge_type: "calls".to_string(),
            edge_class: "structural".to_string(),
            weight: 1.0,
            confidence: 1.0,
            metadata: None,
        },
        EdgeRecord {
            id: None,
            source: "file_b.rs > func_b".to_string(),
            target: "interface.rs > TraitB".to_string(),
            edge_type: "implements".to_string(),
            edge_class: "structural".to_string(),
            weight: 1.0,
            confidence: 1.0,
            metadata: None,
        },
        EdgeRecord {
            id: None,
            source: "file_b.rs > func_b".to_string(),
            target: "dep.rs".to_string(),
            edge_type: "imports".to_string(),
            edge_class: "structural".to_string(),
            weight: 1.0,
            confidence: 1.0,
            metadata: None,
        },
        EdgeRecord {
            id: None,
            source: "docs/spec.md".to_string(),
            target: "file_b.rs > func_b".to_string(),
            edge_type: "documents".to_string(),
            edge_class: "hybrid".to_string(),
            weight: 1.0,
            confidence: 1.0,
            metadata: None,
        },
    ];

    store.insert_edges(&edges).unwrap();

    let b_edges = store.get_edges_for_node("file_b.rs > func_b").unwrap();
    assert_eq!(b_edges.len(), 5);

    let deg = store.get_degree_counts("file_b.rs > func_b").unwrap();
    assert_eq!(deg.calls_in, Some(2));
    assert_eq!(deg.implements, Some(1));
    assert_eq!(deg.imports, Some(1));
    assert_eq!(deg.documents_code, Some(1));
    assert_eq!(deg.calls_out, None);

    // Delete edges for node
    store.delete_edges_for_node("file_b.rs > func_b").unwrap();
    let remaining = store.get_edges_for_node("file_b.rs > func_b").unwrap();
    assert!(remaining.is_empty());
}

#[test]
fn test_external_refs_round_trip_and_idempotent() {
    use ctxvault_common::types::{ExternalRef, ExternalRefKind, ResolutionConfidence};

    let store = Store::open_in_memory().unwrap();

    let refs = vec![
        ExternalRef {
            caller_scope_path: "src/a.rs > run".to_string(),
            raw_target: "external_crate::do_thing".to_string(),
            kind: ExternalRefKind::Call,
            confidence: ResolutionConfidence::Speculative,
        },
        ExternalRef {
            caller_scope_path: "src/a.rs".to_string(),
            raw_target: "serde::Serialize".to_string(),
            kind: ExternalRefKind::Import,
            confidence: ResolutionConfidence::Speculative,
        },
    ];

    store.insert_external_refs("src/a.rs", &refs).unwrap();

    // Round-trip: all rows read back, kind/confidence preserved.
    let all = store.get_external_refs().unwrap();
    assert_eq!(all.len(), 2);
    let call = all.iter().find(|r| r.kind == ExternalRefKind::Call).unwrap();
    assert_eq!(call.raw_target, "external_crate::do_thing");
    assert_eq!(call.confidence, ResolutionConfidence::Speculative);
    let import = all.iter().find(|r| r.kind == ExternalRefKind::Import).unwrap();
    assert_eq!(import.raw_target, "serde::Serialize");

    // Per-file read.
    assert_eq!(store.get_external_refs_for_file("src/a.rs").unwrap().len(), 2);

    // Idempotent re-index cycle (clear-then-insert) does not duplicate rows.
    store.clear_external_refs_for_file("src/a.rs").unwrap();
    store.insert_external_refs("src/a.rs", &refs).unwrap();
    assert_eq!(store.get_external_refs().unwrap().len(), 2);

    // Clearing one file leaves it empty.
    store.clear_external_refs_for_file("src/a.rs").unwrap();
    assert!(store.get_external_refs().unwrap().is_empty());
}
