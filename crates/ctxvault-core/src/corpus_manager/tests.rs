//! Unit tests for CorpusManager routing, federation, and lifecycle.

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use ctxvault_common::config::{
    ChunkingConfig, CorpusConfig, CorpusMode, EdgeSource, EdgeTypeConfig, EmbeddingConfig,
    GraphConfig, IndexMode,
};
use ctxvault_common::ports::{GraphStore, MetadataCatalog};
use ctxvault_common::types::{Edge, EdgeProvenance, ResolutionConfidence};

use crate::corpus_manager::{CorpusManager, ResolverKind};
use crate::engine::Engine;

fn test_config(name: &str, corpus_path: &Path) -> CorpusConfig {
    let _ = fs::create_dir_all(corpus_path.join(".index"));
    CorpusConfig {
        name: name.to_string(),
        path: corpus_path.to_string_lossy().to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: IndexMode::Full,
        chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig { edge_types: Vec::new() },
        templates_dir: None,
        exclude: ctxvault_common::config::ExcludeConfig::default(),
        docs: ctxvault_common::config::DocsConfig::default(),
    }
}

fn add_test_corpus(manager: &mut CorpusManager, config: CorpusConfig) {
    let index_dir = PathBuf::from(&config.path).join(".index");
    manager.add_corpus_with_index_dir(config, &index_dir).unwrap();
}

#[test]
fn test_create_empty_manager() {
    let manager = CorpusManager::new();
    assert_eq!(manager.corpus_count(), 0);
    assert!(manager.default_corpus_name().is_none());
}

#[test]
fn test_add_corpus_sets_default() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("wiki");
    fs::create_dir_all(&corpus_dir).unwrap();

    let mut manager = CorpusManager::new();
    let config = test_config("wiki", &corpus_dir);
    add_test_corpus(&mut manager, config);

    assert_eq!(manager.corpus_count(), 1);
    assert_eq!(manager.default_corpus_name(), Some("wiki"));
    assert!(manager.has_corpus("wiki"));
}

#[test]
fn test_multiple_corpora_isolation() {
    let tmp = TempDir::new().unwrap();
    let wiki_dir = tmp.path().join("wiki");
    let docs_dir = tmp.path().join("docs");
    fs::create_dir_all(&wiki_dir).unwrap();
    fs::create_dir_all(&docs_dir).unwrap();

    let mut manager = CorpusManager::new();
    add_test_corpus(&mut manager, test_config("wiki", &wiki_dir));
    add_test_corpus(&mut manager, test_config("docs", &docs_dir));

    assert_eq!(manager.corpus_count(), 2);

    // Index a file in "wiki".
    {
        let wiki_engine = manager.get_engine_mut("wiki").unwrap();
        wiki_engine.index_file("test.md", "# Wiki Note\n\nContent for wiki.\n").unwrap();
        wiki_engine.commit().unwrap();
    }

    // Index a different file in "docs".
    {
        let docs_engine = manager.get_engine_mut("docs").unwrap();
        docs_engine.index_file("guide.md", "# Guide\n\nDocumentation guide.\n").unwrap();
        docs_engine.commit().unwrap();
    }

    // Wiki should have test.md but not guide.md.
    let wiki_engine = manager.get_engine("wiki").unwrap();
    assert!(wiki_engine.store().get_file("test.md").unwrap().is_some());
    assert!(wiki_engine.store().get_file("guide.md").unwrap().is_none());

    // Docs should have guide.md but not test.md.
    let docs_engine = manager.get_engine("docs").unwrap();
    assert!(docs_engine.store().get_file("guide.md").unwrap().is_some());
    assert!(docs_engine.store().get_file("test.md").unwrap().is_none());
}

