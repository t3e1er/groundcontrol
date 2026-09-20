use std::path::Path;

use ctxvault_common::config::ChunkingConfig;
use ctxvault_common::types::{ExternalRefKind, ResolutionConfidence};

use super::CodeGraphExtractor;
use crate::parser::code::chunker::CodeChunker;

#[test]
fn test_language_specific_ast_edges() {
    let config = ChunkingConfig::default();

    // 1. TypeScript: decorates, extends, implements
    let ts_code = r#"
@Injectable()
export class UserService extends BaseService implements IUserService {
    @Get('/users')
    getUsers() {}
}
"#;
    let ts_res = CodeChunker::parse_and_chunk(Path::new("src/user.ts"), ts_code, &config).unwrap();
    let ts_edges = CodeGraphExtractor::extract_edges_for_file(
        Path::new("src/user.ts"),
        ts_code,
        &ts_res.symbols,
        &ts_res.symbols,
    );
    assert!(
        ts_edges.iter().any(|e| e.edge_type == "extends"
            && e.source == "UserService"
            && e.target == "BaseService"),
        "Expected UserService -[:extends]-> BaseService, got: {:?}",
        ts_edges
    );
    assert!(
        ts_edges.iter().any(|e| e.edge_type == "implements"
            && e.source == "UserService"
            && e.target == "IUserService"),
        "Expected UserService -[:implements]-> IUserService, got: {:?}",
        ts_edges
    );
    assert!(
        ts_edges.iter().any(|e| e.edge_type == "decorates" && e.target == "Injectable"),
        "Expected decorates Injectable, got: {:?}",
        ts_edges
    );

    // 2. Python: decorates, inherits
    let py_code = r#"
@app.route("/items")
class ItemView(BaseView):
    @login_required
    def get(self):
        pass
"#;
    let py_res = CodeChunker::parse_and_chunk(Path::new("app/views.py"), py_code, &config).unwrap();
    let py_edges = CodeGraphExtractor::extract_edges_for_file(
        Path::new("app/views.py"),
        py_code,
        &py_res.symbols,
        &py_res.symbols,
    );
    assert!(
        py_edges
            .iter()
            .any(|e| e.edge_type == "inherits" && e.source == "ItemView" && e.target == "BaseView"),
        "Expected ItemView -[:inherits]-> BaseView, got: {:?}",
        py_edges
    );
    assert!(
        py_edges.iter().any(|e| e.edge_type == "decorates" && e.target == "app.route"),
        "Expected decorates app.route, got: {:?}",
        py_edges
    );

    // 3. Rust: macro_expands
    let rs_code = r#"
pub fn run() {
    println!("hello");
    tokio::select! {
        _ = a => {}
    }
}
"#;
    let rs_res = CodeChunker::parse_and_chunk(Path::new("src/main.rs"), rs_code, &config).unwrap();
    let rs_edges = CodeGraphExtractor::extract_edges_for_file(
        Path::new("src/main.rs"),
        rs_code,
        &rs_res.symbols,
        &rs_res.symbols,
    );
    assert!(
        rs_edges.iter().any(|e| e.edge_type == "macro_expands" && e.target == "println"),
        "Expected run -[:macro_expands]-> println, got: {:?}",
        rs_edges
    );
    assert!(
        rs_edges.iter().any(|e| e.edge_type == "macro_expands" && e.target == "tokio::select"),
        "Expected run -[:macro_expands]-> tokio::select, got: {:?}",
        rs_edges
    );

    // 4. Go: struct_embeds
    let go_code = r#"
package server

import "sync"

type Server struct {
    sync.Mutex
    Logger
    port int
}
"#;
    let go_res =
        CodeChunker::parse_and_chunk(Path::new("server/server.go"), go_code, &config).unwrap();
    let go_edges = CodeGraphExtractor::extract_edges_for_file(
        Path::new("server/server.go"),
        go_code,
        &go_res.symbols,
        &go_res.symbols,
    );
    assert!(
        go_edges.iter().any(|e| e.edge_type == "struct_embeds" && e.target == "sync.Mutex"),
        "Expected Server -[:struct_embeds]-> sync.Mutex, got: {:?}",
        go_edges
    );
    assert!(
        go_edges.iter().any(|e| e.edge_type == "struct_embeds" && e.target == "Logger"),
        "Expected Server -[:struct_embeds]-> Logger, got: {:?}",
        go_edges
    );

    // 5. SQL: foreign_key
    let sql_code = r#"
CREATE TABLE orders (
    id INT PRIMARY KEY,
    user_id INT REFERENCES users(id),
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);
"#;
    let sql_res =
        CodeChunker::parse_and_chunk(Path::new("schema/orders.sql"), sql_code, &config).unwrap();
    let sql_edges = CodeGraphExtractor::extract_edges_for_file(
        Path::new("schema/orders.sql"),
        sql_code,
        &sql_res.symbols,
        &sql_res.symbols,
    );
    assert!(
        sql_edges.iter().any(|e| e.edge_type == "foreign_key" && e.target == "users"),
        "Expected orders -[:foreign_key]-> users, got: {:?}",
        sql_edges
    );
    assert!(
        sql_edges.iter().any(|e| e.edge_type == "foreign_key" && e.target == "accounts"),
        "Expected orders -[:foreign_key]-> accounts, got: {:?}",
        sql_edges
    );
}

