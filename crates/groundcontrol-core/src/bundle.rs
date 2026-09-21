//! Corpus index bundle export, validation, and import.
//!
//! Bundles a corpus `.index/` directory (BM25 Tantivy index, Petgraph `graph.bin`,
//! SQLite `meta.db`, and optional HNSW `vectors.bin`) into a single zstd-compressed
//! tar archive alongside a compatibility `manifest.json`.
//!
//! Enables zero-reindexing transfer of pre-computed indices between environments,
//! validating embedding model/dimension compatibility and graph schema versions.

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::engine::Engine;
use groundcontrol_common::error::{Error, Result};
use groundcontrol_common::ports::MetadataCatalog;

/// Name of the manifest file inside the archive.
pub const MANIFEST_FILENAME: &str = "manifest.json";

/// Metadata stamped into each exported corpus bundle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleManifest {
    /// Name of the corpus that was exported.
    pub corpus_name: String,
    /// Identifier of the embedding model used.
    pub embedding_model: String,
    /// Embedding vector dimensionality (e.g. 768).
    pub embedding_dims: usize,
    /// Stamped graph format schema version.
    pub graph_schema_version: u32,
    /// Groundcontrol crate version that produced the bundle.
    #[serde(alias = "ctxvault_version")]
    pub groundcontrol_version: String,
    /// Optional git commit SHA of the source code when indexed.
    pub source_commit: Option<String>,
}

/// Export a corpus engine's on-disk index into a zstd-compressed tar bundle.
pub fn export_bundle(
    engine: &mut Engine,
    out_path: &Path,
    source_commit: Option<String>,
) -> Result<BundleManifest> {
    // 1. Flush/checkpoint all pending changes to disk.
    engine.commit()?;
    let _ = engine.store().checkpoint();

    let manifest = BundleManifest {
        corpus_name: engine.config().name.clone(),
        embedding_model: engine.config().embedding.model.clone(),
        embedding_dims: engine.embedding_dimension(),
        graph_schema_version: crate::graph::GRAPH_SCHEMA_VERSION,
        groundcontrol_version: env!("CARGO_PKG_VERSION").to_string(),
        source_commit,
    };

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let out_file = File::create(out_path)?;
    let zstd_enc = zstd::Encoder::new(out_file, 3)?;
    let mut tar_builder = tar::Builder::new(zstd_enc);

    // 2. Add manifest.json as the first entry in the archive.
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| Error::Config(format!("failed to serialize bundle manifest: {e}")))?;
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest_bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar_builder.append_data(&mut header, MANIFEST_FILENAME, manifest_bytes.as_slice())?;

    // 3. Append all index files relative to index_dir.
    let index_dir = engine.index_dir().to_path_buf();
    append_dir_recursive(&mut tar_builder, &index_dir, Path::new(""))?;

    let zstd_enc = tar_builder.into_inner()?;
    zstd_enc.finish()?;

    Ok(manifest)
}

fn append_dir_recursive<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    base_dir: &Path,
    rel_dir: &Path,
) -> Result<()> {
    let current = base_dir.join(rel_dir);
    if !current.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(&current)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        // Skip SQLite WAL/shm, lockfiles, and hidden temp files
        if file_name_str.ends_with("-wal")
            || file_name_str.ends_with("-shm")
            || file_name_str.ends_with(".lock")
            || file_name_str == MANIFEST_FILENAME
        {
            continue;
        }

        let child_rel = if rel_dir.as_os_str().is_empty() {
            PathBuf::from(&file_name)
        } else {
            rel_dir.join(&file_name)
        };

        if file_type.is_dir() {
            append_dir_recursive(builder, base_dir, &child_rel)?;
        } else if file_type.is_file() {
            let full_path = entry.path();
            builder.append_path_with_name(&full_path, &child_rel)?;
        }
    }

    Ok(())
}

/// Read and validate a bundle's manifest without unpacking the index payload.
pub fn validate_bundle(
    bundle_path: &Path,
    expected_model: Option<&str>,
    expected_dims: Option<usize>,
) -> Result<BundleManifest> {
    let file = File::open(bundle_path)?;
    let decoder = zstd::Decoder::new(file)?;
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path == Path::new(MANIFEST_FILENAME) {
            let manifest: BundleManifest = serde_json::from_reader(&mut entry)
                .map_err(|e| Error::Config(format!("failed to parse bundle manifest: {e}")))?;

            // Validate graph schema version
            if manifest.graph_schema_version != crate::graph::GRAPH_SCHEMA_VERSION {
                return Err(Error::Config(format!(
                    "graph schema version mismatch: bundle has {}, current runtime requires {}",
                    manifest.graph_schema_version,
                    crate::graph::GRAPH_SCHEMA_VERSION
                )));
            }

            // Validate embedding model if specified
            if let Some(expected) = expected_model {
                if manifest.embedding_model != expected {
                    return Err(Error::Config(format!(
                        "embedding model mismatch: bundle has '{}', expected '{}'",
                        manifest.embedding_model, expected
                    )));
                }
            }

            // Validate embedding dimensions if specified
            if let Some(dims) = expected_dims {
                if manifest.embedding_dims != dims {
                    return Err(Error::Config(format!(
                        "embedding dimension mismatch: bundle has {}, expected {}",
                        manifest.embedding_dims, dims
                    )));
                }
            }

            return Ok(manifest);
        }
    }

    Err(Error::NotFound(format!(
        "bundle at {} does not contain {}",
        bundle_path.display(),
        MANIFEST_FILENAME
    )))
}