#[test]
fn test_resolve_engine_with_corpus_param() {
    let tmp = TempDir::new().unwrap();
    let wiki_dir = tmp.path().join("wiki");
    let docs_dir = tmp.path().join("docs");
    fs::create_dir_all(&wiki_dir).unwrap();
    fs::create_dir_all(&docs_dir).unwrap();

    let mut manager = CorpusManager::new();
    add_test_corpus(&mut manager, test_config("wiki", &wiki_dir));
    add_test_corpus(&mut manager, test_config("docs", &docs_dir));

    // None resolves to default (wiki, since it was added first).
    {
        let engine = manager.resolve_engine_mut(None).unwrap();
        assert_eq!(engine.config().name, "wiki");
    }

    // Explicit name resolves correctly.
    {
        let engine = manager.resolve_engine_mut(Some("docs")).unwrap();
        assert_eq!(engine.config().name, "docs");
    }

    // Non-existent corpus returns error.
    assert!(manager.resolve_engine_mut(Some("nope")).is_err());
}

#[test]
fn test_list_corpora() {
    let tmp = TempDir::new().unwrap();
    let wiki_dir = tmp.path().join("wiki");
    fs::create_dir_all(&wiki_dir).unwrap();

    let mut manager = CorpusManager::new();
    add_test_corpus(&mut manager, test_config("wiki", &wiki_dir));

    let list = manager.list_corpora();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "wiki");
    assert_eq!(list[0].index_mode, "Full");
    assert_eq!(list[0].file_count, 0);
}

// ─── Cross-corpus symbol linking ─────────────────────────────────────────

/// Fast-mode corpus config with a single frontmatter `implements` edge type.
/// Fast mode skips embeddings, so no ONNX model is required.
fn linking_config(name: &str, corpus_path: &Path) -> CorpusConfig {
    let _ = fs::create_dir_all(corpus_path.join(".index"));
    let implements = EdgeTypeConfig {
        name: "implements".to_string(),
        source: EdgeSource::Frontmatter,
        weight: 1.0,
        bidirectional: false,
        field: Some("implements".to_string()),
        direction: None,
        max_frequency: None,
        class: None,
        description: None,
        allowed_source_templates: None,
        allowed_target_templates: None,
    };
    CorpusConfig {
        name: name.to_string(),
        path: corpus_path.to_string_lossy().to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: IndexMode::Fast,
        chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig { edge_types: vec![implements] },
        templates_dir: None,
        exclude: ctxvault_common::config::ExcludeConfig::default(),
        docs: ctxvault_common::config::DocsConfig::default(),
    }
}

/// Rust source defining exactly one top-level symbol with the given name.
/// The extracted `scope_path` for a top-level function equals its bare name.
fn rust_symbol_source(name: &str) -> String {
    format!("pub fn {name}() -> u32 {{\n    42\n}}\n")
}

/// Markdown doc whose frontmatter `implements` a code symbol scope_path.
fn doc_implementing(target_scope: &str) -> String {
    format!("---\nimplements: \"{target_scope}\"\n---\n\n# Design Note\n\nDescribes the impl.\n")
}

fn add_fast_corpus(manager: &mut CorpusManager, name: &str, root: &Path) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    let config = linking_config(name, &dir);
    let index_dir = dir.join(".index");
    manager.add_corpus_with_index_dir(config, &index_dir).unwrap();
}

#[test]
fn test_cross_corpus_unique_match_links() {
    let tmp = TempDir::new().unwrap();
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "A", tmp.path());
    add_fast_corpus(&mut manager, "B", tmp.path());

    // Corpus B uniquely defines a symbol `WidgetEngine`.
    {
        let b = manager.get_engine_mut("B").unwrap();
        b.index_file("src/widget.rs", &rust_symbol_source("WidgetEngine")).unwrap();
        b.commit().unwrap();
    }
    // Corpus A has a doc whose frontmatter implements that scope_path.
    {
        let a = manager.get_engine_mut("A").unwrap();
        a.index_file("design.md", &doc_implementing("WidgetEngine")).unwrap();
        a.commit().unwrap();
    }

    let created = manager.link_cross_corpus_symbols().unwrap();
    assert_eq!(created, 1, "exactly one cross-corpus edge should be created");

    // The doc's forward links must include the cross-corpus node.
    let a = manager.get_engine("A").unwrap();
    let node_key = "B::WidgetEngine";
    assert!(a.graph().contains_node(node_key), "cross-corpus node must exist");

    let edge = a
        .graph()
        .get_all_edges()
        .into_iter()
        .find(|e| e.source == "design.md" && e.target == node_key)
        .expect("cross-corpus edge must exist");
    assert_eq!(edge.target_corpus.as_deref(), Some("B"));
    assert_eq!(edge.confidence, Some(ResolutionConfidence::High));
    assert_eq!(edge.provenance, EdgeProvenance::DocumentsCode);
    assert_eq!(edge.edge_type, "implements");

    // Idempotent: re-running creates no additional edges.
    let created_again = manager.link_cross_corpus_symbols().unwrap();
    assert_eq!(created_again, 1, "re-run resolves the same single candidate (deduped in graph)");
    let a = manager.get_engine("A").unwrap();
    let dup_count = a
        .graph()
        .get_all_edges()
        .into_iter()
        .filter(|e| e.source == "design.md" && e.target == node_key)
        .count();
    assert_eq!(dup_count, 1, "no duplicate cross-corpus edge after re-run");
}