#[test]
fn test_code_graph_extraction_calls_and_defines() {
    let code_a = r#"
pub struct SearchEngine;

impl SearchEngine {
    pub fn search(&self, q: &str) -> Vec<String> {
        let results = rrf_fuse(q);
        results
    }
}

pub fn rrf_fuse(q: &str) -> Vec<String> {
    vec![q.to_string()]
}
"#;
    let config = ChunkingConfig::default();
    let parse_res =
        CodeChunker::parse_and_chunk(Path::new("src/search.rs"), code_a, &config).unwrap();
    let edges = CodeGraphExtractor::extract_edges_for_file(
        Path::new("src/search.rs"),
        code_a,
        &parse_res.symbols,
        &parse_res.symbols,
    );

    // Check defines edges
    assert!(edges.iter().any(|e| e.edge_type == "defines" && e.target == "SearchEngine"));
    assert!(edges.iter().any(|e| e.edge_type == "defines" && e.target == "SearchEngine > search"));
    assert!(edges.iter().any(|e| e.edge_type == "defines" && e.target == "rrf_fuse"));

    // Check calls edges: SearchEngine > search calls rrf_fuse
    assert!(edges.iter().any(|e| e.edge_type == "calls"
        && e.source == "SearchEngine > search"
        && e.target == "rrf_fuse"));
}

#[test]
fn test_call_edge_confidence_unique_is_high() {
    // rrf_fuse resolves uniquely within the current file -> High confidence.
    let code = r#"
pub fn search(q: &str) -> Vec<String> {
    rrf_fuse(q)
}

pub fn rrf_fuse(q: &str) -> Vec<String> {
    vec![q.to_string()]
}
"#;
    let config = ChunkingConfig::default();
    let parse_res =
        CodeChunker::parse_and_chunk(Path::new("src/search.rs"), code, &config).unwrap();
    let edges = CodeGraphExtractor::extract_edges_for_file(
        Path::new("src/search.rs"),
        code,
        &parse_res.symbols,
        &parse_res.symbols,
    );

    let call_edge = edges
        .iter()
        .find(|e| e.edge_type == "calls" && e.target == "rrf_fuse")
        .expect("expected a call edge to rrf_fuse");
    assert_eq!(call_edge.confidence, Some(ResolutionConfidence::High));

    // defines edges are exact -> High.
    let define_edge =
        edges.iter().find(|e| e.edge_type == "defines").expect("expected a defines edge");
    assert_eq!(define_edge.confidence, Some(ResolutionConfidence::High));
}

#[test]
fn test_call_edge_confidence_ambiguous_is_medium_or_speculative() {
    // The caller file has no local `helper`; two workspace candidates named
    // `helper` exist in different files. One shares the caller's directory,
    // so the same-directory heuristic (case 3) applies -> Medium.
    let caller_code = r#"
pub fn run() {
    helper();
}
"#;
    let config = ChunkingConfig::default();
    let caller_res =
        CodeChunker::parse_and_chunk(Path::new("src/a/caller.rs"), caller_code, &config).unwrap();

    // Two distinct `helper` symbols in different files.
    let same_dir = r#"pub fn helper() {}"#;
    let other_dir = r#"pub fn helper() {}"#;
    let same_dir_res =
        CodeChunker::parse_and_chunk(Path::new("src/a/other.rs"), same_dir, &config).unwrap();
    let other_dir_res =
        CodeChunker::parse_and_chunk(Path::new("src/b/other.rs"), other_dir, &config).unwrap();

    let mut all_symbols = caller_res.symbols.clone();
    all_symbols.extend(same_dir_res.symbols.clone());
    all_symbols.extend(other_dir_res.symbols.clone());

    let edges = CodeGraphExtractor::extract_edges_for_file(
        Path::new("src/a/caller.rs"),
        caller_code,
        &caller_res.symbols,
        &all_symbols,
    );

    let call_edge = edges
        .iter()
        .find(|e| e.edge_type == "calls" && e.target == "helper")
        .expect("expected a call edge to helper");
    assert_eq!(
        call_edge.confidence,
        Some(ResolutionConfidence::Medium),
        "same-directory disambiguation should yield Medium confidence"
    );
    assert_ne!(call_edge.confidence, Some(ResolutionConfidence::High));
}

