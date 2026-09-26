//! Metadata catalog port.

use std::collections::HashMap;

use crate::types::{
    ChunkRecord, CodeSymbol, EdgeTypeRecord, FileFormat, FileRecord, IndexingState,
};
use crate::Result;

/// Metadata catalog port: the durable record-keeping contract for a corpus.
///
/// This is the domain-facing contract for the SQLite-backed metadata store. It
/// covers file tracking, text chunks, code symbols, edge-type configuration,
/// key/value corpus config, and resumable indexing state. Every signature
/// speaks only [`crate::types`] domain records and standard-library types — no
/// backend type (`rusqlite::Connection`, statements, rows) ever crosses this
/// boundary, so consumers depend on the contract rather than on SQLite.
///
/// Construction (opening or creating the underlying database) is deliberately
/// **not** part of this port: it is an adapter/composition-root concern. The
/// port describes only the runtime behaviour a catalog must provide.
///
/// The `templates` and `validation_issues` tables exist in the schema but have
/// no accessor methods on the store today, so they are intentionally absent
/// from this contract; the port mirrors exactly the surface that is used.
pub trait MetadataCatalog {
    // ------------------------------------------------------------------
    // File tracking
    // ------------------------------------------------------------------

    /// Insert or replace a file record, stamping it with the current index time.
    fn insert_file(
        &self,
        path: &str,
        content_hash: &str,
        modified_at: i64,
        template: Option<&str>,
        title: Option<&str>,
        format: FileFormat,
    ) -> Result<()>;

    /// Retrieve a single file record by its corpus-relative path.
    fn get_file(&self, path: &str) -> Result<Option<FileRecord>>;

    /// Delete a file record and its associated chunks/issues (via cascade).
    fn delete_file(&self, path: &str) -> Result<()>;

    /// List all tracked files.
    fn list_files(&self) -> Result<Vec<FileRecord>>;

    // ------------------------------------------------------------------
    // Chunks
    // ------------------------------------------------------------------

    /// Insert the given chunks for a file within a single transaction.
    fn insert_chunks(&self, file_path: &str, chunks: &[ChunkRecord]) -> Result<()>;

    /// Retrieve all chunks for a file, ordered by chunk index.
    fn get_chunks_for_file(&self, file_path: &str) -> Result<Vec<ChunkRecord>>;

    /// Retrieve a single chunk for a file by its chunk index.
    fn get_chunk(&self, file_path: &str, chunk_index: usize) -> Result<Option<ChunkRecord>>;

    /// Delete all chunks for a file.
    fn delete_chunks_for_file(&self, file_path: &str) -> Result<()>;

    // ------------------------------------------------------------------
    // Edge types
    // ------------------------------------------------------------------

    /// Insert or replace edge-type configuration records within a transaction.
    fn insert_edge_types(&self, edge_types: &[EdgeTypeRecord]) -> Result<()>;

    /// List all registered edge types.
    fn list_edge_types(&self) -> Result<Vec<EdgeTypeRecord>>;

    // ------------------------------------------------------------------
    // Corpus config (key/value store)
    // ------------------------------------------------------------------

    /// Set a corpus configuration value for the given key.
    fn set_config(&self, key: &str, value: &str) -> Result<()>;

    /// Get a corpus configuration value by key, if present.
    fn get_config(&self, key: &str) -> Result<Option<String>>;

    // ------------------------------------------------------------------
    // Indexing state (resumable paginated indexing)
    // ------------------------------------------------------------------

    /// Retrieve the current indexing state for a corpus, if any.
    fn get_indexing_state(&self, corpus_id: &str) -> Result<Option<IndexingState>>;

    /// Insert or update the indexing state for a corpus.
    fn update_indexing_state(&self, state: &IndexingState) -> Result<()>;

    /// Reset (delete) the indexing state for a corpus.
    fn reset_indexing_state(&self, corpus_id: &str) -> Result<()>;

    // ------------------------------------------------------------------
    // Code symbols
    // ------------------------------------------------------------------

    /// Save the code symbols extracted from a file, replacing any existing ones.
    fn save_code_symbols(&self, file_path: &str, symbols: &[CodeSymbol]) -> Result<()>;

    /// Retrieve all code symbols defined in a file.
    fn get_code_symbols_for_file(&self, file_path: &str) -> Result<Vec<CodeSymbol>>;

    /// Retrieve all code symbols defined across a batch of files in a single query.
    fn get_code_symbols_for_files(
        &self,
        file_paths: &[&str],
    ) -> Result<HashMap<String, Vec<CodeSymbol>>> {
        let mut map = HashMap::new();
        for &path in file_paths {
            let syms = self.get_code_symbols_for_file(path)?;
            if !syms.is_empty() {
                map.insert(path.to_string(), syms);
            }
        }
        Ok(map)
    }

    /// Find code symbols matching a name pattern (fuzzy match).
    fn find_symbols_by_name(&self, name_pattern: &str) -> Result<Vec<CodeSymbol>>;

    /// Find code symbols whose fully qualified scope path matches exactly.
    fn find_symbols_by_qualified_name(&self, scope_path: &str) -> Result<Vec<CodeSymbol>>;

    /// Find code symbols whose scope path matches after normalizing generic parameters.
    fn find_symbols_by_normalized_scope(&self, scope_path: &str) -> Result<Vec<CodeSymbol>>;

    /// Retrieve all code symbols in the entire catalog.
    fn get_all_code_symbols(&self) -> Result<Vec<CodeSymbol>>;

    /// Flush the database write-ahead log or checkpoint changes to disk.
    fn checkpoint(&self) -> Result<()> {
        Ok(())
    }
}