#[test]
fn test_cross_corpus_ambiguous_match_no_edge() {
    let tmp = TempDir::new().unwrap();
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "A", tmp.path());
    add_fast_corpus(&mut manager, "B", tmp.path());
    add_fast_corpus(&mut manager, "C", tmp.path());

    // The SAME scope_path is defined in BOTH B and C => ambiguous.
    {
        let b = manager.get_engine_mut("B").unwrap();
        b.index_file("src/dup.rs", &rust_symbol_source("Dup")).unwrap();
        b.commit().unwrap();
    }
    {
        let c = manager.get_engine_mut("C").unwrap();
        c.index_file("src/dup.rs", &rust_symbol_source("Dup")).unwrap();
        c.commit().unwrap();
    }
    {
        let a = manager.get_engine_mut("A").unwrap();
        a.index_file("design.md", &doc_implementing("Dup")).unwrap();
        a.commit().unwrap();
    }

    // Resolution should find two matches across corpora.
    assert_eq!(manager.resolve_symbol_across_corpora("Dup").len(), 2);

    let created = manager.link_cross_corpus_symbols().unwrap();
    assert_eq!(created, 0, "ambiguous target must not create an edge");

    let a = manager.get_engine("A").unwrap();
    assert!(!a.graph().contains_node("B::Dup"));
    assert!(!a.graph().contains_node("C::Dup"));
}

#[test]
fn test_cross_corpus_unresolved_no_edge() {
    let tmp = TempDir::new().unwrap();
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "A", tmp.path());
    add_fast_corpus(&mut manager, "B", tmp.path());

    // B defines something, but the doc implements a symbol nobody defines.
    {
        let b = manager.get_engine_mut("B").unwrap();
        b.index_file("src/other.rs", &rust_symbol_source("SomethingElse")).unwrap();
        b.commit().unwrap();
    }
    {
        let a = manager.get_engine_mut("A").unwrap();
        a.index_file("design.md", &doc_implementing("NoSuchSymbol")).unwrap();
        a.commit().unwrap();
    }

    assert!(manager.resolve_symbol_across_corpora("NoSuchSymbol").is_empty());

    let created = manager.link_cross_corpus_symbols().unwrap();
    assert_eq!(created, 0, "unresolved target must not create an edge");
}

#[test]
fn test_intra_corpus_doc_to_code_edge_aligns_on_scope_path() {
    let tmp = TempDir::new().unwrap();
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "mono", tmp.path());

    {
        let m = manager.get_engine_mut("mono").unwrap();
        m.index_file("src/thing.rs", &rust_symbol_source("Thing")).unwrap();
        m.index_file("design.md", &doc_implementing("Thing")).unwrap();
        m.commit().unwrap();
    }

    let m = manager.get_engine("mono").unwrap();
    assert!(m.graph().contains_node("Thing"));
    let fwd = m.graph().forwardlinks("design.md", None);
    let implements_targets = fwd.get("implements").expect("implements edge must exist");
    assert!(
        implements_targets.iter().any(|t| t == "Thing"),
        "intra-corpus doc->code edge must land on the symbol node"
    );

    let created = manager.link_cross_corpus_symbols().unwrap();
    assert_eq!(created, 0);
}

