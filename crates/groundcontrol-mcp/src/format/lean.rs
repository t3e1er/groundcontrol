//! Lean Multiline Text Emission Protocol
//!
//! Eliminates repetitive JSON schema overhead by formatting MCP tool responses as:
//! - Turn 1 (`search`): Partitioned Code/Doc hits with syntax-highlighted snippets,
//!   non-zero score breakdowns, and actionable Turn 2a/2b handles.
//! - Turn 2a (`get_snippet`): Bounded definition blocks with prefixed line numbers
//!   (`L<num>:`) and outbound graph scents.
//! - Turn 2b (`graph_match`): 2-space indented hierarchical Cypher-Lite ASCII trees
//!   with hub suppression and cycle detection (~69% token reduction).
//! - Turn 3 (`read_file`): Token-efficient markdown code blocks without JSON string
//!   escapes (`\n`, `\"`) for single and batch file reads.

use groundcontrol_common::types::{GraphMatchResult, GraphTreeNode, ScoreBreakdown, SearchResult};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// Detect a source language from a file extension. Returns `"text"` when unknown.
pub fn language_from_path(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "javascript",
        Some("py") => "python",
        Some("go") => "go",
        Some("java") => "java",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") | Some("hh") => "cpp",
        Some("md") | Some("markdown") => "markdown",
        Some("toml") => "toml",
        Some("json") => "json",
        Some("yaml") | Some("yml") => "yaml",
        Some("sql") => "sql",
        Some("sh") | Some("bash") => "bash",
        _ => "text",
    }
}

// ---------------------------------------------------------------------------
// Turn 1: Search Formatter
// ---------------------------------------------------------------------------

