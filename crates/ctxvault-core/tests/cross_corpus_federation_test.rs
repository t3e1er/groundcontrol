//! End-to-end integration test proving the cross-corpus federation worked case:
//! `api_gateway` (repo A) → `middleware_service` (repo B) → `database_host` (repo C) → `infra_deploy` (repo D)
//! with live multi-graph BFS continuation, hop reporting, infra Resource resolution,
//! and reverse return trace.

use std::fs;
use std::path::Path;
use tempfile::TempDir;

use ctxvault_common::config::{
    ChunkingConfig, CorpusConfig, CorpusMode, EmbeddingConfig, GraphConfig, IndexMode,
};
use ctxvault_core::corpus_manager::CorpusManager;

fn fast_corpus_config(name: &str, corpus_path: &Path) -> CorpusConfig {
    CorpusConfig {
        name: name.to_string(),
        path: corpus_path.to_string_lossy().to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: IndexMode::Fast,
        chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig::default(),
        templates_dir: None,
        exclude: ctxvault_common::config::ExcludeConfig::default(),
        docs: ctxvault_common::config::DocsConfig::default(),
    }
}

fn add_corpus(manager: &mut CorpusManager, name: &str, root: &Path) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).expect("create corpus root");
    let config = fast_corpus_config(name, &dir);
    let index_dir = dir.join(".index");
    manager.add_corpus_with_index_dir(config, &index_dir).expect("add corpus to manager");
}