#[test]
fn test_ensure_and_unload_corpus() {
    let tmp = TempDir::new().unwrap();
    let repo_dir = tmp.path().join("dynamic_repo");
    fs::create_dir_all(&repo_dir).unwrap();
    fs::create_dir_all(repo_dir.join(".index")).unwrap();

    let mut manager = CorpusManager::new();
    let name = manager.ensure_corpus(&repo_dir).unwrap();
    assert_eq!(name, "dynamic_repo");
    assert!(manager.has_corpus("dynamic_repo"));
    assert_eq!(manager.default_corpus_name(), Some("dynamic_repo"));

    // Calling ensure_corpus again on same path returns existing name
    let name2 = manager.ensure_corpus(&repo_dir).unwrap();
    assert_eq!(name2, "dynamic_repo");
    assert_eq!(manager.corpus_count(), 1);

    // Unloading removes corpus
    assert!(manager.unload_corpus("dynamic_repo").unwrap());
    assert!(!manager.has_corpus("dynamic_repo"));
    assert_eq!(manager.corpus_count(), 0);
}

// ─── Cross-corpus ExternalRef resolution ─────────────────────────────────

fn rust_caller_source(callee: &str) -> String {
    format!("pub fn caller() -> u32 {{\n    {callee}()\n}}\n")
}

fn cross_code_edge_count(engine: &Engine) -> usize {
    engine
        .graph()
        .get_all_edges()
        .into_iter()
        .filter(|e| {
            e.target_corpus.is_some()
                && matches!(e.provenance, EdgeProvenance::CodeCalls | EdgeProvenance::CodeImports)
        })
        .count()
}

#[test]
fn test_resolve_external_refs_unique_match_links_both_graphs() {
    let tmp = TempDir::new().unwrap();
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "A", tmp.path());
    add_fast_corpus(&mut manager, "B", tmp.path());

    // B uniquely defines `targetfn`.
    {
        let b = manager.get_engine_mut("B").unwrap();
        b.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
        b.commit().unwrap();
    }
    // A has a caller that calls `targetfn` (unresolved locally => ExternalRef).
    {
        let a = manager.get_engine_mut("A").unwrap();
        a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
        a.commit().unwrap();
    }

    let created = manager.resolve_external_refs().unwrap();
    assert!(created >= 1, "a unique cross-corpus ref must create an edge");

    // Forward edge lives in A: caller -> "B::targetfn".
    let a = manager.get_engine("A").unwrap();
    let node_key = "B::targetfn";
    assert!(a.graph().contains_node(node_key));
    let fwd = a
        .graph()
        .get_all_edges()
        .into_iter()
        .find(|e| e.source == "caller" && e.target == node_key)
        .expect("forward cross-corpus edge must exist in A");
    assert_eq!(fwd.target_corpus.as_deref(), Some("B"));
    assert_eq!(fwd.target_kind.as_deref(), Some("Symbol"));
    assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));
    assert_eq!(fwd.edge_type, "calls");
    assert_eq!(fwd.provenance, EdgeProvenance::CodeCalls);

    // Reverse (mirror) edge lives in B: targetfn -> "A::caller".
    let b = manager.get_engine("B").unwrap();
    let reverse_key = "A::caller";
    assert!(b.graph().contains_node(reverse_key));
    let rev = b
        .graph()
        .get_all_edges()
        .into_iter()
        .find(|e| e.source == "targetfn" && e.target == reverse_key)
        .expect("reverse cross-corpus edge must exist in B");
    assert_eq!(rev.target_corpus.as_deref(), Some("A"));
    assert_eq!(rev.target_symbol.as_deref(), Some("caller"));
    assert_eq!(rev.confidence, Some(ResolutionConfidence::High));
    assert_eq!(rev.edge_type, "calls");
}

