//! Cross-corpus identifier lookup tool (`where`).

use serde::Deserialize;
use serde_json::Value;

use groundcontrol_common::{Error, Result};
use groundcontrol_core::corpus_manager::CorpusManager;
use groundcontrol_core::engine::Engine;

#[derive(Debug, Deserialize)]
pub(crate) struct WhereParams {
    pub identifier: String,
    pub corpora: Option<Vec<String>>,
    pub role: Option<String>,
    pub max_results: Option<usize>,
    pub format: Option<String>,
}

/// Handle `where` tool invocation across mounted corpora or single engine.
pub fn handle_where_corpus_manager(manager: &CorpusManager, args: Value) -> Result<Value> {
    let params: WhereParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params for where: {}", e)))?;

    let max_results = params.max_results.unwrap_or(50).max(1);
    let results = manager.where_identifier(
        &params.identifier,
        params.corpora.as_deref(),
        params.role.as_deref(),
        max_results,
    )?;

    if params.format.as_deref() == Some("lean") {
        Ok(Value::String(format_lean_where(&params.identifier, &results)))
    } else {
        Ok(serde_json::json!({
            "identifier": params.identifier,
            "total_matches": results.len(),
            "results": results,
        }))
    }
}

/// Fallback handle for single-engine execution.
pub fn handle_where_engine(engine: &Engine, args: Value) -> Result<Value> {
    let params: WhereParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params for where: {}", e)))?;

    let max_results = params.max_results.unwrap_or(50).max(1);
    let records = engine.find_identifiers(&params.identifier, params.role.as_deref(), max_results)?;
    let results: Vec<Value> = records
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "corpus": engine.config().name,
                "identifier": r.identifier,
                "file_path": r.file_path,
                "line": r.line,
                "role": r.role,
                "kind": r.kind,
            })
        })
        .collect();

    if params.format.as_deref() == Some("lean") {
        Ok(Value::String(format_lean_where(&params.identifier, &results)))
    } else {
        Ok(serde_json::json!({
            "identifier": params.identifier,
            "total_matches": results.len(),
            "results": results,
        }))
    }
}

fn format_lean_where(identifier: &str, results: &[Value]) -> String {
    let mut out = String::with_capacity(results.len() * 80 + 128);
    out.push_str(&format!("# Where: \"{identifier}\" [matches: {}]\n\n", results.len()));

    if results.is_empty() {
        out.push_str("No occurrences found across corpora.\n");
        return out;
    }

    for (i, r) in results.iter().enumerate() {
        let num = i + 1;
        let corpus = r["corpus"].as_str().unwrap_or("");
        let file = r["file_path"].as_str().unwrap_or("");
        let line = r["line"].as_u64().unwrap_or(1);
        let role = r["role"].as_str().unwrap_or("mentions");
        let kind_str = match r["kind"].as_str() {
            Some(k) => format!(" ({k})"),
            None => String::new(),
        };

        out.push_str(&format!(
            "{num}. [{corpus}] `{file}:L{line}` [role: {role}{kind_str}]\n"
        ));
    }

    out
}