/// Format non-zero score components compactly, e.g. ` (bm25: 14.2, vec: 0.72)`.
fn format_score_components(components: Option<&ScoreBreakdown>) -> String {
    let Some(sc) = components else { return String::new() };
    let mut parts = Vec::new();
    if sc.bm25.abs() > 0.0001 {
        parts.push(format!("bm25: {:.2}", sc.bm25));
    }
    if sc.vector.abs() > 0.0001 {
        parts.push(format!("vec: {:.2}", sc.vector));
    }
    if sc.graph_boost.abs() > 0.0001 {
        parts.push(format!("graph: {:.2}", sc.graph_boost));
    }
    if let Some(hops) = sc.graph_hops {
        parts.push(format!("hops: {hops}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// Format Turn 1 search results in token-optimal lean multiline markdown.
pub fn format_lean_search(
    query: &str,
    mode: &str,
    corpus: Option<&str>,
    code_items: &[SearchResult],
    docs_items: &[SearchResult],
    is_ids_only: bool,
) -> String {
    let mut out = String::with_capacity(1024);
    let total = code_items.len() + docs_items.len();

    let corpus_tag = match corpus {
        Some(c) => format!(", corpus: {c}"),
        None => String::new(),
    };

    out.push_str(&format!("# Search: \"{query}\" [mode: {mode}, hits: {total}{corpus_tag}]\n\n"));

    if total == 0 {
        out.push_str("No matching results found.\n");
        return out;
    }

    if !code_items.is_empty() {
        out.push_str(&format!("## Code Hits ({})\n\n", code_items.len()));
        for (i, hit) in code_items.iter().enumerate() {
            let num = i + 1;
            let symbol_title = hit.symbol.as_deref().unwrap_or(&hit.path);
            let score_str = format!("{:.3}", hit.score);
            let comps = format_score_components(hit.score_components.as_ref());

            out.push_str(&format!("{num}. {symbol_title} (`{}`)", hit.path));
            out.push_str(&format!(" [score: {score_str}{comps}]\n"));

            if !is_ids_only {
                if let Some(ref snippet) = hit.snippet {
                    let lang = language_from_path(&hit.path);
                    out.push_str(&format!("```{lang}\n{snippet}\n```\n"));
                }

                // Turn 2a handle
                if let Some(chunk_idx) = hit.chunk_index {
                    out.push_str(&format!(
                        "-> [T2a fetch] get_snippet(path: \"{}\", chunk_index: {})\n",
                        hit.path, chunk_idx
                    ));
                } else if let Some(ref sym) = hit.symbol {
                    out.push_str(&format!("-> [T2a fetch] get_snippet(symbol: \"{sym}\")\n"));
                } else {
                    out.push_str(&format!(
                        "-> [T2a fetch] get_snippet(path: \"{}\", chunk_index: 0)\n",
                        hit.path
                    ));
                }

                // Turn 2b graph scent
                if let Some(ref graph_str) = hit.graph {
                    out.push_str(&format!("-> [T2b graph] {graph_str}\n"));
                } else if let Some(ref sym) = hit.symbol {
                    out.push_str(&format!(
                        "-> [T2b graph] graph_match(\"(:CodeSymbol {{name: \\\"{sym}\\\"}})<-[:calls]-(caller)\")\n"
                    ));
                }
            }
            out.push('\n');
        }
    }

    if !docs_items.is_empty() {
        out.push_str(&format!("## Doc Hits ({})\n\n", docs_items.len()));
        for (i, hit) in docs_items.iter().enumerate() {
            let num = i + 1;
            let score_str = format!("{:.3}", hit.score);
            let comps = format_score_components(hit.score_components.as_ref());

            out.push_str(&format!("{num}. `{}` [score: {score_str}{comps}]\n", hit.path));

            if !is_ids_only {
                if let Some(ref snippet) = hit.snippet {
                    out.push_str(&format!("```markdown\n{snippet}\n```\n"));
                }

                if let Some(chunk_idx) = hit.chunk_index {
                    out.push_str(&format!(
                        "-> [T2a fetch] get_snippet(path: \"{}\", chunk_index: {})\n",
                        hit.path, chunk_idx
                    ));
                } else {
                    out.push_str(&format!(
                        "-> [T2a fetch] get_snippet(path: \"{}\", chunk_index: 0)\n",
                        hit.path
                    ));
                }

                if let Some(ref graph_str) = hit.graph {
                    out.push_str(&format!("-> [T2b graph] {graph_str}\n"));
                } else {
                    out.push_str(&format!(
                        "-> [T2b graph] graph_match(\"(:DocNode {{path: \\\"{}\\\"}})-[:implements]->(code)\")\n",
                        hit.path
                    ));
                }
            }
            out.push('\n');
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Turn 2a: Get Snippet Formatter
// ---------------------------------------------------------------------------

/// Helper to prefix 1-based line numbers onto each line of code.
fn prefix_line_numbers(code: &str, start_line: usize) -> String {
    let mut out = String::with_capacity(code.len() + code.lines().count() * 8);
    for (i, line) in code.lines().enumerate() {
        let line_num = start_line + i;
        out.push_str(&format!("L{line_num}: {line}\n"));
    }
    out
}

/// Format Turn 2a code symbol snippet with line numbers and progressive disclosure hints.
pub fn format_lean_code_symbol(
    name: &str,
    scope_path: &str,
    file_path: &str,
    start_line: usize,
    end_line: usize,
    total_lines: usize,
    docstring: Option<&str>,
    source: &str,
    truncated: bool,
    incoming: &BTreeMap<String, Vec<Value>>,
    outgoing: &BTreeMap<String, Vec<Value>>,
) -> String {
    let mut out = String::with_capacity(source.len() + 512);
    let line_count = if end_line >= start_line { end_line - start_line + 1 } else { 0 };
    let lang = language_from_path(file_path);

    out.push_str(&format!(
        "# Symbol: {name} (`{file_path}:L{start_line}-L{end_line}`, {line_count} lines) [scope: {scope_path}, total_file_lines: {total_lines}]\n\n"
    ));

    if let Some(doc) = docstring {
        if !doc.trim().is_empty() {
            out.push_str("> **Docstring**:\n");
            for line in doc.lines() {
                out.push_str(&format!("> {line}\n"));
            }
            out.push('\n');
        }
    }

    out.push_str(&format!("```{lang}\n"));
    out.push_str(&prefix_line_numbers(source, start_line));
    out.push_str("```\n");

    if truncated {
        out.push_str("> [Note: Symbol body truncated at max_lines]\n\n");
    }

    // Render neighbor affordances if present
    if !incoming.is_empty() || !outgoing.is_empty() {
        out.push_str("### Relationships\n");
        if !incoming.is_empty() {
            out.push_str("Incoming:\n");
            for (rel, nodes) in incoming {
                for node in nodes {
                    let n_name = node.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let n_file = node.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
                    let n_line = node.get("start_line").and_then(|v| v.as_u64()).unwrap_or(0);
                    out.push_str(&format!("  <-[:{rel}]- {n_name} ({n_file}:L{n_line})\n"));
                }
            }
        }
        if !outgoing.is_empty() {
            out.push_str("Outgoing:\n");
            for (rel, nodes) in outgoing {
                for node in nodes {
                    let n_name = node.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let n_file = node.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
                    let n_line = node.get("start_line").and_then(|v| v.as_u64()).unwrap_or(0);
                    out.push_str(&format!("  -[:{rel}]-> {n_name} ({n_file}:L{n_line})\n"));
                }
            }
        }
        out.push('\n');
    }

    // Actionable next moves
    out.push_str(&format!(
        "-> [T2b callers] graph_match(\"(:CodeSymbol {{name: \\\"{name}\\\"}})<-[:calls]-(caller)\")\n"
    ));
    out.push_str(&format!(
        "-> [T2b callees] graph_match(\"(:CodeSymbol {{name: \\\"{name}\\\"}}-[:calls]->(callee)\")\n"
    ));
    out.push_str(&format!(
        "-> [T3 full file] read_file(path: \"{file_path}\", start_line: {start_line}, end_line: {end_line})\n"
    ));

    out
}

/// Format Turn 2a markdown doc chunk with line numbers and progressive disclosure hints.
pub fn format_lean_doc_chunk(
    file_path: &str,
    chunk_index: usize,
    start_line: usize,
    end_line: usize,
    total_lines: usize,
    chunk_text: &str,
    truncated: bool,
    incoming: &BTreeMap<String, Vec<Value>>,
    outgoing: &BTreeMap<String, Vec<Value>>,
) -> String {
    let mut out = String::with_capacity(chunk_text.len() + 512);
    let line_count = if end_line >= start_line { end_line - start_line + 1 } else { 0 };

    out.push_str(&format!(
        "# Doc Chunk: `{file_path}` [chunk: {chunk_index}, lines: L{start_line}-L{end_line} ({line_count} lines), total: {total_lines}]\n\n"
    ));

    out.push_str("```markdown\n");
    out.push_str(&prefix_line_numbers(chunk_text, start_line));
    out.push_str("```\n");

    if truncated {
        out.push_str("> [Note: Doc chunk truncated at max_lines]\n\n");
    }

    if !incoming.is_empty() || !outgoing.is_empty() {
        out.push_str("### Relationships\n");
        if !incoming.is_empty() {
            out.push_str("Incoming:\n");
            for (rel, nodes) in incoming {
                for node in nodes {
                    let n_path =
                        node.get("node_path").and_then(|v| v.as_str()).unwrap_or("unknown");
                    out.push_str(&format!("  <-[:{rel}]- `{n_path}`\n"));
                }
            }
        }
        if !outgoing.is_empty() {
            out.push_str("Outgoing:\n");
            for (rel, nodes) in outgoing {
                for node in nodes {
                    let n_path =
                        node.get("node_path").and_then(|v| v.as_str()).unwrap_or("unknown");
                    out.push_str(&format!("  -[:{rel}]-> `{n_path}`\n"));
                }
            }
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "-> [T2b lineage] graph_match(\"(:DocNode {{path: \\\"{file_path}\\\"}}-[:implements]->(target)\")\n"
    ));
    out.push_str(&format!("-> [T3 full note] read_file(path: \"{file_path}\")\n"));

    out
}

// ---------------------------------------------------------------------------
// Turn 2b: Graph Match Formatter
// ---------------------------------------------------------------------------

/// Format Turn 2b `graph_match` traversal results as an indented Cypher-Lite ASCII tree.
pub fn format_lean_graph_match(result: &GraphMatchResult) -> String {
    let mut out = String::with_capacity(512);

    let root = result.root.as_deref().unwrap_or("unknown");
    let file = result.file.as_deref().unwrap_or("unknown");

    out.push_str(&format!(
        "root: {root} ({file}) [direct: {}, transitive: {}, files: {}, depth: {}, matches: {}]\n",
        result.summary.direct,
        result.summary.transitive,
        result.summary.files,
        result.summary.max_depth,
        result.total_matches
    ));

    for node in &result.tree {
        format_graph_branch(node, 0, &mut out);
    }

    out.push_str(&format!("\n-> [T2a fetch] get_snippet(symbol: \"{root}\")\n"));
    out
}

fn format_graph_branch(node: &GraphTreeNode, indent_level: usize, out: &mut String) {
    let indent = "  ".repeat(indent_level + 1);
    let rel_str = node.rel.as_deref().unwrap_or("calls");
    let arrow = match rel_str {
        "calls" => "<-[:calls]- ".to_string(),
        "implements" | "implements_trait" => "-[:implements]-> ".to_string(),
        "defines" => "-[:defines]-> ".to_string(),
        "imports" => "-[:imports]-> ".to_string(),
        "wikilink" => "--[:wikilink]-- ".to_string(),
        other => format!("-[:{other}]-> "),
    };

    let loc = match (node.file.as_deref(), node.line) {
        (Some(f), Some(l)) => format!(" ({f}:L{l})"),
        (Some(f), None) => format!(" ({f})"),
        _ => String::new(),
    };

    let hub_str = match (node.hub, node.suppressed) {
        (Some(true), Some(count)) => format!(" [hub: +{count} more]"),
        _ => String::new(),
    };

    out.push_str(&format!("{indent}{arrow}{}{loc}{hub_str}\n", node.node));

    for branch in &node.branches {
        format_graph_branch(branch, indent_level + 1, out);
    }
}

// ---------------------------------------------------------------------------
// Turn 3: Read File Formatter
// ---------------------------------------------------------------------------

/// Format Turn 3 single file read as clean markdown code block with line numbers.
pub fn format_lean_read_file(
    path: &str,
    start_line: usize,
    end_line: usize,
    total_lines: usize,
    content: &str,
    language: &str,
    is_markdown_note: bool,
    truncated: bool,
) -> String {
    let mut out = String::with_capacity(content.len() + 256);

    if is_markdown_note && start_line <= 1 && end_line >= total_lines {
        out.push_str(&format!("# File: `{path}` [markdown_note, total_lines: {total_lines}]\n\n"));
        out.push_str(content);
        out.push('\n');
    } else {
        out.push_str(&format!(
            "# File: `{path}` [lines: L{start_line}-L{end_line} of {total_lines}, language: {language}]\n\n"
        ));
        out.push_str(&format!("```{language}\n"));
        out.push_str(&prefix_line_numbers(content, start_line));
        out.push_str("```\n");
    }

    if truncated {
        out.push_str("> [Note: File read truncated at max_lines ceiling]\n");
    }

    out
}

/// Format Turn 3 batch multiple file read as sequential markdown sections.
pub fn format_lean_read_multiple(results: &[(String, Result<Value, String>)]) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str(&format!("# Batch Read ({} files)\n\n", results.len()));

    for (i, (path, res)) in results.iter().enumerate() {
        let num = i + 1;
        out.push_str(&format!("---\n## {num}. `{path}`\n\n"));
        match res {
            Ok(val) => {
                let content = val.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let start_line =
                    val.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let end_line = val.get("end_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let total_lines =
                    val.get("total_lines").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let lang = val.get("language").and_then(|v| v.as_str()).unwrap_or("text");
                let truncated = val.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);

                out.push_str(&format!("```{lang}\n"));
                out.push_str(&prefix_line_numbers(content, start_line));
                out.push_str("```\n");
                if truncated {
                    out.push_str("> [Note: File truncated at max_lines]\n");
                }
                out.push_str(&format!("[L{start_line}-L{end_line} of {total_lines}]\n"));
            }
            Err(e) => {
                out.push_str(&format!("> **Error**: {e}\n"));
            }
        }
        out.push('\n');
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use groundcontrol_common::types::{GraphImpactSummary, ScoreBreakdown};

    #[test]
    fn test_format_lean_search_renders_correctly() {
        let mut hit1 = SearchResult::new("crates/core/src/engine.rs", 0.95);
        hit1.symbol = Some("SearchEngine".to_string());
        hit1.chunk_index = Some(2);
        hit1.score_components =
            Some(ScoreBreakdown { bm25: 15.2, vector: 0.85, graph_boost: 0.1, graph_hops: None });
        hit1.snippet = Some("pub struct SearchEngine {\n    config: Config,\n}".to_string());

        let out = format_lean_search("engine", "hybrid", Some("ctxvault"), &[hit1], &[], false);

        assert!(out.contains("# Search: \"engine\" [mode: hybrid, hits: 1, corpus: ctxvault]"));
        assert!(out.contains("1. SearchEngine (`crates/core/src/engine.rs`) [score: 0.950 (bm25: 15.20, vec: 0.85, graph: 0.10)]"));
        assert!(out.contains("```rust\npub struct SearchEngine"));
        assert!(out.contains(
            "-> [T2a fetch] get_snippet(path: \"crates/core/src/engine.rs\", chunk_index: 2)"
        ));
        assert!(out.contains("-> [T2b graph] graph_match"));
    }

    #[test]
    fn test_format_lean_graph_match_tree() {
        let mut result = GraphMatchResult::default();
        result.root = Some("detect_bundle".to_string());
        result.file = Some("crates/core/src/bundle.rs:L216".to_string());
        result.total_matches = 2;
        result.summary = GraphImpactSummary { direct: 1, transitive: 1, files: 2, max_depth: 2 };

        let child = GraphTreeNode {
            node: "prompt_bundle_extraction".to_string(),
            rel: Some("calls".to_string()),
            file: Some("crates/cli/src/main.rs".to_string()),
            line: Some(279),
            hop: 1,
            branches: vec![GraphTreeNode {
                node: "main".to_string(),
                rel: Some("calls".to_string()),
                file: Some("crates/cli/src/main.rs".to_string()),
                line: Some(316),
                hop: 2,
                branches: vec![],
                suppressed: None,
                hub: None,
            }],
            suppressed: None,
            hub: None,
        };
        result.tree = vec![child];

        let out = format_lean_graph_match(&result);
        assert!(out.contains("root: detect_bundle (crates/core/src/bundle.rs:L216) [direct: 1, transitive: 1, files: 2, depth: 2, matches: 2]"));
        assert!(
            out.contains("  <-[:calls]- prompt_bundle_extraction (crates/cli/src/main.rs:L279)")
        );
        assert!(out.contains("    <-[:calls]- main (crates/cli/src/main.rs:L316)"));
        assert!(out.contains("-> [T2a fetch] get_snippet(symbol: \"detect_bundle\")"));
    }

    #[test]
    fn test_format_lean_read_file_line_numbers() {
        let code = "fn hello() {\n    println!(\"hello\");\n}";
        let out = format_lean_read_file("src/main.rs", 10, 12, 50, code, "rust", false, false);

        assert!(out.contains("# File: `src/main.rs` [lines: L10-L12 of 50, language: rust]"));
        assert!(
            out.contains("```rust\nL10: fn hello() {\nL11:     println!(\"hello\");\nL12: }\n```")
        );
    }
}
