//! Read tools: `read_file`, `get_snippet`, `list_notes`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use groundcontrol_common::ports::{GraphStore, MetadataCatalog};
use groundcontrol_common::{Error, Result};
use groundcontrol_core::engine::Engine;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum PathOrPaths {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReadFileParams {
    pub path: Option<PathOrPaths>,
    pub paths: Option<Vec<String>>,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub max_lines: Option<usize>,
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListNotesParams {
    pub path: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GetSnippetParams {
    pub name: Option<String>,
    pub path: Option<String>,
    pub chunk_index: Option<usize>,
    pub qualified_name: Option<String>,
    pub max_lines: Option<usize>,
    #[serde(default)]
    pub include_neighbors: bool,
    pub format: Option<String>,
}

#[derive(Serialize)]
struct NoteListItem {
    path: String,
    title: Option<String>,
    template: Option<String>,
    content_hash: String,
}

/// Detect a source language from a file extension. Returns `"text"` when unknown.
pub(crate) fn language_from_path(path: &str) -> &'static str {
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
        _ => "text",
    }
}

/// Bound a body of source lines to `max_lines`, joining with newlines and
/// reporting whether truncation occurred.
pub(crate) fn cap_lines(lines: &[&str], max_lines: usize) -> (String, bool) {
    if lines.len() > max_lines {
        (lines[..max_lines].join("\n"), true)
    } else {
        (lines.join("\n"), false)
    }
}

/// Build a bare handle (no body) for a code symbol: scope_path + file + line range + signature/docstring.
pub(crate) fn code_symbol_handle(sym: &groundcontrol_common::types::CodeSymbol) -> Value {
    serde_json::json!({
        "scope_path": sym.scope_path,
        "name": sym.name,
        "file_path": sym.file_path,
        "start_line": sym.start_line,
        "end_line": sym.end_line,
        "language": sym.language,
        "symbol_type": sym.symbol_type,
        "signature": sym.signature,
        "docstring": sym.docstring,
    })
}

pub(crate) fn read_file_lossy(path: &Path) -> std::io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Read a single file for [`handle_read_file`].
fn read_single_file(
    engine: &Engine,
    path: &str,
    start_line: Option<usize>,
    end_line: Option<usize>,
    max_lines: usize,
) -> Result<Value> {
    let proj_path = engine.projection_path(path);
    let is_projected = proj_path.is_file();
    let raw = if is_projected {
        read_file_lossy(&proj_path)
            .map_err(|e| Error::NotFound(format!("cannot read projection for {}: {}", path, e)))?
    } else {
        let corpus_root = Path::new(&engine.config().path);
        let full_path = corpus_root.join(path);
        read_file_lossy(&full_path)
            .map_err(|e| Error::NotFound(format!("cannot read {}: {}", path, e)))?
    };

    if is_projected {
        let file_lines: Vec<&str> = raw.lines().collect();
        let total_lines = file_lines.len();
        let start = start_line.unwrap_or(1).max(1);
        let end = end_line.unwrap_or(total_lines).min(total_lines);

        let (content, truncated) = if start > total_lines {
            (String::new(), false)
        } else {
            let slice_start = start - 1;
            let slice_end = end.max(slice_start);
            let slice = &file_lines[slice_start..slice_end];
            cap_lines(slice, max_lines)
        };

        return Ok(serde_json::json!({
            "kind": "projected_doc",
            "path": path,
            "start_line": start,
            "end_line": end,
            "total_lines": total_lines,
            "language": "markdown",
            "content": content,
            "truncated": truncated,
        }));
    }

    let is_markdown = matches!(language_from_path(path), "markdown");
    if is_markdown && start_line.is_none() && end_line.is_none() {
        let doc = groundcontrol_core::parser::parse_document(Path::new(path), &raw)?;
        let lines: Vec<&str> = doc.content.lines().collect();
        let (content, truncated) = cap_lines(&lines, max_lines);
        return Ok(serde_json::json!({
            "kind": "markdown_note",
            "path": path,
            "title": doc.title,
            "frontmatter": doc.frontmatter,
            "content": content,
            "truncated": truncated,
            "content_hash": doc.content_hash,
        }));
    }

    let file_lines: Vec<&str> = raw.lines().collect();
    let total_lines = file_lines.len();
    let start = start_line.unwrap_or(1).max(1);
    let end = end_line.unwrap_or(total_lines).min(total_lines);

    if start > total_lines {
        return Ok(serde_json::json!({
            "kind": if is_markdown { "markdown_note" } else { "code_file" },
            "path": path,
            "start_line": start,
            "end_line": end,
            "total_lines": total_lines,
            "content": "",
            "truncated": false,
        }));
    }

    let slice_start = start - 1;
    let slice_end = end.max(slice_start);
    let slice = &file_lines[slice_start..slice_end];
    let (content, truncated) = cap_lines(slice, max_lines);

    Ok(serde_json::json!({
        "kind": if is_markdown { "markdown_note" } else { "code_file" },
        "path": path,
        "start_line": start,
        "end_line": end,
        "total_lines": total_lines,
        "language": language_from_path(path),
        "content": content,
        "truncated": truncated,
    }))
}

/// Tier 3 read of one or more files (markdown or source code).
pub fn handle_read_file(engine: &Engine, args: Value) -> Result<Value> {
    let params: ReadFileParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let target_paths = if let Some(paths) = params.paths {
        PathOrPaths::Multiple(paths)
    } else if let Some(p) = params.path {
        p
    } else {
        return Err(Error::Config("read_file requires 'path' or 'paths'".to_string()));
    };

    let is_lean = params.format.as_deref() == Some("lean");

    match target_paths {
        PathOrPaths::Single(p) => {
            let max_lines = params.max_lines.unwrap_or(1000).max(1);
            let val = read_single_file(engine, &p, params.start_line, params.end_line, max_lines)?;
            if is_lean {
                let content = val.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let start_line =
                    val.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let end_line = val.get("end_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let total_lines =
                    val.get("total_lines").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let language = val.get("language").and_then(|v| v.as_str()).unwrap_or("text");
                let kind = val.get("kind").and_then(|v| v.as_str());
                let is_markdown = kind == Some("markdown_note") || kind == Some("projected_doc");
                let truncated = val.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);

                let lean = crate::format::lean::format_lean_read_file(
                    &p,
                    start_line,
                    end_line,
                    total_lines,
                    content,
                    language,
                    is_markdown,
                    truncated,
                );
                Ok(Value::String(lean))
            } else {
                Ok(val)
            }
        }
        PathOrPaths::Multiple(paths) => {
            let max_lines = params.max_lines.unwrap_or(500).max(1);
            let results: Vec<(String, std::result::Result<Value, String>)> = paths
                .iter()
                .map(|p| {
                    let res = read_single_file(engine, p, None, None, max_lines)
                        .map_err(|e| e.to_string());
                    (p.clone(), res)
                })
                .collect();

            if is_lean {
                let lean = crate::format::lean::format_lean_read_multiple(&results);
                Ok(Value::String(lean))
            } else {
                let json_results: Vec<Value> = results
                    .into_iter()
                    .map(|(p, res)| match res {
                        Ok(val) => val,
                        Err(e) => serde_json::json!({ "path": p, "error": e }),
                    })
                    .collect();
                Ok(serde_json::json!({
                    "count": json_results.len(),
                    "results": json_results,
                }))
            }
        }
    }
}

/// Tier 2 fetch: return exactly one code symbol's source or one doc chunk,
/// bounded by `max_lines`, with optional neighbor expansion.
pub fn handle_get_snippet(engine: &Engine, args: Value) -> Result<Value> {
    let params: GetSnippetParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let max_lines = params.max_lines.unwrap_or(500).max(1);
    let corpus_root = Path::new(&engine.config().path);
    let is_lean = params.format.as_deref() == Some("lean");

    let target_name = params.qualified_name.or(params.name);
    if let Some(ref qualified_name) = target_name {
        return fetch_code_symbol(
            engine,
            corpus_root,
            qualified_name,
            max_lines,
            params.include_neighbors,
            is_lean,
        );
    }

    if let Some(path) = params.path.as_deref() {
        if let Some(chunk_index) = params.chunk_index {
            return fetch_doc_chunk(
                engine,
                path,
                chunk_index,
                max_lines,
                params.include_neighbors,
                is_lean,
            );
        }
        return Err(Error::Config(format!(
            "get_snippet needs a chunk_index for a doc fetch on '{path}'. \
             For a whole file use Tier 3: read_file.",
        )));
    }

    Err(Error::Config(
        "get_snippet requires either `name`/`qualified_name` (code) or `path`+`chunk_index` (doc)."
            .to_string(),
    ))
}

/// Fetch a single code symbol's bounded source by qualified name (or fuzzy name),
/// optionally attaching caller/callee handles.
fn fetch_code_symbol(
    engine: &Engine,
    corpus_root: &Path,
    qualified_name: &str,
    max_lines: usize,
    include_neighbors: bool,
    is_lean: bool,
) -> Result<Value> {
    let mut matches = engine.store().find_symbols_by_qualified_name(qualified_name)?;
    if matches.is_empty() {
        matches = engine.store().find_symbols_by_normalized_scope(qualified_name)?;
    }
    if matches.is_empty() {
        matches = engine.store().find_symbols_by_name(qualified_name)?;
    }

    match matches.len() {
        0 => {
            let leaf = qualified_name.split(" > ").last().unwrap_or(qualified_name).trim();
            let leaf_candidates = engine.store().find_symbols_by_name(leaf).unwrap_or_default();
            let leaf_matches: Vec<_> =
                leaf_candidates.into_iter().filter(|s| s.name.eq_ignore_ascii_case(leaf)).collect();

            if !leaf_matches.is_empty() {
                let candidates: Vec<Value> = leaf_matches.iter().map(code_symbol_handle).collect();
                if is_lean {
                    let mut s = format!(
                        "# Candidate Suggestions for '{qualified_name}'\n\nNo code symbol matches '{qualified_name}', but found {} candidate(s) with leaf name '{leaf}'. Disambiguate with an exact scope_path:\n\n",
                        candidates.len()
                    );
                    for (i, c) in candidates.iter().enumerate() {
                        let num = i + 1;
                        let name = c["name"].as_str().unwrap_or("");
                        let scope = c["scope_path"].as_str().unwrap_or("");
                        let file = c["file_path"].as_str().unwrap_or("");
                        let start = c["start_line"].as_u64().unwrap_or(0);
                        let end = c["end_line"].as_u64().unwrap_or(0);
                        s.push_str(&format!(
                            "{num}. {name} (`{file}`:L{start}-L{end}) [scope: `{scope}`]\n"
                        ));
                        s.push_str(&format!("   -> get_snippet(name: \"{scope}\")\n"));
                    }
                    return Ok(Value::String(s));
                }
                Ok(serde_json::json!({
                    "kind": "candidate_suggestions",
                    "note": format!(
                        "No code symbol matches '{qualified_name}', but found {} candidate(s) with leaf name '{leaf}'. Disambiguate with an exact scope_path.",
                        candidates.len()
                    ),
                    "candidates": candidates,
                }))
            } else {
                Err(Error::NotFound(format!("no code symbol matches '{qualified_name}'")))
            }
        }
        1 => {
            let sym = &matches[0];
            let full_path = corpus_root.join(&sym.file_path);
            let content = read_file_lossy(&full_path)
                .map_err(|e| Error::NotFound(format!("cannot read {}: {}", sym.file_path, e)))?;
            let file_lines: Vec<&str> = content.lines().collect();

            let (source, truncated) = if sym.start_line > 0 && sym.start_line <= file_lines.len() {
                let start_idx = sym.start_line - 1;
                let end_idx = sym.end_line.min(file_lines.len());
                cap_lines(&file_lines[start_idx..end_idx], max_lines)
            } else {
                (String::new(), false)
            };

            let total_lines = file_lines.len();
            let mut out = serde_json::json!({
                "path": sym.file_path,
                "start_line": sym.start_line,
                "end_line": sym.end_line,
                "total_lines": total_lines,
                "source": source,
            });
            if let Some(ref doc) = sym.docstring {
                if !doc.trim().is_empty() {
                    out["docstring"] = serde_json::Value::String(doc.clone());
                }
            }
            if truncated {
                out["truncated"] = serde_json::Value::Bool(true);
            }

            let mut incoming: BTreeMap<String, Vec<Value>> = BTreeMap::new();
            let mut outgoing: BTreeMap<String, Vec<Value>> = BTreeMap::new();

            if include_neighbors {
                let all_symbols = engine.store().get_all_code_symbols().unwrap_or_default();
                let mut sym_map: HashMap<String, &groundcontrol_common::types::CodeSymbol> =
                    HashMap::with_capacity(all_symbols.len() * 2);
                for s in &all_symbols {
                    sym_map.insert(s.scope_path.clone(), s);
                    sym_map.insert(s.name.clone(), s);
                }

                let edges = engine.graph().get_all_edges();
                let matches_sym =
                    |candidate: &str| candidate == sym.scope_path || candidate == sym.name;

                // Grammar-driven graph relationships grouped by edge_type:
                // incoming (edges where target is this symbol)
                // outgoing (edges where source is this symbol)
                let mut seen_incoming = HashSet::new();
                let mut seen_outgoing = HashSet::new();

                for e in edges.iter().filter(|e| matches_sym(&e.target)) {
                    if seen_incoming.insert((e.edge_type.clone(), e.source.clone())) {
                        let node = if let Some(s) = sym_map.get(&e.source) {
                            code_symbol_handle(s)
                        } else {
                            serde_json::json!({
                                "name": e.source,
                                "scope_path": e.source,
                                "unresolved": true,
                            })
                        };
                        incoming.entry(e.edge_type.clone()).or_default().push(node);
                    }
                }

                for e in edges.iter().filter(|e| matches_sym(&e.source)) {
                    if seen_outgoing.insert((e.edge_type.clone(), e.target.clone())) {
                        let node = if let Some(s) = sym_map.get(&e.target) {
                            code_symbol_handle(s)
                        } else if let Some(ref target_corpus) = e.target_corpus {
                            serde_json::json!({
                                "name": e.target_symbol.as_deref().unwrap_or(&e.target),
                                "scope_path": e.target,
                                "corpus": target_corpus,
                                "file_path": e.target_path,
                                "symbol_type": e.target_kind,
                                "confidence": e.confidence,
                                "cross_corpus": true,
                            })
                        } else {
                            serde_json::json!({
                                "name": e.target,
                                "scope_path": e.target,
                                "unresolved": true,
                            })
                        };
                        outgoing.entry(e.edge_type.clone()).or_default().push(node);
                    }
                }

                out["relationships"] = serde_json::json!({
                    "incoming": incoming,
                    "outgoing": outgoing,
                });
            }

            if is_lean {
                let lean = crate::format::lean::format_lean_code_symbol(
                    &sym.name,
                    &sym.scope_path,
                    &sym.file_path,
                    sym.start_line,
                    sym.end_line,
                    total_lines,
                    sym.docstring.as_deref(),
                    &source,
                    truncated,
                    &incoming,
                    &outgoing,
                );
                Ok(Value::String(lean))
            } else {
                Ok(out)
            }
        }
        _ => {
            let candidates: Vec<Value> = matches.iter().map(code_symbol_handle).collect();
            if is_lean {
                let mut s = format!(
                    "# Ambiguous Symbol: '{qualified_name}' ({} matches)\n\nDisambiguate with an exact scope_path:\n\n",
                    candidates.len()
                );
                for (i, c) in candidates.iter().enumerate() {
                    let num = i + 1;
                    let name = c["name"].as_str().unwrap_or("");
                    let scope = c["scope_path"].as_str().unwrap_or("");
                    let file = c["file_path"].as_str().unwrap_or("");
                    let start = c["start_line"].as_u64().unwrap_or(0);
                    let end = c["end_line"].as_u64().unwrap_or(0);
                    s.push_str(&format!(
                        "{num}. {name} (`{file}`:L{start}-L{end}) [scope: `{scope}`]\n"
                    ));
                    s.push_str(&format!("   -> get_snippet(name: \"{scope}\")\n"));
                }
                return Ok(Value::String(s));
            }
            Ok(serde_json::json!({
                "kind": "ambiguous",
                "note": format!(
                    "'{qualified_name}' is ambiguous ({} matches); disambiguate with an exact scope_path.",
                    candidates.len()
                ),
                "candidates": candidates,
            }))
        }
    }
}

/// Fetch a single doc chunk's bounded text, optionally with adjacent chunks.
fn fetch_doc_chunk(
    engine: &Engine,
    path: &str,
    chunk_index: usize,
    max_lines: usize,
    include_neighbors: bool,
    is_lean: bool,
) -> Result<Value> {
    let chunks = engine.store().get_chunks_for_file(path)?;
    if chunks.is_empty() {
        return Err(Error::NotFound(format!("no indexed chunks for '{path}'")));
    }

    let chunk = chunks
        .iter()
        .find(|c| c.chunk_index == chunk_index)
        .ok_or_else(|| Error::NotFound(format!("chunk {chunk_index} not found for '{path}'")))?;

    let chunk_text = engine.fetch_chunk_text(path, chunk.start_byte, chunk.end_byte)?;
    let text_lines: Vec<&str> = chunk_text.lines().collect();
    let (text, truncated) = cap_lines(&text_lines, max_lines);

    let full_doc_path = Path::new(&engine.config().path).join(path);
    let total_lines =
        read_file_lossy(&full_doc_path).map(|c| c.lines().count()).unwrap_or(text_lines.len());

    let mut out = serde_json::json!({
        "path": path,
        "chunk_index": chunk.chunk_index,
        "total_lines": total_lines,
        "text": text,
    });
    if truncated {
        out["truncated"] = serde_json::Value::Bool(true);
    }

    if include_neighbors {
        let neighbor_cap = (max_lines / 2).max(1);
        let neighbor = |target: usize| -> Option<Value> {
            chunks.iter().find(|c| c.chunk_index == target).and_then(|c| {
                let n_text = engine.fetch_chunk_text(path, c.start_byte, c.end_byte).ok()?;
                let nlines: Vec<&str> = n_text.lines().collect();
                let (ntext, ntrunc) = cap_lines(&nlines, neighbor_cap);
                Some(serde_json::json!({
                    "chunk_index": c.chunk_index,
                    "start_byte": c.start_byte,
                    "end_byte": c.end_byte,
                    "text": ntext,
                    "truncated": ntrunc,
                }))
            })
        };

        out["previous"] = chunk_index.checked_sub(1).and_then(neighbor).unwrap_or(Value::Null);
        out["next"] = neighbor(chunk_index + 1).unwrap_or(Value::Null);
    }

    if is_lean {
        let incoming = BTreeMap::new();
        let outgoing = BTreeMap::new();
        let lean = crate::format::lean::format_lean_doc_chunk(
            path,
            chunk.chunk_index,
            chunk.start_line,
            chunk.end_line,
            total_lines,
            &text,
            truncated,
            &incoming,
            &outgoing,
        );
        Ok(Value::String(lean))
    } else {
        Ok(out)
    }
}

/// List all indexed notes with metadata, or inspect single note's frontmatter and metadata if `path` is provided.
pub fn handle_list_notes(engine: &Engine, args: Value) -> Result<Value> {
    let params: ListNotesParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    if let Some(path) = params.path {
        let corpus_path = PathBuf::from(&engine.config().path);
        let full_path = corpus_path.join(&path);
        let content = std::fs::read_to_string(&full_path)
            .map_err(|e| Error::NotFound(format!("cannot read {}: {}", path, e)))?;
        let doc = groundcontrol_core::parser::parse_document(Path::new(&path), &content)?;
        return Ok(serde_json::json!({
            "path": path,
            "title": doc.title,
            "frontmatter": doc.frontmatter,
            "content_hash": doc.content_hash,
        }));
    }

    let limit = params.limit.unwrap_or(100);
    let offset = params.offset.unwrap_or(0);

    let files = engine.store().list_files()?;

    let items: Vec<NoteListItem> = files
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|f| NoteListItem {
            path: f.path,
            title: f.title,
            template: f.template,
            content_hash: f.content_hash,
        })
        .collect();

    serde_json::to_value(items).map_err(|e| Error::Config(format!("serialize error: {}", e)))
}
