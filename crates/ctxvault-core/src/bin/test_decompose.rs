//! Quick test binary for search_multihop crash debugging.
//! Run with: cargo run --release -p ctxvault-core --bin test_decompose

use std::path::PathBuf;

use ctxvault_common::config::CorpusConfig;
use ctxvault_common::ports::{SearchQuery, SearchService};
use ctxvault_core::engine::Engine;
use ctxvault_core::search;

fn main() {
    eprintln!("=== test_decompose ===");

    // Enable backtraces.
    std::env::set_var("RUST_BACKTRACE", "1");
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        eprintln!("=== PANIC ===");
        eprintln!("{info}");
        eprintln!("{bt}");
    }));

    let corpus_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"c:\dev\ctx\ctxcorpus\anthropic"));
    let config_path = corpus_path.join("ctxvault.toml");

    eprintln!("Loading config from {:?}", config_path);
    let config: CorpusConfig = {
        let text = std::fs::read_to_string(&config_path).expect("read ctxvault.toml");
        toml::from_str(&text).expect("parse ctxvault.toml")
    };

    let index_dir = corpus_path.join(".index");
    eprintln!("Opening engine at {:?}", index_dir);
    let engine = Engine::open(config, &index_dir).expect("open engine");

    // Ensure embedder is ready.
    eprintln!("Ensuring embedder...");
    let _ = engine.ensure_embedder().expect("ensure_embedder");

    let query = "How do embeddings connect RAG to knowledge graphs?";
    eprintln!("Query: {query}");

    // Test decompose_query directly.
    let concepts = search::decompose_query(query);
    eprintln!("Decomposed into {} concepts: {:?}", concepts.len(), concepts);

    // Now dispatch a multi-hop hybrid search through the engine's search service.
    eprintln!("Calling multi-hop hybrid search...");
    let service = engine.search_service();
    let sq = SearchQuery {
        query: query.to_string(),
        mode: Some("hybrid".to_string()),
        limit: Some(10),
        modality: ctxvault_common::types::Modality::Both,
        depth: ctxvault_common::types::SearchDepth::default(),
        graph_depth: Some(2),
        edge_types: None,
        edge_class: None,
        decompose: Some(true),
        snippets: None,
    };
    match service.search(&sq) {
        Ok(results) => {
            eprintln!("SUCCESS: {} results", results.len());
            for (i, r) in results.iter().enumerate() {
                eprintln!("  {}: {} (score={:.4})", i + 1, r.path, r.score);
            }
        }
        Err(e) => {
            eprintln!("ERROR: {e}");
        }
    }

    eprintln!("=== done ===");
}
