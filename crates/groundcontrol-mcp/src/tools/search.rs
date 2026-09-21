//! Search tools: `search`, `search_related`.

use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use groundcontrol_common::ports::{MetadataCatalog, SearchQuery, SearchService};
use groundcontrol_common::{Error, Result};
use groundcontrol_core::engine::Engine;

use super::read::{cap_lines, read_file_lossy};
use super::registry::build_dynamic_schema_envelope;

#[derive(Debug, Deserialize)]
pub(crate) struct SearchParams {
    pub query: String,
    #[serde(default)]
    pub mode: Option<String>,
    pub limit: Option<usize>,
    pub depth: Option<String>,
    pub graph_depth: Option<usize>,
    pub edge_types: Option<Vec<String>>,
    pub edge_class: Option<String>,
    pub decompose: Option<bool>,
    pub modality: Option<String>,
    pub detail: Option<String>,
    pub snippets: Option<usize>,
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchRelatedParams {
    pub seeds: Vec<String>,
    pub limit: Option<usize>,
    pub modality: Option<String>,
    pub detail: Option<String>,
}

/// Apply detail level shaping to search results.
///
/// `detail == "ids"` strips the `snippet`, `lineage`, and `score_components`
/// from every result, leaving bare handles (path/qualified-name + line range + metadata carried by
/// `entity_kind`/`language`/`chunk_index`). Any other value (including the
/// omitted default) keeps the existing short snippet and metadata. Full bodies are never
/// emitted here — callers fetch source via `get_snippet`.
fn apply_detail(
    mut results: Vec<groundcontrol_common::types::SearchResult>,
    detail: Option<&str>,
) -> Vec<groundcontrol_common::types::SearchResult> {
    if detail == Some("ids") {
        for r in &mut results {
            r.snippet = None;
            r.lineage = None;
            r.score_components = None;
        }
    }
    results
}

/// Inlines bounded source text for the top K results across a partition.
fn populate_top_snippets(
    engine: &Engine,
    results: &mut [groundcontrol_common::types::SearchResult],
    k: usize,
    max_lines: usize,
) {
    let corpus_root = Path::new(&engine.config().path);
    for (i, item) in results.iter_mut().enumerate() {
        if i >= k {
            item.snippet = None;
            continue;
        }

        if let Some(ref s) = item.snippet {
            if s.len() > 120 {
                let lines: Vec<&str> = s.lines().collect();
                if lines.len() > max_lines {
                    let (capped, _) = cap_lines(&lines, max_lines);
                    item.snippet = Some(capped);
                }
                continue;
            }
        }

        if let Some(chunk_index) = item.chunk_index {
            if let Ok(chunks) = engine.store().get_chunks_for_file(&item.path) {
                if let Some(chunk) = chunks.iter().find(|c| c.chunk_index == chunk_index) {
                    if let Ok(chunk_text) =
                        engine.fetch_chunk_text(&item.path, chunk.start_byte, chunk.end_byte)
                    {
                        let lines: Vec<&str> = chunk_text.lines().collect();
                        let (capped, _) = cap_lines(&lines, max_lines);
                        item.snippet = Some(capped);
                        continue;
                    }
                }
            }
        }

        if let Ok(symbols) = engine.store().find_symbols_by_name(&item.path) {
            if let Some(sym) = symbols.first() {
                let full_path = corpus_root.join(&sym.file_path);
                if let Ok(content) = read_file_lossy(&full_path) {
                    let file_lines: Vec<&str> = content.lines().collect();
                    if sym.start_line > 0 && sym.start_line <= file_lines.len() {
                        let start_idx = sym.start_line - 1;
                        let end_idx = sym.end_line.min(file_lines.len());
                        let (capped, _) = cap_lines(&file_lines[start_idx..end_idx], max_lines);
                        item.snippet = Some(capped);
                        continue;
                    }
                }
            }
        }

        let full_path = corpus_root.join(&item.path);
        if let Ok(content) = read_file_lossy(&full_path) {
            let lines: Vec<&str> = content.lines().collect();
            let (capped, _) = cap_lines(&lines, max_lines);
            item.snippet = Some(capped);
        }
    }
}

/// Consolidated search tool: dispatches to a retrieval mode selected by `mode`
/// (default `hybrid`). Modes: `bm25`, `semantic`, `hybrid`, `graph`, `explain`.
///
/// This is a thin adapter: it obtains the engine's search service (which
/// resolves the retrieval backends internally) and delegates the mode dispatch
/// to it via the [`SearchService`] port, then applies detail/verbosity shaping
/// and JSON serialization. Every mode honors `modality` (docs|code|both) and
/// `detail` (ids|default) via `apply_detail`. `explain` returns the
/// score-breakdown shape ([`SearchService::explain`]) rather than a plain
/// result array.
pub fn handle_search(engine: &Engine, args: Value) -> Result<Value> {
    let params: SearchParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let mode_str = params.mode.as_deref().unwrap_or("hybrid").to_string();
    let is_semantic = mode_str == "semantic";
    let is_explain = mode_str == "explain";
    let modality = params
        .modality
        .as_deref()
        .and_then(groundcontrol_common::types::Modality::from_str_name)
        .unwrap_or_default();
    let depth = params
        .depth
        .as_deref()
        .and_then(groundcontrol_common::types::SearchDepth::from_str_name)
        .unwrap_or_default();

    // Semantic mode lazily initializes the embedder, but only once the fast-mode
    // guard (no vector index) has passed — mirroring the original ordering.
    if is_semantic && engine.has_vector_index() {
        let _ = engine.ensure_embedder()?;
    }

    // Build the search service from the engine (it resolves its own backends
    // internally) and dispatch through the port. Detail/verbosity shaping and
    // serialization stay here.
    let service = engine.search_service();

    let query = SearchQuery {
        query: params.query,
        mode: params.mode,
        limit: params.limit,
        modality,
        depth,
        graph_depth: params.graph_depth,
        edge_types: params.edge_types,
        edge_class: params.edge_class,
        decompose: params.decompose,
        snippets: params.snippets,
    };

    if is_explain {
        let mut explanations = service.explain(&query)?;

        // Tier-1: `detail=ids` strips snippets, leaving bare handles + score breakdown.
        if params.detail.as_deref() == Some("ids") {
            for e in &mut explanations {
                e.snippet = None;
            }
        }

        serde_json::to_value(explanations)
            .map_err(|e| Error::Config(format!("serialize error: {}", e)))
    } else {
        let results = service.search(&query)?;
        let results = apply_detail(results, params.detail.as_deref());

        let mut docs_items = Vec::new();
        let mut code_items = Vec::new();

        for r in results {
            let is_code = r
                .entity_kind
                .as_ref()
                .map(|k| k.is_code())
                .unwrap_or_else(|| !r.path.ends_with(".md"));
            if is_code {
                code_items.push(r);
            } else {
                docs_items.push(r);
            }
        }

        let is_lean = params.detail.as_deref() == Some("ids");
        let k =
            if is_lean { 0 } else { params.snippets.unwrap_or_else(|| params.limit.unwrap_or(10)) };

        populate_top_snippets(engine, &mut docs_items, k, 20);
        populate_top_snippets(engine, &mut code_items, k, 20);

        if is_lean {
            for item in &mut docs_items {
                item.graph_affordances = None;
                item.graph = None;
                item.score_components = None;
                item.language = None;
                item.entity_kind = None;
                item.chunk_index = None;
                item.symbol = None;
            }
        } else {
            for item in &mut docs_items {
                item.language = None;
                item.entity_kind = None;
                item.chunk_index = None;
                item.symbol = None;
                item.graph_affordances = None;
                item.graph = engine.format_cypher_affordances(&item.path, 3);
            }
        }

        let code_paths: Vec<&str> = code_items.iter().map(|item| item.path.as_str()).collect();
        let symbols_by_file =
            engine.store().get_code_symbols_for_files(&code_paths).unwrap_or_default();

        for item in &mut code_items {
            let mut matched_symbol: Option<String> = None;
            let mut target_scope_path: Option<String> = None;

            if let Some(file_symbols) = symbols_by_file.get(&item.path) {
                if let Some(chunk_index) = item.chunk_index {
                    if let Ok(chunks) = engine.store().get_chunks_for_file(&item.path) {
                        if let Some(chunk) = chunks.iter().find(|c| c.chunk_index == chunk_index) {
                            if let Some(sym) = file_symbols.iter().find(|s| {
                                s.start_line <= chunk.end_line && s.end_line >= chunk.start_line
                            }) {
                                matched_symbol = Some(sym.name.clone());
                                target_scope_path = Some(sym.scope_path.clone());
                            }
                        }
                    }
                }
                if matched_symbol.is_none() {
                    if let Some(first_sym) = file_symbols.first() {
                        matched_symbol = Some(first_sym.name.clone());
                        target_scope_path = Some(first_sym.scope_path.clone());
                    }
                }
            } else if let Ok(symbols) = engine.store().find_symbols_by_name(&item.path) {
                if let Some(sym) = symbols.first() {
                    matched_symbol = Some(sym.name.clone());
                    target_scope_path = Some(sym.scope_path.clone());
                }
            }

            if is_lean {
                item.graph_affordances = None;
                item.graph = None;
                item.score_components = None;
            } else {
                let cypher = if let Some(ref scope) = target_scope_path {
                    engine.format_cypher_affordances(scope, 3)
                } else {
                    None
                };

                let cypher = cypher.or_else(|| {
                    if let Some(file_symbols) = symbols_by_file.get(&item.path) {
                        for sym in file_symbols {
                            if let Some(c) = engine.format_cypher_affordances(&sym.scope_path, 3) {
                                return Some(c);
                            }
                        }
                    }
                    engine.format_cypher_affordances(&item.path, 3)
                });

                item.graph = cypher;
                item.graph_affordances = None;
            }

            // Zero semantic duplication:
            // 1. Language is omitted (file extension in `path` conveys it).
            item.language = None;
            // 2. Entity kind is omitted (implied by snippet/symbol).
            item.entity_kind = None;
            // 3. Chunk index is omitted.
            item.chunk_index = None;
            // 4. Bare symbol identifier is surfaced only when snippet is omitted (trailing hits or snippets: 0)
            //    or when in lean emission mode for Tier 2 progressive disclosure scent.
            if params.format.as_deref() == Some("lean") || item.snippet.is_none() {
                item.symbol = matched_symbol;
            } else {
                item.symbol = None;
            }
        }

        if params.format.as_deref() == Some("lean") {
            let lean_text = crate::format::lean::format_lean_search(
                &query.query,
                &mode_str,
                Some(engine.config().name.as_str()),
                &code_items,
                &docs_items,
                is_lean,
            );
            return Ok(Value::String(lean_text));
        }

        let active_doc_edges = if is_lean {
            Vec::new()
        } else {
            engine.active_edge_types(Some(groundcontrol_common::config::EdgeClass::Structural))
        };
        let docs_partition =
            if !docs_items.is_empty() || modality != groundcontrol_common::types::Modality::Code {
                Some(groundcontrol_common::types::SearchPartition {
                    total_matches: docs_items.len(),
                    top_k_returned: docs_items.len(),
                    schema_envelope: if is_lean {
                        groundcontrol_common::types::SchemaEnvelope::default()
                    } else {
                        build_dynamic_schema_envelope(&docs_items, false, &active_doc_edges)
                    },
                    results: docs_items,
                })
            } else {
                None
            };

        let active_code_edges = if is_lean {
            Vec::new()
        } else {
            engine.active_edge_types(Some(groundcontrol_common::config::EdgeClass::Code))
        };
        let code_partition =
            if !code_items.is_empty() || modality != groundcontrol_common::types::Modality::Docs {
                Some(groundcontrol_common::types::SearchPartition {
                    total_matches: code_items.len(),
                    top_k_returned: code_items.len(),
                    schema_envelope: if is_lean {
                        groundcontrol_common::types::SchemaEnvelope::default()
                    } else {
                        build_dynamic_schema_envelope(&code_items, true, &active_code_edges)
                    },
                    results: code_items,
                })
            } else {
                None
            };

        let response = groundcontrol_common::types::SearchResponse {
            docs: docs_partition,
            code: code_partition,
        };

        serde_json::to_value(response).map_err(|e| Error::Config(format!("serialize error: {}", e)))
    }
}

/// Find related documents via PPR approximation.
pub fn handle_search_related(engine: &Engine, args: Value) -> Result<Value> {
    let params: SearchRelatedParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let limit = params.limit.unwrap_or(10);
    let modality = params
        .modality
        .as_deref()
        .and_then(groundcontrol_common::types::Modality::from_str_name)
        .unwrap_or_default();

    // Related search only traverses the graph, but the service is built the same
    // way `handle_search` builds it; the embedder is left as-is (never lazily
    // initialized here, matching prior behaviour) since related does not touch
    // it. Detail/verbosity shaping stays here.
    let service = engine.search_service();

    let results = service.search_related(&params.seeds, limit, modality)?;
    let results = apply_detail(results, params.detail.as_deref());

    serde_json::to_value(results).map_err(|e| Error::Config(format!("serialize error: {}", e)))
}
