//! Code symbols and external references persistence methods.

use std::collections::HashMap;

use rusqlite::params;

use ctxvault_common::types::{
    CodeSymbol, CodeSymbolType, ExternalRef, ExternalRefKind, ResolutionConfidence,
};
use ctxvault_common::{Error, Result};

use super::Store;

impl Store {
    /// Save code symbols extracted from a file. Replaces any existing symbols for the file.
    pub fn save_code_symbols(&self, file_path: &str, symbols: &[CodeSymbol]) -> Result<()> {
        let conn = self.conn();
        if conn.is_autocommit() {
            let tx = conn.unchecked_transaction().map_err(|e| Error::Database(e.to_string()))?;

            // Delete existing symbols for this file
            tx.execute("DELETE FROM code_symbols WHERE file_path = ?1", params![file_path])
                .map_err(|e| Error::Database(e.to_string()))?;

            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO code_symbols (file_path, name, scope_path, symbol_type, language, signature, docstring, start_line, end_line)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    )
                    .map_err(|e| Error::Database(e.to_string()))?;

                for sym in symbols {
                    let type_str = serde_json::to_string(&sym.symbol_type)
                        .unwrap_or_default()
                        .trim_matches('"')
                        .to_string();
                    stmt.execute(params![
                        sym.file_path,
                        sym.name,
                        sym.scope_path,
                        type_str,
                        sym.language,
                        sym.signature,
                        sym.docstring,
                        sym.start_line as i64,
                        sym.end_line as i64,
                    ])
                    .map_err(|e| Error::Database(e.to_string()))?;
                }
            }