/// Unpack an index bundle into `target_index_dir` after verifying manifest compatibility.
pub fn import_bundle(
    bundle_path: &Path,
    target_index_dir: &Path,
    expected_model: Option<&str>,
    expected_dims: Option<usize>,
) -> Result<BundleManifest> {
    let manifest = validate_bundle(bundle_path, expected_model, expected_dims)?;

    fs::create_dir_all(target_index_dir)?;

    let file = File::open(bundle_path)?;
    let decoder = zstd::Decoder::new(file)?;
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let rel_path = entry.path()?.to_path_buf();
        if rel_path == Path::new(MANIFEST_FILENAME) {
            entry.unpack(target_index_dir.join(MANIFEST_FILENAME))?;
        } else {
            let out_file = target_index_dir.join(&rel_path);
            if let Some(parent) = out_file.parent() {
                fs::create_dir_all(parent)?;
            }
            entry.unpack(&out_file)?;
        }
    }

    Ok(manifest)
}

/// Detect portable compressed index bundle (.groundcontrol/vault.tar.zst or .ctxvault/vault.tar.zst) in the repository.
pub fn detect_bundle(corpus_root: &Path) -> Option<PathBuf> {
    let primary = corpus_root.join(".groundcontrol").join("vault.tar.zst");
    if primary.is_file() {
        return Some(primary);
    }
    let legacy = corpus_root.join(".ctxvault").join("vault.tar.zst");
    if legacy.is_file() {
        return Some(legacy);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use groundcontrol_common::config::*;
    use tempfile::TempDir;

    fn create_test_engine(tmp: &TempDir, name: &str) -> Engine {
        let corpus_dir = tmp.path().join(name);
        let index_dir = corpus_dir.join(".index");
        fs::create_dir_all(&index_dir).unwrap();

        let config = CorpusConfig {
            name: name.to_string(),
            path: corpus_dir.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Fast,
            chunking: Default::default(),
            embedding: EmbeddingConfig { model: "jina-embeddings-v2-base-code".to_string() },
            graph: Default::default(),
            templates_dir: None,
            exclude: Default::default(),
            docs: Default::default(),
        };

        let mut engine = Engine::open(config, &index_dir).unwrap();
        let content = "# Hello\n\nThis is test content for bundle export.\n";
        fs::write(corpus_dir.join("hello.md"), content).unwrap();
        engine.index_file("hello.md", content).unwrap();
        engine.commit().unwrap();
        engine
    }

    #[test]
    fn test_bundle_export_import_round_trip() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp, "origin");

        let bundle_file = tmp.path().join("origin.bundle.tar.zst");
        let manifest =
            export_bundle(&mut engine, &bundle_file, Some("abc1234".to_string())).unwrap();

        assert_eq!(manifest.corpus_name, "origin");
        assert_eq!(manifest.embedding_model, "jina-embeddings-v2-base-code");
        assert_eq!(manifest.source_commit.as_deref(), Some("abc1234"));
        assert_eq!(manifest.graph_schema_version, crate::graph::GRAPH_SCHEMA_VERSION);
        assert!(bundle_file.exists());

        // Validate bundle
        let validated =
            validate_bundle(&bundle_file, Some("jina-embeddings-v2-base-code"), Some(768)).unwrap();
        assert_eq!(validated, manifest);

        // Import into fresh directory
        let import_dir = tmp.path().join("imported_index");
        let imported_manifest = import_bundle(&bundle_file, &import_dir, None, None).unwrap();
        assert_eq!(imported_manifest, manifest);

        assert!(import_dir.join("graph.bin").exists());
        assert!(import_dir.join("meta.db").exists());
        assert!(import_dir.join(MANIFEST_FILENAME).exists());
    }

    #[test]
    fn test_bundle_validation_rejects_mismatch() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp, "mismatch_test");

        let bundle_file = tmp.path().join("test.bundle.tar.zst");
        export_bundle(&mut engine, &bundle_file, None).unwrap();

        // Wrong model name
        assert!(validate_bundle(&bundle_file, Some("wrong-model"), None).is_err());

        // Wrong dimension
        assert!(validate_bundle(&bundle_file, None, Some(1536)).is_err());
    }

    #[test]
    fn test_detect_bundle() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        assert!(detect_bundle(root).is_none());

        let bundle_dir = root.join(".ctxvault");
        fs::create_dir_all(&bundle_dir).unwrap();
        let bundle_path = bundle_dir.join("vault.tar.zst");
        fs::write(&bundle_path, b"dummy-bundle").unwrap();
        assert_eq!(detect_bundle(root), Some(bundle_path));
    }
}
