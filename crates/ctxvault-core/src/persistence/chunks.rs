//! Chunk storage persistence methods.

use rusqlite::params;

use ctxvault_common::types::ChunkRecord;
use ctxvault_common::{Error, Result};

use super::Store;

impl Store {
    /// Insert chunks for a file within a transaction.
    pub fn insert_chunks(&self, file_path: &str, chunks: &[ChunkRecord]) -> Result<()> {
        let conn = self.conn();
        if conn.is_autocommit() {
            let tx = conn.unchecked_transaction().map_err(|e| Error::Database(e.to_string()))?;
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO chunks (file_path, chunk_index, start_byte, end_byte, start_line, end_line)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )
                    .map_err(|e| Error::Database(e.to_string()))?;

                for chunk in chunks {
                    let _ = stmt
                        .execute(params![
                            file_path,
                            chunk.chunk_index as i64,
                            chunk.start_byte as i64,
                            chunk.end_byte as i64,
                            chunk.start_line as i64,
                            chunk.end_line as i64,
                        ])
                        .map_err(|e| Error::Database(e.to_string()))?;
                }
            }
            tx.commit().map_err(|e| Error::Database(e.to_string()))?;
        } else {
            let mut stmt = conn
                .prepare(
                    "INSERT INTO chunks (file_path, chunk_index, start_byte, end_byte, start_line, end_line)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(|e| Error::Database(e.to_string()))?;

            for chunk in chunks {
                let _ = stmt
                    .execute(params![
                        file_path,
                        chunk.chunk_index as i64,
                        chunk.start_byte as i64,
                        chunk.end_byte as i64,
                        chunk.start_line as i64,
                        chunk.end_line as i64,
                    ])
                    .map_err(|e| Error::Database(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Retrieve all chunks for a given file, ordered by chunk_index.
    pub fn get_chunks_for_file(&self, file_path: &str) -> Result<Vec<ChunkRecord>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT chunk_index, start_byte, end_byte, start_line, end_line
                 FROM chunks WHERE file_path = ?1 ORDER BY chunk_index",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![file_path], |row| {
                Ok(ChunkRecord {
                    chunk_index: row.get::<_, i64>(0)? as usize,
                    start_byte: row.get::<_, i64>(1)? as usize,
                    end_byte: row.get::<_, i64>(2)? as usize,
                    start_line: row.get::<_, i64>(3)? as usize,
                    end_line: row.get::<_, i64>(4)? as usize,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(|e| Error::Database(e.to_string()))
    }

    /// Retrieve a single chunk for a given file and chunk index.
    pub fn get_chunk(&self, file_path: &str, chunk_index: usize) -> Result<Option<ChunkRecord>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT chunk_index, start_byte, end_byte, start_line, end_line
                 FROM chunks WHERE file_path = ?1 AND chunk_index = ?2",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut rows = stmt
            .query_map(params![file_path, chunk_index as i64], |row| {
                Ok(ChunkRecord {
                    chunk_index: row.get::<_, i64>(0)? as usize,
                    start_byte: row.get::<_, i64>(1)? as usize,
                    end_byte: row.get::<_, i64>(2)? as usize,
                    start_line: row.get::<_, i64>(3)? as usize,
                    end_line: row.get::<_, i64>(4)? as usize,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        match rows.next() {
            Some(res) => res.map(Some).map_err(|e| Error::Database(e.to_string())),
            None => Ok(None),
        }
    }

    /// Delete all chunks for a given file.
    pub fn delete_chunks_for_file(&self, file_path: &str) -> Result<()> {
        let _ = self
            .conn()
            .execute("DELETE FROM chunks WHERE file_path = ?1", params![file_path])
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }
}
