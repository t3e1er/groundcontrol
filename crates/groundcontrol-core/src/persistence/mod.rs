//! Persistence layer: SQLite metadata, file tracking, incremental state.
//!
//! Provides a [`Store`] backed by SQLite for managing file records, chunks,
//! edge types, templates, and validation issues. Uses WAL mode for concurrency
//! and foreign keys for referential integrity.

pub mod chunks;
pub mod config;
pub mod edges;
pub mod files;
pub mod schema;
pub mod symbols;

#[cfg(test)]
mod tests;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use groundcontrol_common::ports::MetadataCatalog;
use groundcontrol_common::types::{
    ChunkRecord, CodeSymbol, EdgeRecord, EdgeTypeRecord, ExternalRef, FileFormat, FileRecord,
    GraphAffordances, IndexingState,
};
use groundcontrol_common::{Error, Result};

/// SQLite-backed persistence store for the groundcontrol engine.
pub struct Store {
    conn: std::sync::Mutex<Connection>,
}

impl Store {
    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("store connection mutex poisoned")
    }

    /// Open (or create) a SQLite database at the given path and run migrations.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(|e| Error::Database(e.to_string()))?;
        Self::initialize(conn)
    }

    /// Open an in-memory database (useful for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|e| Error::Database(e.to_string()))?;
        Self::initialize(conn)
    }

    /// Common initialization: pragmas + schema.
    fn initialize(conn: Connection) -> Result<Self> {
        schema::initialize_db(&conn)?;
        Ok(Self { conn: std::sync::Mutex::new(conn) })
    }

    /// Checkpoint the SQLite WAL journal.
    pub fn checkpoint(&self) -> Result<()> {
        let conn = self.conn();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Begin an intermediate batch transaction boundary if not already within a transaction.
    pub fn begin_batch(&self) -> Result<()> {
        let conn = self.conn();
        if conn.is_autocommit() {
            conn.execute_batch("BEGIN IMMEDIATE;").map_err(|e| Error::Database(e.to_string()))?;
        }
        Ok(())
    }

    /// Commit the active intermediate batch transaction boundary if open.
    pub fn commit_batch(&self) -> Result<()> {
        let conn = self.conn();
        if !conn.is_autocommit() {
            conn.execute_batch("COMMIT;").map_err(|e| Error::Database(e.to_string()))?;
        }
        Ok(())
    }

    /// Roll back the active intermediate batch transaction boundary if open.
    pub fn rollback_batch(&self) -> Result<()> {
        let conn = self.conn();
        if !conn.is_autocommit() {
            conn.execute_batch("ROLLBACK;").map_err(|e| Error::Database(e.to_string()))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Port adapter: MetadataCatalog
// ---------------------------------------------------------------------------

impl MetadataCatalog for Store {
    fn insert_file(
        &self,
        path: &str,
        content_hash: &str,
        modified_at: i64,
        template: Option<&str>,
        title: Option<&str>,
        format: FileFormat,
    ) -> Result<()> {
        Store::insert_file(self, path, content_hash, modified_at, template, title, format)
    }

    fn get_file(&self, path: &str) -> Result<Option<FileRecord>> {
        Store::get_file(self, path)
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        Store::delete_file(self, path)
    }

    fn list_files(&self) -> Result<Vec<FileRecord>> {
        Store::list_files(self)
    }

    fn insert_chunks(&self, file_path: &str, chunks: &[ChunkRecord]) -> Result<()> {
        Store::insert_chunks(self, file_path, chunks)
    }

    fn get_chunks_for_file(&self, file_path: &str) -> Result<Vec<ChunkRecord>> {
        Store::get_chunks_for_file(self, file_path)
    }

    fn get_chunk(&self, file_path: &str, chunk_index: usize) -> Result<Option<ChunkRecord>> {
        Store::get_chunk(self, file_path, chunk_index)
    }

    fn delete_chunks_for_file(&self, file_path: &str) -> Result<()> {
        Store::delete_chunks_for_file(self, file_path)
    }

    fn insert_edge_types(&self, edge_types: &[EdgeTypeRecord]) -> Result<()> {
        Store::insert_edge_types(self, edge_types)
    }

    fn list_edge_types(&self) -> Result<Vec<EdgeTypeRecord>> {
        Store::list_edge_types(self)
    }

    fn insert_edges(&self, edges: &[EdgeRecord]) -> Result<()> {
        Store::insert_edges(self, edges)
    }

    fn delete_edges_for_node(&self, path: &str) -> Result<()> {
        Store::delete_edges_for_node(self, path)
    }

    fn get_edges_for_node(&self, path: &str) -> Result<Vec<EdgeRecord>> {
        Store::get_edges_for_node(self, path)
    }

    fn get_degree_counts(&self, node: &str) -> Result<GraphAffordances> {
        Store::get_degree_counts(self, node)
    }

    fn clear_all_edges(&self) -> Result<()> {
        Store::clear_all_edges(self)
    }

    fn set_config(&self, key: &str, value: &str) -> Result<()> {
        Store::set_config(self, key, value)
    }

    fn get_config(&self, key: &str) -> Result<Option<String>> {
        Store::get_config(self, key)
    }

    fn get_indexing_state(&self, corpus_id: &str) -> Result<Option<IndexingState>> {
        Store::get_indexing_state(self, corpus_id)
    }

    fn update_indexing_state(&self, state: &IndexingState) -> Result<()> {
        Store::update_indexing_state(self, state)
    }

    fn reset_indexing_state(&self, corpus_id: &str) -> Result<()> {
        Store::reset_indexing_state(self, corpus_id)
    }

    fn save_code_symbols(&self, file_path: &str, symbols: &[CodeSymbol]) -> Result<()> {
        Store::save_code_symbols(self, file_path, symbols)
    }

    fn get_code_symbols_for_file(&self, file_path: &str) -> Result<Vec<CodeSymbol>> {
        Store::get_code_symbols_for_file(self, file_path)
    }

    fn get_code_symbols_for_files(
        &self,
        file_paths: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<CodeSymbol>>> {
        Store::get_code_symbols_for_files(self, file_paths)
    }

    fn find_symbols_by_name(&self, name_pattern: &str) -> Result<Vec<CodeSymbol>> {
        Store::find_symbols_by_name(self, name_pattern)
    }

    fn find_symbols_by_qualified_name(&self, scope_path: &str) -> Result<Vec<CodeSymbol>> {
        Store::find_symbols_by_qualified_name(self, scope_path)
    }

    fn find_symbols_by_normalized_scope(&self, scope_path: &str) -> Result<Vec<CodeSymbol>> {
        Store::find_symbols_by_normalized_scope(self, scope_path)
    }

    fn get_all_code_symbols(&self) -> Result<Vec<CodeSymbol>> {
        Store::get_all_code_symbols(self)
    }

    fn clear_external_refs_for_file(&self, file_path: &str) -> Result<()> {
        Store::clear_external_refs_for_file(self, file_path)
    }

    fn insert_external_refs(&self, file_path: &str, refs: &[ExternalRef]) -> Result<()> {
        Store::insert_external_refs(self, file_path, refs)
    }

    fn get_external_refs(&self) -> Result<Vec<ExternalRef>> {
        Store::get_external_refs(self)
    }

    fn get_external_refs_for_file(&self, file_path: &str) -> Result<Vec<ExternalRef>> {
        Store::get_external_refs_for_file(self, file_path)
    }

    fn checkpoint(&self) -> Result<()> {
        Store::checkpoint(self)
    }
}

/// Current Unix timestamp in seconds.
pub(crate) fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}