#[test]
fn test_e2e_gateway_middleware_db_infra_federation() {
    let tmp = TempDir::new().expect("tempdir");
    let mut manager = CorpusManager::new();

    // 1. Setup 4 independent corpora representing distinct repositories:
    //    - api_gateway: HTTP routing & public handlers
    //    - middleware_service: core business logic & dispatch
    //    - database_host: data access layer & query handlers
    //    - infra_deploy: infrastructure definition (Terraform HCL)
    add_corpus(&mut manager, "api_gateway", tmp.path());
    add_corpus(&mut manager, "middleware_service", tmp.path());
    add_corpus(&mut manager, "database_host", tmp.path());
    add_corpus(&mut manager, "infra_deploy", tmp.path());

    // 2. Index infra_deploy with an HCL Terraform resource
    {
        let infra = manager.get_engine_mut("infra_deploy").unwrap();
        let tf_content = r#"
resource "aws_s3_bucket" "orders_data" {
  bucket = "prod-orders-data-bucket"
  force_destroy = false
}
"#;
        infra.index_file("terraform/storage.tf", tf_content).unwrap();
        infra.commit().unwrap();
    }

    // 3. Index database_host with a data query function referencing the infra resource
    {
        let db = manager.get_engine_mut("database_host").unwrap();
        let db_src = r#"
pub fn query_orders_table() -> u64 {
    aws_s3_bucket.orders_data();
    100
}
"#;
        db.index_file("src/orders.rs", db_src).unwrap();
        db.commit().unwrap();
    }

    // 4. Index middleware_service calling database_host's query function
    {
        let mw = manager.get_engine_mut("middleware_service").unwrap();
        let mw_src = r#"
pub fn process_order() -> bool {
    query_orders_table();
    true
}
"#;
        mw.index_file("src/service.rs", mw_src).unwrap();
        mw.commit().unwrap();
    }

    // 5. Index api_gateway calling middleware_service's process_order function
    {
        let gw = manager.get_engine_mut("api_gateway").unwrap();
        let gw_src = r#"
pub fn handle_post_order() -> u16 {
    process_order();
    200
}
"#;
        gw.index_file("src/routes.rs", gw_src).unwrap();
        gw.commit().unwrap();
    }

    // 6. Run Phase B cross-corpus external reference reconciliation
    let linked = manager.resolve_external_refs().expect("resolve cross-corpus external refs");
    assert!(
        linked >= 3,
        "must link at least 3 cross-corpus boundaries (A->B, B->C, C->D), got {linked}"
    );

    // 7. Perform forward federated traversal from api_gateway::handle_post_order
    //    with continuation across corpora up to 4 hops deep
    let forward_trace = manager
        .federated_traverse("api_gateway", "handle_post_order", 4, 5, true)
        .expect("forward federated trace");

    // Verify all 4 corpora were reached in the traversal
    let corpora_reached: Vec<String> =
        forward_trace.nodes.iter().map(|n| n.corpus.clone()).collect();
    assert!(corpora_reached.contains(&"api_gateway".to_string()));
    assert!(corpora_reached.contains(&"middleware_service".to_string()));
    assert!(corpora_reached.contains(&"database_host".to_string()));
    assert!(corpora_reached.contains(&"infra_deploy".to_string()));

    // Verify the specific symbols reached in each corpus
    assert!(forward_trace
        .nodes
        .iter()
        .any(|n| n.corpus == "api_gateway" && n.node == "handle_post_order"));
    assert!(forward_trace
        .nodes
        .iter()
        .any(|n| n.corpus == "middleware_service" && n.node == "process_order"));
    assert!(forward_trace
        .nodes
        .iter()
        .any(|n| n.corpus == "database_host" && n.node == "query_orders_table"));
    assert!(forward_trace
        .nodes
        .iter()
        .any(|n| n.corpus == "infra_deploy" && n.node == "aws_s3_bucket.orders_data"));

    // Verify forward chain hops are present with proper targets:
    let hop0 = forward_trace
        .hops
        .iter()
        .find(|h| h.from_corpus == "api_gateway" && h.to_corpus == "middleware_service")
        .expect("hop api_gateway -> middleware_service must exist");
    assert_eq!(hop0.to_node.as_deref(), Some("process_order"));
    assert_eq!(hop0.edge_type, "calls");

    let hop1 = forward_trace
        .hops
        .iter()
        .find(|h| h.from_corpus == "middleware_service" && h.to_corpus == "database_host")
        .expect("hop middleware_service -> database_host must exist");
    assert_eq!(hop1.to_node.as_deref(), Some("query_orders_table"));
    assert_eq!(hop1.edge_type, "calls");

    let hop2 = forward_trace
        .hops
        .iter()
        .find(|h| h.from_corpus == "database_host" && h.to_corpus == "infra_deploy")
        .expect("hop database_host -> infra_deploy must exist");
    assert_eq!(hop2.to_node.as_deref(), Some("aws_s3_bucket.orders_data"));
    assert_eq!(hop2.target_kind.as_deref(), Some("Resource"));

    // 8. Prove return path / reverse trace:
    //    Starting at infra_deploy::aws_s3_bucket.orders_data, trace backwards to api_gateway
    let reverse_trace = manager
        .federated_traverse("infra_deploy", "aws_s3_bucket.orders_data", 4, 5, true)
        .expect("reverse federated trace");

    // Reverse trace hops: infra_deploy -> database_host -> middleware_service -> api_gateway
    let rhop0 = reverse_trace
        .hops
        .iter()
        .find(|h| h.from_corpus == "infra_deploy" && h.to_corpus == "database_host")
        .expect("reverse hop infra_deploy -> database_host must exist");
    assert_eq!(rhop0.to_node.as_deref(), Some("query_orders_table"));
    assert_eq!(rhop0.target_kind.as_deref(), Some("Resource"));

    let rhop1 = reverse_trace
        .hops
        .iter()
        .find(|h| h.from_corpus == "database_host" && h.to_corpus == "middleware_service")
        .expect("reverse hop database_host -> middleware_service must exist");
    assert_eq!(rhop1.to_node.as_deref(), Some("process_order"));

    let rhop2 = reverse_trace
        .hops
        .iter()
        .find(|h| h.from_corpus == "middleware_service" && h.to_corpus == "api_gateway")
        .expect("reverse hop middleware_service -> api_gateway must exist");
    assert_eq!(rhop2.to_node.as_deref(), Some("handle_post_order"));

    // All original nodes are resolved on the return journey
    assert!(reverse_trace
        .nodes
        .iter()
        .any(|n| n.corpus == "api_gateway" && n.node == "handle_post_order"));
    assert!(reverse_trace
        .nodes
        .iter()
        .any(|n| n.corpus == "middleware_service" && n.node == "process_order"));
    assert!(reverse_trace
        .nodes
        .iter()
        .any(|n| n.corpus == "database_host" && n.node == "query_orders_table"));
    assert!(reverse_trace
        .nodes
        .iter()
        .any(|n| n.corpus == "infra_deploy" && n.node == "aws_s3_bucket.orders_data"));
}
