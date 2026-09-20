//! Relational edges and edge types persistence methods.

use rusqlite::params;

use ctxvault_common::types::{EdgeRecord, EdgeTypeRecord, GraphAffordances};
use ctxvault_common::{Error, Result};

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

    /// Insert a batch of relational edges within a single transaction.
    pub fn insert_edges(&self, edges: &[EdgeRecord]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(|e| Error::Database(e.to_string()))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO edges (source, target, edge_type, edge_class, weight, confidence, metadata)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .map_err(|e| Error::Database(e.to_string()))?;

            for edge in edges {
                stmt.execute(params![
                    edge.source,
                    edge.target,
                    edge.edge_type,
                    edge.edge_class,
                    edge.weight,
                    edge.confidence,
                    edge.metadata,
                ])
                .map_err(|e| Error::Database(e.to_string()))?;
            }
        }
        tx.commit().map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Delete all edges where the given path is source or target.
    pub fn delete_edges_for_node(&self, path: &str) -> Result<()> {
        let _ = self
            .conn()
            .execute("DELETE FROM edges WHERE source = ?1 OR target = ?1", params![path])
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Retrieve all incident edges (where node is source or target).
    pub fn get_edges_for_node(&self, path: &str) -> Result<Vec<EdgeRecord>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, source, target, edge_type, edge_class, weight, confidence, metadata
                 FROM edges WHERE source = ?1 OR target = ?1",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![path], |row| {
                Ok(EdgeRecord {
                    id: Some(row.get(0)?),
                    source: row.get(1)?,
                    target: row.get(2)?,
                    edge_type: row.get(3)?,
                    edge_class: row.get(4)?,
                    weight: row.get(5)?,
                    confidence: row.get(6)?,
                    metadata: row.get(7)?,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(|e| Error::Database(e.to_string()))
    }

    /// Retrieve degree affordance counts for a node using indexed SQLite aggregation.
    pub fn get_degree_counts(&self, node: &str) -> Result<GraphAffordances> {
        let conn = self.conn();

        let mut out_stmt = conn
            .prepare("SELECT edge_type, COUNT(*) FROM edges WHERE source = ?1 GROUP BY edge_type")
            .map_err(|e| Error::Database(e.to_string()))?;
        let out_rows = out_stmt
            .query_map(params![node], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut in_stmt = conn
            .prepare("SELECT edge_type, COUNT(*) FROM edges WHERE target = ?1 GROUP BY edge_type")
            .map_err(|e| Error::Database(e.to_string()))?;
        let in_rows = in_stmt
            .query_map(params![node], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        let mut affordances = GraphAffordances::default();

        for item in out_rows {
            let (edge_type, count) = item.map_err(|e| Error::Database(e.to_string()))?;
            match edge_type.as_str() {
                "calls" => affordances.calls_out = Some(count),
                "implements" => affordances.implements = Some(count),
                "imports" => affordances.imports = Some(count),
                "wikilink" => affordances.wikilinks_out = Some(count),
                "documents" => affordances.documents_code = Some(count),
                _ => {}
            }
        }

        for item in in_rows {
            let (edge_type, count) = item.map_err(|e| Error::Database(e.to_string()))?;
            match edge_type.as_str() {
                "calls" => affordances.calls_in = Some(count),
                "wikilink" => affordances.wikilinks_in = Some(count),
                "documents" => {
                    let existing = affordances.documents_code.unwrap_or(0);
                    affordances.documents_code = Some(existing + count);
                }
                _ => {}
            }
        }

        Ok(affordances)
    }

    /// Remove all edges from the database.
    pub fn clear_all_edges(&self) -> Result<()> {
        let _ = self
            .conn()
            .execute("DELETE FROM edges", [])
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }
}