#[test]
fn test_resolve_external_refs_ambiguous_no_edge() {
    let tmp = TempDir::new().unwrap();
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "A", tmp.path());
    add_fast_corpus(&mut manager, "B", tmp.path());
    add_fast_corpus(&mut manager, "C", tmp.path());

    // Both B and C define `targetfn` => ambiguous.
    {
        let b = manager.get_engine_mut("B").unwrap();
        b.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
        b.commit().unwrap();
    }
    {
        let c = manager.get_engine_mut("C").unwrap();
        c.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
        c.commit().unwrap();
    }
    {
        let a = manager.get_engine_mut("A").unwrap();
        a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
        a.commit().unwrap();
    }

    assert_eq!(manager.resolve_symbol_across_corpora("targetfn").len(), 2);

    let created = manager.resolve_external_refs().unwrap();
    assert_eq!(created, 0, "ambiguous cross-corpus ref must not create an edge");

    // No cross edge to any "*::targetfn" node in A.
    let a = manager.get_engine("A").unwrap();
    assert!(!a.graph().contains_node("B::targetfn"));
    assert!(!a.graph().contains_node("C::targetfn"));
    assert_eq!(cross_code_edge_count(a), 0);
}

#[test]
fn test_resolve_external_refs_scip_tier_resolves_first() {
    let tmp = TempDir::new().unwrap();
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "A", tmp.path());
    add_fast_corpus(&mut manager, "B", tmp.path());

    assert!(
        manager.resolve_symbol_across_corpora("search").is_empty(),
        "no qualified-name `search` symbol may exist for this test"
    );
    {
        let b = manager.get_engine_mut("B").unwrap();
        let moniker = "scip-rust cargo b 0.0.1 search().".to_string();
        let edge = Edge {
            source: "src/search.rs".to_string(),
            target: moniker,
            edge_type: "defines".to_string(),
            weight: 1.0,
            provenance: EdgeProvenance::CodeDefines,
            target_corpus: None,
            confidence: Some(ResolutionConfidence::High),
            target_path: None,
            target_symbol: None,
            target_kind: None,
        };
        b.graph_mut().add_code_edge(&edge);
        b.commit().unwrap();
    }
    {
        let a = manager.get_engine_mut("A").unwrap();
        a.index_file("src/main.rs", &rust_caller_source("search")).unwrap();
        a.commit().unwrap();
    }

    let created = manager.resolve_external_refs().unwrap();
    assert!(created >= 1, "the SCIP moniker must resolve the cross-repo call");

    let a = manager.get_engine("A").unwrap();
    let node_key = "B::scip-rust cargo b 0.0.1 search().";
    assert!(a.graph().contains_node(node_key), "SCIP-resolved cross node must exist");
    let fwd = a
        .graph()
        .get_all_edges()
        .into_iter()
        .find(|e| e.source == "caller" && e.target == node_key)
        .expect("forward SCIP cross-corpus edge must exist in A");
    assert_eq!(fwd.target_corpus.as_deref(), Some("B"));
    assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));
    assert_eq!(fwd.edge_type, "calls");
    assert_eq!(fwd.target_symbol.as_deref(), Some("scip-rust cargo b 0.0.1 search()."));

    assert_eq!(
        manager.resolve_ref_across_corpora("A", "search").map(|(_, _, k)| k),
        Some(ResolverKind::Scip)
    );
}

#[test]
fn test_resolve_external_refs_falls_back_to_qualname() {
    let tmp = TempDir::new().unwrap();
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "A", tmp.path());
    add_fast_corpus(&mut manager, "B", tmp.path());

    {
        let b = manager.get_engine_mut("B").unwrap();
        b.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
        b.commit().unwrap();
    }
    {
        let a = manager.get_engine_mut("A").unwrap();
        a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
        a.commit().unwrap();
    }

    assert_eq!(
        manager.resolve_ref_across_corpora("A", "targetfn").map(|(_, _, k)| k),
        Some(ResolverKind::QualName)
    );

    let created = manager.resolve_external_refs().unwrap();
    assert!(created >= 1, "qualified-name fallback must resolve the cross-repo call");

    let a = manager.get_engine("A").unwrap();
    let fwd = a
        .graph()
        .get_all_edges()
        .into_iter()
        .find(|e| e.source == "caller" && e.target == "B::targetfn")
        .expect("qual-name forward cross edge must exist");
    assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));
}

