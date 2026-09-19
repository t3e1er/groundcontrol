//! Workspace-level integration tests.
//!
//! These tests verify cross-crate behavior: parsing → indexing → search.
//! They use tempfile for filesystem fixtures.

use ctxvault_common::config::{ChunkingConfig, CorpusConfig, CorpusMode, EmbeddingConfig, GraphConfig};

#[test]
fn corpus_config_round_trips_through_toml() {
    let config = CorpusConfig {
        name: "test-wiki".to_string(),
        path: "./test-data".to_string(),
        mode: CorpusMode::ReadWrite,
        index_mode: ctxvault_common::config::IndexMode::Full,
        chunking: ChunkingConfig::default(),
        embedding: EmbeddingConfig::default(),
        graph: GraphConfig::default(),
        templates_dir: Some(".templates".to_string()),
        exclude: ctxvault_common::config::ExcludeConfig::default(),
        docs: ctxvault_common::config::DocsConfig::default(),
    };

    let toml_str = toml::to_string_pretty(&config).expect("serialize to toml");
    let parsed: CorpusConfig = toml::from_str(&toml_str).expect("parse from toml");

    assert_eq!(parsed.name, "test-wiki");
    assert_eq!(parsed.mode, CorpusMode::ReadWrite);
    assert_eq!(parsed.chunking.target_tokens, 512);
}