            tx.commit().map_err(|e| Error::Database(e.to_string()))?;
        } else {
            conn.execute("DELETE FROM code_symbols WHERE file_path = ?1", params![file_path])
                .map_err(|e| Error::Database(e.to_string()))?;

            let mut stmt = conn
                .prepare(
                    "INSERT INTO code_symbols (file_path, name, scope_path, symbol_type, language, signature, docstring, start_line, end_line)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )
                .map_err(|e| Error::Database(e.to_string()))?;

            for sym in symbols {
                let type_str = serde_json::to_string(&sym.symbol_type)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string();
                stmt.execute(params![
                    sym.file_path,
                    sym.name,
                    sym.scope_path,
                    type_str,
                    sym.language,
                    sym.signature,
                    sym.docstring,
                    sym.start_line as i64,
                    sym.end_line as i64,
                ])
                .map_err(|e| Error::Database(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Retrieve all code symbols defined in a given file.
    pub fn get_code_symbols_for_file(&self, file_path: &str) -> Result<Vec<CodeSymbol>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT file_path, name, scope_path, symbol_type, language, signature, docstring, start_line, end_line
                 FROM code_symbols WHERE file_path = ?1 ORDER BY start_line",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![file_path], |row| {
                let type_str: String = row.get(3)?;
                let symbol_type: CodeSymbolType = serde_json::from_str(&format!("\"{type_str}\""))
                    .unwrap_or(CodeSymbolType::Function);
                Ok(CodeSymbol {
                    file_path: row.get(0)?,
                    name: row.get(1)?,
                    scope_path: row.get(2)?,
                    symbol_type,
                    language: row.get(4)?,
                    signature: row.get(5)?,
                    docstring: row.get(6)?,
                    start_line: row.get::<_, i64>(7)? as usize,
                    end_line: row.get::<_, i64>(8)? as usize,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(|e| Error::Database(e.to_string()))
    }

    /// Retrieve all code symbols defined across a batch of files in a single batch query.
    pub fn get_code_symbols_for_files(
        &self,
        file_paths: &[&str],
    ) -> Result<HashMap<String, Vec<CodeSymbol>>> {
        let mut map: HashMap<String, Vec<CodeSymbol>> = HashMap::with_capacity(file_paths.len());
        if file_paths.is_empty() {
            return Ok(map);
        }

        let conn = self.conn();
        for chunk in file_paths.chunks(500) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT file_path, name, scope_path, symbol_type, language, signature, docstring, start_line, end_line
                 FROM code_symbols WHERE file_path IN ({placeholders}) ORDER BY file_path, start_line"
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| Error::Database(e.to_string()))?;
            let params_vec: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|p| p as &dyn rusqlite::ToSql).collect();

            let rows = stmt
                .query_map(params_vec.as_slice(), |row| {
                    let type_str: String = row.get(3)?;
                    let symbol_type: CodeSymbolType =
                        serde_json::from_str(&format!("\"{type_str}\""))
                            .unwrap_or(CodeSymbolType::Function);
                    Ok(CodeSymbol {
                        file_path: row.get(0)?,
                        name: row.get(1)?,
                        scope_path: row.get(2)?,
                        symbol_type,
                        language: row.get(4)?,
                        signature: row.get(5)?,
                        docstring: row.get(6)?,
                        start_line: row.get::<_, i64>(7)? as usize,
                        end_line: row.get::<_, i64>(8)? as usize,
                    })
                })
                .map_err(|e| Error::Database(e.to_string()))?;

            for sym in rows {
                let s = sym.map_err(|e| Error::Database(e.to_string()))?;
                map.entry(s.file_path.clone()).or_default().push(s);
            }
        }

        Ok(map)
    }

    /// Find code symbols matching a name pattern.
    pub fn find_symbols_by_name(&self, name_pattern: &str) -> Result<Vec<CodeSymbol>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT file_path, name, scope_path, symbol_type, language, signature, docstring, start_line, end_line
                 FROM code_symbols WHERE name LIKE ?1 OR scope_path LIKE ?1 ORDER BY name",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let search_pattern = format!("%{name_pattern}%");
        let rows = stmt
            .query_map(params![search_pattern], |row| {
                let type_str: String = row.get(3)?;
                let symbol_type: CodeSymbolType = serde_json::from_str(&format!("\"{type_str}\""))
                    .unwrap_or(CodeSymbolType::Function);
                Ok(CodeSymbol {
                    file_path: row.get(0)?,
                    name: row.get(1)?,
                    scope_path: row.get(2)?,
                    symbol_type,
                    language: row.get(4)?,
                    signature: row.get(5)?,
                    docstring: row.get(6)?,
                    start_line: row.get::<_, i64>(7)? as usize,
                    end_line: row.get::<_, i64>(8)? as usize,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(|e| Error::Database(e.to_string()))
    }

    /// Find code symbols whose fully qualified `scope_path` matches exactly.
    ///
    /// If no exact matches are found, falls back to normalized scope path resolution
    /// (ignoring generic type/lifetime parameters).
    pub fn find_symbols_by_qualified_name(&self, scope_path: &str) -> Result<Vec<CodeSymbol>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT file_path, name, scope_path, symbol_type, language, signature, docstring, start_line, end_line
                 FROM code_symbols WHERE scope_path = ?1 ORDER BY file_path, start_line",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![scope_path], |row| {
                let type_str: String = row.get(3)?;
                let symbol_type: CodeSymbolType = serde_json::from_str(&format!("\"{type_str}\""))
                    .unwrap_or(CodeSymbolType::Function);
                Ok(CodeSymbol {
                    file_path: row.get(0)?,
                    name: row.get(1)?,
                    scope_path: row.get(2)?,
                    symbol_type,
                    language: row.get(4)?,
                    signature: row.get(5)?,
                    docstring: row.get(6)?,
                    start_line: row.get::<_, i64>(7)? as usize,
                    end_line: row.get::<_, i64>(8)? as usize,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        let exact_matches: Vec<CodeSymbol> = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(e.to_string()))?;

        drop(stmt);
        drop(conn);

        if !exact_matches.is_empty() {
            return Ok(exact_matches);
        }

        self.find_symbols_by_normalized_scope(scope_path)
    }

    /// Look up code symbols matching the given scope path ignoring generic type/lifetime parameters.
    pub fn find_symbols_by_normalized_scope(&self, scope_path: &str) -> Result<Vec<CodeSymbol>> {
        let norm_query = crate::parser::code::normalize_scope_path(scope_path);
        let leaf = norm_query.split(" > ").last().unwrap_or(&norm_query).trim();

        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT file_path, name, scope_path, symbol_type, language, signature, docstring, start_line, end_line
                 FROM code_symbols WHERE name = ?1 ORDER BY file_path, start_line",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![leaf], |row| {
                let type_str: String = row.get(3)?;
                let symbol_type: CodeSymbolType = serde_json::from_str(&format!("\"{type_str}\""))
                    .unwrap_or(CodeSymbolType::Function);
                Ok(CodeSymbol {
                    file_path: row.get(0)?,
                    name: row.get(1)?,
                    scope_path: row.get(2)?,
                    symbol_type,
                    language: row.get(4)?,
                    signature: row.get(5)?,
                    docstring: row.get(6)?,
                    start_line: row.get::<_, i64>(7)? as usize,
                    end_line: row.get::<_, i64>(8)? as usize,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        let candidates: Vec<CodeSymbol> = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(e.to_string()))?;

        let matched: Vec<CodeSymbol> = candidates
            .into_iter()
            .filter(|sym| {
                let norm_candidate = crate::parser::code::normalize_scope_path(&sym.scope_path);
                crate::parser::code::scope_matches(&norm_candidate, &norm_query)
            })
            .collect();

        Ok(matched)
    }

    /// Retrieve all code symbols in the entire store.
    pub fn get_all_code_symbols(&self) -> Result<Vec<CodeSymbol>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT file_path, name, scope_path, symbol_type, language, signature, docstring, start_line, end_line
                 FROM code_symbols ORDER BY file_path, start_line",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let type_str: String = row.get(3)?;
                let symbol_type: CodeSymbolType = serde_json::from_str(&format!("\"{type_str}\""))
                    .unwrap_or(CodeSymbolType::Function);
                Ok(CodeSymbol {
                    file_path: row.get(0)?,
                    name: row.get(1)?,
                    scope_path: row.get(2)?,
                    symbol_type,
                    language: row.get(4)?,
                    signature: row.get(5)?,
                    docstring: row.get(6)?,
                    start_line: row.get::<_, i64>(7)? as usize,
                    end_line: row.get::<_, i64>(8)? as usize,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(|e| Error::Database(e.to_string()))
    }

    /// Delete all external references captured for a file.
    pub fn clear_external_refs_for_file(&self, file_path: &str) -> Result<()> {
        let _ = self
            .conn()
            .execute("DELETE FROM external_refs WHERE file_path = ?1", params![file_path])
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Insert a batch of external references for a file within a single transaction.
    pub fn insert_external_refs(&self, file_path: &str, refs: &[ExternalRef]) -> Result<()> {
        let conn = self.conn();
        if conn.is_autocommit() {
            let tx = conn.unchecked_transaction().map_err(|e| Error::Database(e.to_string()))?;
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO external_refs (file_path, caller_scope_path, raw_target, kind, confidence)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                    )
                    .map_err(|e| Error::Database(e.to_string()))?;

                for r in refs {
                    stmt.execute(params![
                        file_path,
                        r.caller_scope_path,
                        r.raw_target,
                        external_ref_kind_to_str(r.kind),
                        resolution_confidence_to_str(r.confidence),
                    ])
                    .map_err(|e| Error::Database(e.to_string()))?;
                }
            }
            tx.commit().map_err(|e| Error::Database(e.to_string()))?;
        } else {
            let mut stmt = conn
                .prepare(
                    "INSERT INTO external_refs (file_path, caller_scope_path, raw_target, kind, confidence)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .map_err(|e| Error::Database(e.to_string()))?;

            for r in refs {
                stmt.execute(params![
                    file_path,
                    r.caller_scope_path,
                    r.raw_target,
                    external_ref_kind_to_str(r.kind),
                    resolution_confidence_to_str(r.confidence),
                ])
                .map_err(|e| Error::Database(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Retrieve every external reference in the store.
    pub fn get_external_refs(&self) -> Result<Vec<ExternalRef>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT caller_scope_path, raw_target, kind, confidence FROM external_refs ORDER BY id",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], external_ref_from_row)
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(|e| Error::Database(e.to_string()))
    }

    /// Retrieve the external references captured for a single file.
    pub fn get_external_refs_for_file(&self, file_path: &str) -> Result<Vec<ExternalRef>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT caller_scope_path, raw_target, kind, confidence
                 FROM external_refs WHERE file_path = ?1 ORDER BY id",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![file_path], external_ref_from_row)
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(|e| Error::Database(e.to_string()))
    }
}

/// Persisted string tag for an [`ExternalRefKind`].
pub fn external_ref_kind_to_str(kind: ExternalRefKind) -> &'static str {
    match kind {
        ExternalRefKind::Call => "call",
        ExternalRefKind::Import => "import",
    }
}

/// Parse a persisted external-reference kind tag, defaulting to `Call`.
pub fn external_ref_kind_from_str(s: &str) -> ExternalRefKind {
    match s {
        "import" => ExternalRefKind::Import,
        _ => ExternalRefKind::Call,
    }
}

/// Persisted string tag for a [`ResolutionConfidence`].
pub fn resolution_confidence_to_str(confidence: ResolutionConfidence) -> &'static str {
    match confidence {
        ResolutionConfidence::High => "high",
        ResolutionConfidence::Medium => "medium",
        ResolutionConfidence::Speculative => "speculative",
    }
}

/// Parse a persisted resolution-confidence tag, defaulting to `Speculative`.
pub fn resolution_confidence_from_str(s: &str) -> ResolutionConfidence {
    match s {
        "high" => ResolutionConfidence::High,
        "medium" => ResolutionConfidence::Medium,
        _ => ResolutionConfidence::Speculative,
    }
}

/// Map an `external_refs` row (`caller_scope_path, raw_target, kind, confidence`)
/// to a domain [`ExternalRef`].
pub fn external_ref_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExternalRef> {
    let kind_str: String = row.get(2)?;
    let conf_str: String = row.get(3)?;
    Ok(ExternalRef {
        caller_scope_path: row.get(0)?,
        raw_target: row.get(1)?,
        kind: external_ref_kind_from_str(&kind_str),
        confidence: resolution_confidence_from_str(&conf_str),
    })
}
