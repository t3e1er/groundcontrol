//! Edge types configuration persistence methods.

use rusqlite::params;

use groundcontrol_common::types::EdgeTypeRecord;
use groundcontrol_common::{Error, Result};

use super::Store;

impl Store {
    /// Insert or replace edge type records within a transaction.
    pub fn insert_edge_types(&self, edge_types: &[EdgeTypeRecord]) -> Result<()> {
        let conn = self.conn();
        let tx = conn.unchecked_transaction().map_err(|e| Error::Database(e.to_string()))?;

        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO edge_types (name, source, weight, bidirectional, field, config)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(|e| Error::Database(e.to_string()))?;

            for et in edge_types {
                let _ = stmt
                    .execute(params![
                        et.name,
                        et.source,
                        et.weight as f64,
                        et.bidirectional as i32,
                        et.field,
                        et.config,
                    ])
                    .map_err(|e| Error::Database(e.to_string()))?;
            }
        }

        tx.commit().map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// List all registered edge types.
    pub fn list_edge_types(&self) -> Result<Vec<EdgeTypeRecord>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT name, source, weight, bidirectional, field, config FROM edge_types ORDER BY name")
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(EdgeTypeRecord {
                    name: row.get(0)?,
                    source: row.get(1)?,
                    weight: row.get::<_, f64>(2)? as f32,
                    bidirectional: row.get::<_, i32>(3)? != 0,
                    field: row.get(4)?,
                    config: row.get(5)?,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(|e| Error::Database(e.to_string()))
    }
}