#[test]
fn test_resolve_external_refs_idempotent() {
    let tmp = TempDir::new().unwrap();
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "A", tmp.path());
    add_fast_corpus(&mut manager, "B", tmp.path());

    {
        let b = manager.get_engine_mut("B").unwrap();
        b.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
        b.commit().unwrap();
    }
    {
        let a = manager.get_engine_mut("A").unwrap();
        a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
        a.commit().unwrap();
    }

    manager.resolve_external_refs().unwrap();
    let first = cross_code_edge_count(manager.get_engine("A").unwrap());
    assert!(first >= 1);

    manager.resolve_external_refs().unwrap();
    let second = cross_code_edge_count(manager.get_engine("A").unwrap());
    assert_eq!(first, second, "re-run must keep the cross-edge count stable");
}

#[test]
fn test_resolver_trust_ladder_scip_then_qualname() {
    let moniker = "scip-rust cargo b 0.0.1 targetfn().";

    // ── SCIP branch ──────────────────────────────────────────────────────
    {
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp.path());
        add_fast_corpus(&mut manager, "B", tmp.path());

        {
            let b = manager.get_engine_mut("B").unwrap();
            let edge = Edge {
                source: "src/lib.rs".to_string(),
                target: moniker.to_string(),
                edge_type: "defines".to_string(),
                weight: 1.0,
                provenance: EdgeProvenance::CodeDefines,
                target_corpus: None,
                confidence: Some(ResolutionConfidence::High),
                target_path: None,
                target_symbol: None,
                target_kind: None,
            };
            b.graph_mut().add_code_edge(&edge);
            b.commit().unwrap();
        }
        assert!(
            manager.resolve_symbol_across_corpora("targetfn").is_empty(),
            "SCIP branch must have no qualified-name `targetfn` symbol"
        );
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
            a.commit().unwrap();
        }

        let created = manager.resolve_external_refs().unwrap();
        assert!(created >= 1, "SCIP moniker must resolve the cross-repo call");

        let a = manager.get_engine("A").unwrap();
        let node_key = format!("B::{moniker}");
        assert!(a.graph().contains_node(&node_key), "SCIP-resolved cross node must exist");
        let fwd = a
            .graph()
            .get_all_edges()
            .into_iter()
            .find(|e| e.source == "caller" && e.target == node_key)
            .expect("forward SCIP cross edge must exist in A");
        assert_eq!(fwd.target_symbol.as_deref(), Some(moniker), "SCIP tier => moniker target");
        assert_eq!(fwd.target_corpus.as_deref(), Some("B"));
        assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));
        assert_eq!(fwd.edge_type, "calls");
    }

    // ── QualName fallback branch ─────────────────────────────────────────
    {
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp.path());
        add_fast_corpus(&mut manager, "B", tmp.path());

        {
            let b = manager.get_engine_mut("B").unwrap();
            b.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
            b.commit().unwrap();
        }
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
            a.commit().unwrap();
        }

        let created = manager.resolve_external_refs().unwrap();
        assert!(created >= 1, "qualified-name fallback must resolve the cross-repo call");

        let a = manager.get_engine("A").unwrap();
        let node_key = "B::targetfn";
        assert!(a.graph().contains_node(node_key), "qual-name cross node must exist");
        let fwd = a
            .graph()
            .get_all_edges()
            .into_iter()
            .find(|e| e.source == "caller" && e.target == node_key)
            .expect("forward qual-name cross edge must exist in A");
        assert_eq!(fwd.target_symbol.as_deref(), Some("targetfn"), "qual-name tier => scope_path");
        assert_ne!(
            fwd.target_symbol.as_deref(),
            Some(moniker),
            "qual-name target must not be a moniker"
        );
        assert_eq!(fwd.target_corpus.as_deref(), Some("B"));
        assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));
        assert_eq!(fwd.edge_type, "calls");
    }
}

// ─── Phase 5: infra resource matching (grammar-pure) ─────────────────────

