//! Fast, line-addressed disk-verified pattern matching tool (`grep`).

use std::fs;
use std::path::Path;

use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use groundcontrol_common::ports::MetadataCatalog;
use groundcontrol_common::{Error, Result};
use groundcontrol_core::engine::Engine;

#[derive(Debug, Deserialize)]
pub(crate) struct GrepParams {
    /// Search pattern (regex by default, or literal string if is_literal is true).
    pub pattern: String,
    /// Optional relative file path or directory prefix to restrict the search.
    pub path: Option<String>,
    /// Match case-sensitively (default false).
    #[serde(default)]
    pub case_sensitive: bool,
    /// Treat pattern as a literal string instead of regex (default false).
    #[serde(default)]
    pub is_literal: bool,
    /// Maximum matching lines to return (default 100, hard cap 500).
    pub max_matches: Option<usize>,
    /// Number of context lines before and after each match (default 0, capped at 5).
    pub context_lines: Option<usize>,
    /// Output format: "lean" (default) or "json".
    pub format: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub(crate) struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub context_before: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub context_after: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct GrepResponse {
    pub pattern: String,
    pub total_matches: usize,
    pub matches: Vec<GrepMatch>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub truncated: bool,
}

/// Execute a grep search across the corpus on disk.
pub fn handle_grep(engine: &Engine, args: Value) -> Result<Value> {
    let params: GrepParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    if params.pattern.is_empty() {
        return Err(Error::Config("pattern cannot be empty".to_string()));
    }

    let max_matches = params.max_matches.unwrap_or(100).clamp(1, 500);
    let context_lines = params.context_lines.unwrap_or(0).min(5);
    let is_lean = params.format.as_deref() == Some("lean") || params.format.is_none();

    let regex_pattern =
        if params.is_literal { regex::escape(&params.pattern) } else { params.pattern.clone() };

    let regex = RegexBuilder::new(&regex_pattern)
        .case_insensitive(!params.case_sensitive)
        .build()
        .map_err(|e| Error::Config(format!("invalid regex pattern '{}': {e}", params.pattern)))?;

    let corpus_root = Path::new(&engine.config().path);
    let all_files = engine.store().list_files()?;

    let normalized_filter = params.path.as_deref().map(|p| p.replace('\\', "/"));

    let mut matches = Vec::new();
    let mut truncated = false;

    for file in &all_files {
        let norm_file_path = file.path.replace('\\', "/");
        if let Some(ref filter) = normalized_filter {
            if !norm_file_path.starts_with(filter) && !norm_file_path.contains(filter) {
                continue;
            }
        }

        let full_path = corpus_root.join(&file.path);
        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue, // Skip unreadable or binary files
        };

        let lines: Vec<&str> = content.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            if regex.is_match(line) {
                let line_number = idx + 1;
                let context_before = if context_lines > 0 {
                    let start = idx.saturating_sub(context_lines);
                    lines[start..idx].iter().map(|s| s.to_string()).collect()
                } else {
                    Vec::new()
                };

                let context_after = if context_lines > 0 {
                    let end = (idx + 1 + context_lines).min(lines.len());
                    lines[(idx + 1)..end].iter().map(|s| s.to_string()).collect()
                } else {
                    Vec::new()
                };

                matches.push(GrepMatch {
                    path: file.path.clone(),
                    line_number,
                    line: line.to_string(),
                    context_before,
                    context_after,
                });

                if matches.len() >= max_matches {
                    truncated = true;
                    break;
                }
            }
        }

        if truncated {
            break;
        }
    }

    if is_lean {
        let lean_output = format_lean_grep(&params.pattern, &matches, truncated, context_lines > 0);
        Ok(Value::String(lean_output))
    } else {
        let resp = GrepResponse {
            pattern: params.pattern,
            total_matches: matches.len(),
            matches,
            truncated,
        };
        serde_json::to_value(resp).map_err(|e| Error::Config(format!("serialize error: {}", e)))
    }
}

/// Format grep matches into concise, standard ripgrep-style line format.
fn format_lean_grep(
    pattern: &str,
    matches: &[GrepMatch],
    truncated: bool,
    has_context: bool,
) -> String {
    let mut out = String::with_capacity(matches.len() * 80 + 128);
    let trunc_str = if truncated { " (truncated at limit)" } else { "" };
    out.push_str(&format!("# Grep: \"{pattern}\" [matches: {}{trunc_str}]\n\n", matches.len()));

    if matches.is_empty() {
        out.push_str("No matching lines found.\n");
        return out;
    }

    for m in matches {
        if has_context {
            let start_line = m.line_number.saturating_sub(m.context_before.len());
            for (offset, ctx_line) in m.context_before.iter().enumerate() {
                out.push_str(&format!("{}-{}-{}\n", m.path, start_line + offset, ctx_line));
            }
            out.push_str(&format!("{}:{}:{}\n", m.path, m.line_number, m.line));
            for (offset, ctx_line) in m.context_after.iter().enumerate() {
                out.push_str(&format!("{}-{}-{}\n", m.path, m.line_number + 1 + offset, ctx_line));
            }
            out.push_str("--\n");
        } else {
            out.push_str(&format!("{}:{}:{}\n", m.path, m.line_number, m.line));
        }
    }

    out
}
