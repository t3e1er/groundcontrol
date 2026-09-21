//! Test Kubernetes traversal with dedicated EdgeClass::Code.

use groundcontrol_common::config::{
    ChunkingConfig, CorpusConfig, CorpusMode, EmbeddingConfig, GraphConfig, IndexMode,
};
use groundcontrol_common::ports::{MetadataCatalog, SearchQuery, SearchService};
use groundcontrol_common::types::{Modality, SearchDepth};
use groundcontrol_core::engine::Engine;
use std::path::PathBuf;

#[test]
#[ignore = "benchmark test on local Kubernetes corpus"]
fn test_kubernetes_traversal_with_code_edge_class() {
    let corpus_path = PathBuf::from("C:/dev/ctx/corpus/kubernetes");
    let index_dir = corpus_path.join(".index");
    if !index_dir.exists() {
        println!("Kubernetes index not found at {:?}, skipping test", index_dir);
        return;
    }

    println!("\n=== Testing Kubernetes Traversal on NewMainKubelet with EdgeClass::Code ===");
    let config = CorpusConfig {
        name: "kubernetes".to_string(),
        path: corpus_path.to_string_lossy().to_string(),
        mode: CorpusMode::ReadOnly,
        index_mode: IndexMode::Fast,
        chunking: ChunkingConfig::default(),
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig::default(),
        templates_dir: None,
        exclude: groundcontrol_common::config::ExcludeConfig::default(),
        docs: groundcontrol_common::config::DocsConfig::default(),
    };

    let start_open = std::time::Instant::now();
    let engine = Engine::open(config, &index_dir).expect("open engine");
    println!("Engine opened in {:?}", start_open.elapsed());

    // 1. Locate symbol in catalog
    let matches = engine.store().find_symbols_by_name("NewMainKubelet").expect("get symbols");
    println!("Found {} symbols named 'NewMainKubelet'", matches.len());
    for sym in &matches {
        println!(
            "  - file: {}, scope: {}, type: {:?}",
            sym.file_path, sym.scope_path, sym.symbol_type
        );
    }
    assert!(!matches.is_empty(), "NewMainKubelet must be present in catalog");

    let kubelet_sym = &matches[0];

    // 2. Petgraph affordances
    let affordances = engine.compute_affordances(&kubelet_sym.scope_path);
    println!("\nPetgraph affordances for {}:", kubelet_sym.scope_path);
    println!("  calls_in: {:?}, calls_out: {:?}", affordances.calls_in, affordances.calls_out);

    // 3. Test graph_match outbound (callees) with edge_class="code"
    let start_callees = std::time::Instant::now();
    let callees_result = engine
        .graph_match(
            "(:CodeSymbol {name: \"NewMainKubelet\"})-[:calls]->(target)",
            Some("code"),
            None,
            100,
            2,
        )
        .expect("graph_match callees");
    let callees_time = start_callees.elapsed();
    println!(
        "\nOutbound callees graph_match (edge_class=\"code\", limit=100) completed in {:?}",
        callees_time
    );
    println!(
        "  Matches: {}, Direct: {}, Transitive: {}, Files: {}, MaxDepth: {}",
        callees_result.total_matches,
        callees_result.summary.direct,
        callees_result.summary.transitive,
        callees_result.summary.files,
        callees_result.summary.max_depth,
    );
    for (i, node) in callees_result.tree.iter().take(10).enumerate() {
        println!(
            "    [{}] target node: '{}', rel: '{:?}', hop: {}",
            i + 1,
            node.node,
            node.rel,
            node.hop
        );
    }
    assert!(!callees_result.tree.is_empty(), "callees must not be empty");

    // 4. Test graph_match inbound (callers) with edge_class="code"
    let start_callers = std::time::Instant::now();
    let callers_result = engine
        .graph_match(
            "(:CodeSymbol {name: \"NewMainKubelet\"})<-[:calls]-(caller)",
            Some("code"),
            None,
            20,
            2,
        )
        .expect("graph_match callers");
    let callers_time = start_callers.elapsed();
    println!("\nInbound callers graph_match (edge_class=\"code\") completed in {:?}", callers_time);
    println!(
        "  Matches: {}, Direct: {}, Transitive: {}, Files: {}, MaxDepth: {}",
        callers_result.total_matches,
        callers_result.summary.direct,
        callers_result.summary.transitive,
        callers_result.summary.files,
        callers_result.summary.max_depth,
    );
    for (i, node) in callers_result.tree.iter().enumerate() {
        println!(
            "    [{}] caller node: '{}', rel: '{:?}', hop: {}",
            i + 1,
            node.node,
            node.rel,
            node.hop
        );
    }
    assert!(!callers_result.tree.is_empty(), "callers must not be empty");

    // 5. Test bi-modal search for NewMainKubelet
    println!("\nExecuting search_hybrid with modality=\"code\":");
    let query = SearchQuery {
        query: "NewMainKubelet".to_string(),
        mode: Some("hybrid".to_string()),
        limit: Some(5),
        modality: Modality::Code,
        depth: SearchDepth::Precise,
        graph_depth: None,
        edge_types: None,
        edge_class: Some("code".to_string()),
        decompose: None,
        snippets: Some(3),
    };
    let search_res = engine.search_service().search(&query).expect("search_hybrid");
    println!("Code search hits: {}", search_res.len());
    for (i, hit) in search_res.iter().enumerate() {
        println!("  [{}] {} (score: {:.4})", i + 1, hit.path, hit.score);
    }
    assert!(!search_res.is_empty(), "code search must return NewMainKubelet");
    assert_eq!(search_res[0].path, "code/pkg/kubelet/kubelet.go", "Top hit should be kubelet.go");

    println!("\n=== All Traversal & Search Checks Passed Successfully ===");
}