#[test]
fn test_resolve_external_refs_links_infra_resource_bidirectional() {
    let tmp = TempDir::new().unwrap();
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "service_b", tmp.path());
    add_fast_corpus(&mut manager, "infra_d", tmp.path());

    {
        let d = manager.get_engine_mut("infra_d").unwrap();
        let tf_content = "resource \"aws_s3_bucket\" \"b\" {\n  bucket = \"my-bucket\"\n}\n";
        d.index_file("main.tf", tf_content).unwrap();
        d.commit().unwrap();
    }

    {
        let b = manager.get_engine_mut("service_b").unwrap();
        let caller_src = "pub fn upload() {\n    aws_s3_bucket.b();\n}\n";
        b.index_file("src/upload.rs", caller_src).unwrap();
        b.commit().unwrap();
    }

    let created = manager.resolve_external_refs().unwrap();
    assert!(created >= 1, "unresolved reference to infra resource must create a cross edge");

    let b = manager.get_engine("service_b").unwrap();
    let node_key = "infra_d::aws_s3_bucket.b";
    assert!(b.graph().contains_node(node_key), "proxy node for infra resource must exist");
    let fwd = b
        .graph()
        .get_all_edges()
        .into_iter()
        .find(|e| e.source == "upload" && e.target == node_key)
        .expect("forward cross edge to infra resource must exist");
    assert_eq!(fwd.target_corpus.as_deref(), Some("infra_d"));
    assert_eq!(fwd.target_symbol.as_deref(), Some("aws_s3_bucket.b"));
    assert_eq!(fwd.target_kind.as_deref(), Some("Resource"));
    assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));

    let d = manager.get_engine("infra_d").unwrap();
    let rev_target = "service_b::upload";
    assert!(d.graph().contains_node(rev_target), "reverse proxy caller node must exist in infra_d");
    let rev = d
        .graph()
        .get_all_edges()
        .into_iter()
        .find(|e| e.source == "aws_s3_bucket.b" && e.target == rev_target)
        .expect("reverse mirror edge from infra resource must exist");
    assert_eq!(rev.target_corpus.as_deref(), Some("service_b"));
    assert_eq!(rev.target_symbol.as_deref(), Some("upload"));
    assert_eq!(rev.target_kind.as_deref(), Some("Resource"));
    assert_eq!(rev.confidence, Some(ResolutionConfidence::High));
}

// ─── Phase 6: federated (cross-corpus continuation) traversal ────────────

fn rust_named_caller_source(caller_name: &str, callee: &str) -> String {
    format!("pub fn {caller_name}() -> u32 {{\n    {callee}()\n}}\n")
}

fn build_abc_chain(tmp: &Path) -> CorpusManager {
    let mut manager = CorpusManager::new();
    add_fast_corpus(&mut manager, "A", tmp);
    add_fast_corpus(&mut manager, "B", tmp);
    add_fast_corpus(&mut manager, "C", tmp);

    {
        let c = manager.get_engine_mut("C").unwrap();
        c.index_file("src/leaf.rs", &rust_symbol_source("leaf")).unwrap();
        c.commit().unwrap();
    }
    {
        let b = manager.get_engine_mut("B").unwrap();
        b.index_file("src/mid.rs", &rust_named_caller_source("mid", "leaf")).unwrap();
        b.commit().unwrap();
    }
    {
        let a = manager.get_engine_mut("A").unwrap();
        a.index_file("src/top.rs", &rust_named_caller_source("top", "mid")).unwrap();
        a.commit().unwrap();
    }

    let created = manager.resolve_external_refs().unwrap();
    assert!(created >= 2, "both A->B and B->C cross edges must be created");
    manager
}

