//! SQLite schema definition and database initialization.

use rusqlite::Connection;

use groundcontrol_common::{Error, Result};

/// SQL schema script for all tables and indices.
pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    modified_at INTEGER NOT NULL,
    template TEXT,
    title TEXT,
    indexed_at INTEGER NOT NULL,
    format TEXT NOT NULL DEFAULT 'source'
);

CREATE TABLE IF NOT EXISTS chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_chunks_file_chunk ON chunks(file_path, chunk_index);
CREATE INDEX IF NOT EXISTS idx_chunks_file_covering ON chunks(file_path, chunk_index, start_line, end_line, start_byte, end_byte);

CREATE TABLE IF NOT EXISTS edge_types (
    name TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    weight REAL NOT NULL DEFAULT 1.0,
    bidirectional INTEGER NOT NULL DEFAULT 0,
    field TEXT,
    config TEXT
);

CREATE TABLE IF NOT EXISTS templates (
    name TEXT PRIMARY KEY,
    definition TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS validation_issues (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    severity TEXT NOT NULL,
    message TEXT NOT NULL,
    field TEXT,
    checked_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS corpus_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS indexing_state (
    corpus_id TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    total_files INTEGER NOT NULL DEFAULT 0,
    indexed_files INTEGER NOT NULL DEFAULT 0,
    last_processed_path TEXT,
    started_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0,
    error_message TEXT
);

CREATE TABLE IF NOT EXISTS code_symbols (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    name TEXT NOT NULL,
    scope_path TEXT NOT NULL,
    symbol_type TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_code_symbols_name ON code_symbols(name);
CREATE INDEX IF NOT EXISTS idx_code_symbols_file ON code_symbols(file_path);
CREATE INDEX IF NOT EXISTS idx_code_symbols_scope ON code_symbols(scope_path);
CREATE INDEX IF NOT EXISTS idx_code_symbols_file_covering ON code_symbols(file_path, scope_path, symbol_type, start_line, end_line);
"#;

/// Apply pragmatic configurations and initialize schema tables.
pub fn initialize_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        PRAGMA busy_timeout = 10000;
        PRAGMA mmap_size = 268435456;
        PRAGMA temp_store = MEMORY;
        PRAGMA cache_size = -65536;
        PRAGMA synchronous = NORMAL;
        "#,
    )
    .map_err(|e| Error::Database(e.to_string()))?;
    conn.execute_batch(SCHEMA_SQL).map_err(|e| Error::Database(e.to_string()))?;
    Ok(())
}
