//! Portable team graph artifact export and import (.ctxvault/vault.tar.zst).
//!
//! Enables zero-reindex onboarding across teams by packaging the derived indices
//! (SQLite metadata, Tantivy BM25, and Petgraph graph) into a compressed archive
//! with a compatibility manifest.

use ctxvault_core::bundle::{export_bundle, import_bundle};
use std::path::{Path, PathBuf};

/// Export the corpus index into a compressed team sharing artifact (.ctxvault/vault.tar.zst).
pub fn export_artifact(
    index_dir: &Path,
    repo_root: &Path,
    output_path: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    if !index_dir.exists() {
        anyhow::bail!("Index directory '{}' does not exist", index_dir.display());
    }

    let dest = if let Some(out) = output_path {
        out.to_path_buf()
    } else {
        repo_root.join(".ctxvault").join("vault.tar.zst")
    };

    let corpus_name =
        repo_root.file_name().and_then(|n| n.to_str()).unwrap_or("default").to_string();

    let config = ctxvault_common::config::CorpusConfig {
        name: corpus_name,
        path: repo_root.to_string_lossy().to_string(),
        mode: ctxvault_common::config::CorpusMode::ReadWrite,
        index_mode: ctxvault_common::config::IndexMode::Fast,
        chunking: Default::default(),
        embedding: Default::default(),
        graph: Default::default(),
        templates_dir: None,
        exclude: Default::default(),
        docs: Default::default(),
    };

    let mut engine = ctxvault_core::engine::Engine::open(config, index_dir)?;
    let manifest = export_bundle(&mut engine, &dest, None)?;
    tracing::info!(
        corpus = %manifest.corpus_name,
        schema_version = manifest.graph_schema_version,
        "exported index bundle"
    );

    Ok(dest)
}

/// Import a compressed team sharing artifact into the destination index directory.
pub fn import_artifact(src_path: &Path, dest_index_dir: &Path) -> anyhow::Result<PathBuf> {
    if !src_path.exists() {
        anyhow::bail!("Artifact archive '{}' does not exist", src_path.display());
    }

    let manifest = import_bundle(src_path, dest_index_dir, None, None)?;
    tracing::info!(
        corpus = %manifest.corpus_name,
        schema_version = manifest.graph_schema_version,
        "imported and validated index bundle"
    );

    Ok(dest_index_dir.to_path_buf())
}