#[test]
fn test_federated_traverse_continues_across_abc_chain() {
    let tmp = TempDir::new().unwrap();
    let manager = build_abc_chain(tmp.path());

    let result = manager
        .federated_traverse("A", "top", 4, 3, true)
        .expect("federated traversal from a mounted corpus must succeed");

    let hop_corpora: Vec<&str> = result.hops.iter().map(|h| h.to_corpus.as_str()).collect();
    assert!(
        hop_corpora.contains(&"B") && hop_corpora.contains(&"C"),
        "hops must name both target corpora B and C: {hop_corpora:?}"
    );
    let first_b = hop_corpora.iter().position(|c| *c == "B");
    let first_c = hop_corpora.iter().position(|c| *c == "C");
    assert!(first_b < first_c, "the B hop must precede the C hop: {hop_corpora:?}");

    let ab = result.hops.iter().find(|h| h.to_corpus == "B").unwrap();
    assert_eq!(ab.from_corpus, "A");
    assert_eq!(ab.to_node.as_deref(), Some("mid"));
    assert_eq!(ab.edge_type, "calls");
    assert_eq!(ab.corpus_depth, 1);

    assert!(
        result.nodes.iter().any(|n| n.corpus == "B" && n.node == "mid"),
        "continuation must reach B's `mid` node"
    );
    assert!(
        result.nodes.iter().any(|n| n.corpus == "C" && n.node == "leaf"),
        "continuation must reach C's `leaf` node"
    );
}

#[test]
fn test_federated_traverse_hop_budget_caps_deterministically() {
    let tmp = TempDir::new().unwrap();
    let manager = build_abc_chain(tmp.path());

    let result = manager.federated_traverse("A", "top", 4, 1, true).unwrap();

    assert!(
        result.nodes.iter().any(|n| n.corpus == "B" && n.node == "mid"),
        "a budget of 1 still enters B"
    );
    assert!(
        result.nodes.iter().all(|n| n.corpus != "C"),
        "a corpus-hop budget of 1 must NOT enqueue any C node: {:?}",
        result.nodes
    );

    let again = manager.federated_traverse("A", "top", 4, 1, true).unwrap();
    let nodes_one: Vec<(String, String)> =
        result.nodes.iter().map(|n| (n.corpus.clone(), n.node.clone())).collect();
    let nodes_two: Vec<(String, String)> =
        again.nodes.iter().map(|n| (n.corpus.clone(), n.node.clone())).collect();
    assert_eq!(nodes_one, nodes_two, "traversal order + result must be deterministic");
}

#[test]
fn test_federated_traverse_records_hops_without_continuing() {
    let tmp = TempDir::new().unwrap();
    let manager = build_abc_chain(tmp.path());

    let result = manager.federated_traverse("A", "top", 4, 3, false).unwrap();

    assert!(
        result.hops.iter().any(|h| h.to_corpus == "B"),
        "the A->B hop must still be recorded when continuation is off"
    );
    assert!(
        result.nodes.iter().all(|n| n.corpus == "A"),
        "with continuation off, no cross-corpus node may be entered: {:?}",
        result.nodes
    );
}

#[test]
fn test_federated_traverse_missing_corpus_and_node() {
    let tmp = TempDir::new().unwrap();
    let manager = build_abc_chain(tmp.path());

    assert!(manager.federated_traverse("ZZ", "top", 4, 3, true).is_err());

    let empty = manager.federated_traverse("A", "no_such_node", 4, 3, true).unwrap();
    assert!(empty.nodes.is_empty() && empty.hops.is_empty());
}

#[test]
fn test_ensure_corpus_with_name_unconfigured_imports_gitignore_and_registers() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    std::env::set_var("CTXV_CACHE_DIR", &cache_dir);

    let repo_dir = tmp.path().join("unconfigured_repo");
    fs::create_dir_all(&repo_dir).unwrap();
    fs::write(repo_dir.join(".gitignore"), "custom_build/\n*.secret\n").unwrap();

    let mut manager = CorpusManager::new();
    let name = manager.ensure_corpus_with_name(&repo_dir, Some("unconfigured_repo")).unwrap();
    assert_eq!(name, "unconfigured_repo");

    let engine = manager.get_engine(&name).unwrap();
    assert!(engine.config().exclude.patterns.contains(&"custom_build/".to_string()));
    assert!(engine.config().exclude.patterns.contains(&"*.secret".to_string()));

    let global = ctxvault_common::config::load_global_config();
    assert!(global.corpora.registered.contains_key("unconfigured_repo"));
    assert!(global.corpora.default.is_some());
}
