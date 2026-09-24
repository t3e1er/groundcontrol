//! Corpus configuration and indexing state persistence methods.

use rusqlite::params;

use groundcontrol_common::types::{IndexingState, IndexingStatus};
use groundcontrol_common::{Error, Result};

use super::{now_unix, Store};

impl Store {
    /// Set a configuration value.
    pub fn set_config(&self, key: &str, value: &str) -> Result<()> {
        let updated_at = now_unix();
        let _ = self
            .conn()
            .execute(
                "INSERT OR REPLACE INTO corpus_config (key, value, updated_at)
                 VALUES (?1, ?2, ?3)",
                params![key, value, updated_at],
            )
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Get a configuration value.
    pub fn get_config(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT value FROM corpus_config WHERE key = ?1")
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut rows = stmt
            .query_map(params![key], |row| row.get(0))
            .map_err(|e| Error::Database(e.to_string()))?;

        match rows.next() {
            Some(Ok(value)) => Ok(Some(value)),
            Some(Err(e)) => Err(Error::Database(e.to_string())),
            None => Ok(None),
        }
    }

    /// Retrieve the current indexing state for a corpus.
    pub fn get_indexing_state(&self, corpus_id: &str) -> Result<Option<IndexingState>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT corpus_id, status, total_files, indexed_files, last_processed_path, started_at, updated_at, error_message
                 FROM indexing_state WHERE corpus_id = ?1",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut rows = stmt
            .query_map(params![corpus_id], |row| {
                let status_str: String = row.get(1)?;
                let status = status_str.parse::<IndexingStatus>().unwrap_or(IndexingStatus::Idle);
                Ok(IndexingState {
                    corpus_id: row.get(0)?,
                    status,
                    total_files: row.get::<_, i64>(2)? as usize,
                    indexed_files: row.get::<_, i64>(3)? as usize,
                    last_processed_path: row.get(4)?,
                    started_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    error_message: row.get(7)?,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        match rows.next() {
            Some(Ok(state)) => Ok(Some(state)),
            Some(Err(e)) => Err(Error::Database(e.to_string())),
            None => Ok(None),
        }
    }

    /// Insert or update the indexing state for a corpus.
    pub fn update_indexing_state(&self, state: &IndexingState) -> Result<()> {
        let status_str = state.status.to_string();
        let _ = self
            .conn()
            .execute(
                "INSERT OR REPLACE INTO indexing_state (corpus_id, status, total_files, indexed_files, last_processed_path, started_at, updated_at, error_message)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    state.corpus_id,
                    status_str,
                    state.total_files as i64,
                    state.indexed_files as i64,
                    state.last_processed_path,
                    state.started_at,
                    state.updated_at,
                    state.error_message,
                ],
            )
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Reset or delete the indexing state for a corpus.
    pub fn reset_indexing_state(&self, corpus_id: &str) -> Result<()> {
        let _ = self
            .conn()
            .execute("DELETE FROM indexing_state WHERE corpus_id = ?1", params![corpus_id])
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }
}
