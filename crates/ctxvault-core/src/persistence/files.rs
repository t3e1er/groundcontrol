//! File tracking persistence methods.

use rusqlite::params;

use ctxvault_common::types::{FileFormat, FileRecord};
use ctxvault_common::{Error, Result};

use super::{now_unix, Store};

impl Store {
    /// Insert or replace a file record. Sets `indexed_at` to the current time.
    pub fn insert_file(
        &self,
        path: &str,
        content_hash: &str,
        modified_at: i64,
        template: Option<&str>,
        title: Option<&str>,
        format: FileFormat,
    ) -> Result<()> {
        let indexed_at = now_unix();
        let _ = self
            .conn()
            .execute(
                "INSERT OR REPLACE INTO files (path, content_hash, modified_at, template, title, indexed_at, format)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![path, content_hash, modified_at, template, title, indexed_at, format.as_str()],
            )
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Retrieve a single file record by path.
    pub fn get_file(&self, path: &str) -> Result<Option<FileRecord>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT path, content_hash, modified_at, template, title, indexed_at, format
                 FROM files WHERE path = ?1",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut rows = stmt
            .query_map(params![path], |row| {
                let fmt_str: String = row.get(6).unwrap_or_else(|_| "source".to_string());
                Ok(FileRecord {
                    path: row.get(0)?,
                    content_hash: row.get(1)?,
                    modified_at: row.get(2)?,
                    template: row.get(3)?,
                    title: row.get(4)?,
                    indexed_at: row.get(5)?,
                    format: FileFormat::from_str_name(&fmt_str),
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        match rows.next() {
            Some(Ok(record)) => Ok(Some(record)),
            Some(Err(e)) => Err(Error::Database(e.to_string())),
            None => Ok(None),
        }
    }

    /// Delete a file record and its associated chunks/validation issues (via CASCADE).
    pub fn delete_file(&self, path: &str) -> Result<()> {
        let _ = self
            .conn()
            .execute("DELETE FROM files WHERE path = ?1", params![path])
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// List all tracked files.
    pub fn list_files(&self) -> Result<Vec<FileRecord>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT path, content_hash, modified_at, template, title, indexed_at, format FROM files ORDER BY path",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let fmt_str: String = row.get(6).unwrap_or_else(|_| "source".to_string());
                Ok(FileRecord {
                    path: row.get(0)?,
                    content_hash: row.get(1)?,
                    modified_at: row.get(2)?,
                    template: row.get(3)?,
                    title: row.get(4)?,
                    indexed_at: row.get(5)?,
                    format: FileFormat::from_str_name(&fmt_str),
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(|e| Error::Database(e.to_string()))
    }
}
