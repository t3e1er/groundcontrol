//! Identifier persistence methods for cross-corpus lookup (`where` tool).

use rusqlite::params;

use groundcontrol_common::types::IdentifierRecord;
use groundcontrol_common::{Error, Result};

use super::Store;

impl Store {
    /// Insert identifiers extracted for a file within a transaction.
    pub fn insert_identifiers(&self, file_path: &str, items: &[IdentifierRecord]) -> Result<()> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare_cached(
                "INSERT INTO identifiers (identifier, file_path, line, role, kind)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        for item in items {
            let _ = stmt
                .execute(params![
                    item.identifier,
                    file_path,
                    item.line as i64,
                    item.role,
                    item.kind,
                ])
                .map_err(|e| Error::Database(e.to_string()))?;
        }
        Ok(())
    }

    /// Delete all identifiers recorded for a given file.
    pub fn delete_identifiers_for_file(&self, file_path: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM identifiers WHERE file_path = ?1", params![file_path])
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Find identifier records matching the given name and optional role filter.
    pub fn find_identifiers(
        &self,
        identifier: &str,
        role: Option<&str>,
        limit: usize,
    ) -> Result<Vec<IdentifierRecord>> {
        let conn = self.conn();
        let (sql, use_role) = match role {
            Some(_) => (
                "SELECT identifier, file_path, line, role, kind
                 FROM identifiers WHERE identifier = ?1 AND role = ?2 LIMIT ?3",
                true,
            ),
            None => (
                "SELECT identifier, file_path, line, role, kind
                 FROM identifiers WHERE identifier = ?1 LIMIT ?2",
                false,
            ),
        };

        let mut stmt = conn.prepare(sql).map_err(|e| Error::Database(e.to_string()))?;
        let mut results = Vec::new();
        if use_role {
            let mut rows = stmt
                .query(params![identifier, role.unwrap(), limit as i64])
                .map_err(|e| Error::Database(e.to_string()))?;
            while let Some(row) = rows.next().map_err(|e| Error::Database(e.to_string()))? {
                let line_i64: i64 = row.get(2).map_err(|e| Error::Database(e.to_string()))?;
                results.push(IdentifierRecord {
                    identifier: row.get(0).map_err(|e| Error::Database(e.to_string()))?,
                    file_path: row.get(1).map_err(|e| Error::Database(e.to_string()))?,
                    line: line_i64 as usize,
                    role: row.get(3).map_err(|e| Error::Database(e.to_string()))?,
                    kind: row.get(4).map_err(|e| Error::Database(e.to_string()))?,
                });
            }
        } else {
            let mut rows = stmt
                .query(params![identifier, limit as i64])
                .map_err(|e| Error::Database(e.to_string()))?;
            while let Some(row) = rows.next().map_err(|e| Error::Database(e.to_string()))? {
                let line_i64: i64 = row.get(2).map_err(|e| Error::Database(e.to_string()))?;
                results.push(IdentifierRecord {
                    identifier: row.get(0).map_err(|e| Error::Database(e.to_string()))?,
                    file_path: row.get(1).map_err(|e| Error::Database(e.to_string()))?,
                    line: line_i64 as usize,
                    role: row.get(3).map_err(|e| Error::Database(e.to_string()))?,
                    kind: row.get(4).map_err(|e| Error::Database(e.to_string()))?,
                });
            }
        }

        // If no explicit identifier records match, check code symbols defining this name
        if results.is_empty() {
            let sym_sql = "SELECT name, file_path, start_line FROM code_symbols WHERE name = ?1 LIMIT ?2";
            if let Ok(mut sym_stmt) = conn.prepare(sym_sql) {
                if let Ok(mut sym_rows) = sym_stmt.query(params![identifier, limit as i64]) {
                    while let Ok(Some(row)) = sym_rows.next() {
                        let line_i64: i64 = row.get(2).unwrap_or(1);
                        results.push(IdentifierRecord {
                            identifier: row.get(0).unwrap_or_default(),
                            file_path: row.get(1).unwrap_or_default(),
                            line: line_i64 as usize,
                            role: "defines".to_string(),
                            kind: Some("symbol".to_string()),
                        });
                    }
                }
            }
        }

        Ok(results)
    }
}
