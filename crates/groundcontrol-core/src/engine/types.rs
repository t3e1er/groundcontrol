//! Domain and status types for the Engine orchestrator.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use groundcontrol_common::types::{ChunkEmbedPolicy, IndexingStatus};
use groundcontrol_common::{Error, Result};

/// Detailed indexing status response for client queries and monitoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingStatusResponse {
    /// Corpus identifier/name.
    pub corpus_id: String,
    /// Current status (idle, indexing, paused, error, completed).
    pub status: IndexingStatus,
    /// Total markdown files discovered.
    pub total_files: usize,
    /// Count of markdown files successfully committed.
    pub indexed_files: usize,
    /// Progress as a percentage (0.0 - 100.0).
    pub progress_percent: f64,
    /// Relative path of last committed file.
    pub last_processed_path: Option<String>,
    /// When indexing started (Unix timestamp seconds).
    pub started_at: i64,
    /// When status was last updated (Unix timestamp seconds).
    pub updated_at: i64,
    /// Total elapsed time in seconds.
    pub elapsed_seconds: i64,
    /// Estimated indexing throughput in documents per second.
    pub estimated_throughput_docs_per_sec: f64,
    /// Estimated time remaining in seconds until completion.
    pub estimated_time_remaining_seconds: f64,
    /// Error message if status is Error.
    pub error_message: Option<String>,
}

/// Operational stage in the indexing lifecycle for progress reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexingStage {
    /// Initial repository scan and candidate discovery.
    Discovery,
    /// Parallel AST parsing, BM25 indexing, and SQLite ingestion.
    ParsingAndIndexing,
    /// Batch neural vector embedding generation via ONNX.
    GeneratingEmbeddings,
    /// Cross-file code reference and wikilink edge resolution.
    ResolvingGraphEdges,
    /// Committing database transactions and flushing search indices.
    Committing,
    /// Indexing process completed.
    Completed,
}

/// Real-time progress metrics emitted during indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingProgress {
    /// Current operational stage.
    pub stage: IndexingStage,
    /// Number of files processed so far.
    pub processed_files: usize,
    /// Total number of candidate files to index.
    pub total_files: usize,
    /// Relative path of the file currently being processed.
    pub current_path: Option<String>,
    /// Number of text chunks embedded so far.
    pub embedded_chunks: usize,
    /// Elapsed seconds since indexing started.
    pub elapsed_seconds: f64,
    /// Current indexing throughput in files per second.
    pub files_per_second: f64,
}

/// Callback function invoked when indexing progress updates are emitted.
pub type ProgressCallback = std::sync::Arc<dyn Fn(&IndexingProgress) + Send + Sync>;

/// A text chunk staged for vectorized batch embedding.
#[derive(Debug, Clone)]
pub struct PendingChunk {
    /// Document relative path.
    pub doc_path: String,
    /// Chunk index within document.
    pub chunk_index: usize,
    /// Prepared context-prefixed text for embedding.
    pub text: String,
    /// Policy determining if this chunk receives a dense vector embedding.
    pub embed_policy: ChunkEmbedPolicy,
    /// Coarse modality tag ("code" / "docs") carried into the vector index.
    pub modality: String,
}

/// Result of a delta scan comparing filesystem state against the index.
#[derive(Debug, Clone)]
pub struct DeltaScanResult {
    /// Files that exist on disk but were not previously indexed.
    pub new_files: Vec<String>,
    /// Files whose content hash changed since last indexing.
    pub modified_files: Vec<String>,
    /// Files that were indexed but no longer exist on disk.
    pub deleted_files: Vec<String>,
}

/// Current Unix timestamp in seconds.
pub(crate) fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("system time before epoch").as_secs() as i64
}

/// Recursively walk a directory and collect all indexable files (.md and polyglot source code).
/// Returns `(relative_path, absolute_path)` pairs.
pub(crate) fn walk_markdown_files(
    root: &Path,
    matcher: &crate::index::exclude::ExcludeMatcher,
    classifier: &crate::index::classifier::FileClassifier,
) -> Result<Vec<(String, PathBuf)>> {
    let mut results = Vec::new();
    if !root.exists() {
        return Ok(results);
    }
    walk_dir_recursive(root, root, matcher, classifier, &mut results)?;
    Ok(results)
}

fn walk_dir_recursive(
    root: &Path,
    current: &Path,
    matcher: &crate::index::exclude::ExcludeMatcher,
    classifier: &crate::index::classifier::FileClassifier,
    results: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    let entries = fs::read_dir(current)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Check exclusion for directories; prune subtrees immediately
            if matcher.is_excluded(&path, true) {
                continue;
            }
            walk_dir_recursive(root, &path, matcher, classifier, results)?;
        } else {
            // Check exclusion for individual files
            if matcher.is_excluded(&path, false) {
                continue;
            }
            let is_indexable = classifier.classify(&path, None).is_indexable();
            if is_indexable {
                let rel = path.strip_prefix(root).map_err(|e| {
                    Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                })?;
                // Normalize path separators to forward slashes.
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                results.push((rel_str, path.clone()));
            }
        }
    }
    Ok(())
}