#[test]
fn test_hybrid_lsp_receiver_method_disambiguation_rust() {
    let caller_code = r#"
pub struct QueryService;

impl QueryService {
    pub fn execute(&self) {
        let client = SearchClient::new();
        client.query("rust");
    }
}
"#;
    let search_client_code = r#"
pub struct SearchClient;

impl SearchClient {
    pub fn new() -> Self { SearchClient }
    pub fn query(&self, q: &str) -> Vec<String> { vec![] }
}
"#;
    let db_client_code = r#"
pub struct DatabaseClient;

impl DatabaseClient {
    pub fn query(&self, sql: &str) -> Vec<String> { vec![] }
}
"#;
    let config = ChunkingConfig::default();
    let caller_res =
        CodeChunker::parse_and_chunk(Path::new("src/service.rs"), caller_code, &config).unwrap();
    let search_res =
        CodeChunker::parse_and_chunk(Path::new("src/search.rs"), search_client_code, &config)
            .unwrap();
    let db_res =
        CodeChunker::parse_and_chunk(Path::new("src/db.rs"), db_client_code, &config).unwrap();

    let mut all_symbols = caller_res.symbols.clone();
    all_symbols.extend(search_res.symbols.clone());
    all_symbols.extend(db_res.symbols.clone());

    let edges = CodeGraphExtractor::extract_edges_for_file(
        Path::new("src/service.rs"),
        caller_code,
        &caller_res.symbols,
        &all_symbols,
    );

    let call_edge = edges
        .iter()
        .find(|e| e.edge_type == "calls" && e.target == "SearchClient > query")
        .expect("expected a call edge to SearchClient > query");
    assert_eq!(call_edge.confidence, Some(ResolutionConfidence::High));
}

#[test]
fn test_hybrid_lsp_receiver_method_disambiguation_typescript() {
    let ts_caller = r#"
export class Controller {
    handleRequest() {
        const client = new ApiClient();
        client.fetchData();
    }
}
"#;
    let ts_target = r#"
export class ApiClient {
    fetchData() {
        return "data";
    }
}
"#;
    let config = ChunkingConfig::default();
    let caller_res =
        CodeChunker::parse_and_chunk(Path::new("src/controller.ts"), ts_caller, &config).unwrap();
    let target_res =
        CodeChunker::parse_and_chunk(Path::new("src/api.ts"), ts_target, &config).unwrap();

    let mut all_symbols = caller_res.symbols.clone();
    all_symbols.extend(target_res.symbols.clone());

    let edges = CodeGraphExtractor::extract_edges_for_file(
        Path::new("src/controller.ts"),
        ts_caller,
        &caller_res.symbols,
        &all_symbols,
    );

    let call_edge = edges
        .iter()
        .find(|e| e.edge_type == "calls" && e.target == "ApiClient > fetchData")
        .expect("expected a call edge to ApiClient > fetchData");
    assert_eq!(call_edge.confidence, Some(ResolutionConfidence::High));
}

#[test]
fn test_external_ref_capture_local_vs_unresolved_call() {
    // The caller defines `run`, calls the locally-defined `helper` (resolves to a
    // real in-corpus symbol) and `missing_external` (resolves to nothing).
    let code = r#"
pub fn helper() {}

pub fn run() {
    helper();
    missing_external();
}
"#;
    let config = ChunkingConfig::default();
    let res = CodeChunker::parse_and_chunk(Path::new("src/lib.rs"), code, &config).unwrap();

    let symbol_index = CodeGraphExtractor::build_symbol_index(&res.symbols);
    let extraction = CodeGraphExtractor::extract_edges_for_file_with_index(
        Path::new("src/lib.rs"),
        code,
        &res.symbols,
        &symbol_index,
    );

    // (a) The fully-local call yields a normal `calls` edge...
    let local_edge = extraction
        .edges
        .iter()
        .find(|e| e.edge_type == "calls" && e.target == "helper")
        .expect("expected a resolved `calls` edge to the local helper");
    assert_eq!(local_edge.confidence, Some(ResolutionConfidence::High));

    // ...and NO external ref for the resolved local call.
    assert!(
        !extraction.external_refs.iter().any(|r| r.raw_target == "helper"),
        "a fully-local call must not produce an ExternalRef, got: {:?}",
        extraction.external_refs
    );

    // (b) The unresolved external call is captured as an ExternalRef.
    let ext = extraction
        .external_refs
        .iter()
        .find(|r| r.raw_target == "missing_external" && r.kind == ExternalRefKind::Call)
        .expect("expected an ExternalRef for the unresolved external call");
    assert_eq!(ext.caller_scope_path, "run");
    assert_eq!(ext.confidence, ResolutionConfidence::Speculative);
}
