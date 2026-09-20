//! System and corpus management tools: `status`, `sync_corpus`, `list_corpora`, `index_corpus`, `unload_corpus`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use ctxvault_common::ports::{GraphStore, MetadataCatalog};
use ctxvault_common::{Error, Result};
use ctxvault_core::corpus_manager::CorpusManager;
use ctxvault_core::engine::Engine;

#[derive(Debug, Deserialize)]
pub(crate) struct StatusParams {
    pub scope: Option<String>,
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SyncCorpusParams {
    pub mode: Option<String>,
    pub batch_size: Option<usize>,
    pub resume: Option<bool>,
    pub fast: Option<bool>,
    pub index_mode: Option<String>,
}

/// Report index coverage + parse status for the given paths or path prefixes.
fn check_index_coverage_inner(engine: &Engine, paths: &[String]) -> Result<Value> {
    let all_files = engine.store().list_files()?;

    let mut reports = Vec::with_capacity(paths.len());
    let mut covered = 0usize;

    for scope in paths {
        let matched: Vec<&str> = all_files
            .iter()
            .map(|f| f.path.as_str())
            .filter(|p| *p == scope || p.starts_with(scope.as_str()))
            .collect();

        let indexed = !matched.is_empty();
        let mut chunk_count = 0usize;
        let mut symbol_count = 0usize;
        for file_path in &matched {
            chunk_count += engine.store().get_chunks_for_file(file_path).map(|c| c.len())?;
            symbol_count += engine.store().get_code_symbols_for_file(file_path).map(|s| s.len())?;
        }

        let parsed = indexed && (chunk_count > 0 || symbol_count > 0);
        if indexed {
            covered += 1;
        }

        let mut matched_files: Vec<String> = matched.iter().map(|p| p.to_string()).collect();
        matched_files.sort();

        reports.push(serde_json::json!({
            "path": scope,
            "indexed": indexed,
            "parsed": parsed,
            "chunk_count": chunk_count,
            "symbol_count": symbol_count,
            "matched_files": matched_files,
        }));
    }

    let total = paths.len();
    Ok(serde_json::json!({
        "reports": reports,
        "summary": {
            "total": total,
            "covered": covered,
            "uncovered": total - covered,
        },
    }))
}

/// Per-corpus statistics (document counts, mode, chunking, embedding model).
fn corpus_stats(engine: &Engine) -> Result<Value> {
    let files = engine.store().list_files()?;
    let is_indexed = engine.is_indexed();
    Ok(serde_json::json!({
        "status": "healthy",
        "corpus_name": engine.config().name,
        "corpus_path": engine.config().path,
        "document_count": files.len(),
        "indexed": is_indexed,
        "mode": format!("{:?}", engine.config().mode),
        "index_mode": format!("{:?}", engine.config().index_mode),
        "chunking": format!("{:?}", engine.config().chunking.strategy),
        "embedding_model": engine.config().embedding.model,
    }))
}

/// Consolidated status tool (engine-level): combines per-corpus statistics,
/// indexing progress, graph topology/density, and coverage inspection.
pub fn handle_status(engine: &Engine, args: Value) -> Result<Value> {
    let params: StatusParams =
        serde_json::from_value(args).unwrap_or(StatusParams { scope: None, paths: None });
    let scope = params.scope.as_deref().unwrap_or("all");

    match scope {
        "corpus" => corpus_stats(engine),
        "indexing" => {
            let status = engine.get_indexing_status()?;
            serde_json::to_value(status)
                .map_err(|e| Error::Config(format!("serialize error: {}", e)))
        }
        "graph" => {
            let stats = engine.graph().stats();
            let density = engine.analyze_density(10);
            Ok(serde_json::json!({
                "stats": stats,
                "density": density,
            }))
        }
        "coverage" => {
            let paths = params.paths.unwrap_or_default();
            check_index_coverage_inner(engine, &paths)
        }
        "census" | "architecture" => {
            let stats = engine.graph().stats();
            let symbols = engine.store().get_all_code_symbols().unwrap_or_default();
            let mut symbol_types: HashMap<String, usize> = HashMap::new();
            let mut languages: HashMap<String, usize> = HashMap::new();
            for s in &symbols {
                *symbol_types.entry(format!("{:?}", s.symbol_type)).or_insert(0) += 1;
                *languages.entry(s.language.clone()).or_insert(0) += 1;
            }
            let active_edges = engine.active_edge_types(None);
            let files = engine.store().list_files().unwrap_or_default();
            Ok(serde_json::json!({
                "corpus_name": engine.config().name,
                "corpus_path": engine.config().path,
                "total_files": files.len(),
                "total_symbols": symbols.len(),
                "total_graph_nodes": stats.node_count,
                "total_graph_edges": stats.edge_count,
                "symbol_types": symbol_types,
                "languages": languages,
                "active_edge_types": active_edges,
            }))
        }
        _ => {
            let corpus = corpus_stats(engine)?;
            let indexing = serde_json::to_value(engine.get_indexing_status()?)
                .map_err(|e| Error::Config(format!("serialize error: {}", e)))?;
            let stats = engine.graph().stats();
            let density = engine.analyze_density(10);
            Ok(serde_json::json!({
                "corpus": corpus,
                "indexing": indexing,
                "graph": {
                    "stats": stats,
                    "density": density,
                },
            }))
        }
    }
}

/// Helper to apply index_mode overrides dynamically on an engine.
fn apply_index_mode_override(
    engine: &mut Engine,
    index_mode: Option<&str>,
    fast: Option<bool>,
) -> Result<()> {
    if let Some(mode_str) = index_mode {
        match mode_str.to_lowercase().as_str() {
            "fast" => engine.set_index_mode(ctxvault_common::config::IndexMode::Fast),
            "full" => engine.set_index_mode(ctxvault_common::config::IndexMode::Full),
            other => return Err(Error::Config(format!("invalid index_mode '{}'", other))),
        }
    } else if let Some(fast) = fast {
        engine.set_index_mode(if fast {
            ctxvault_common::config::IndexMode::Fast
        } else {
            ctxvault_common::config::IndexMode::Full
        });
    }
    Ok(())
}

/// Sync or reindex corpus in configurable batches. Supports mode: "delta" | "full" | "reembed".
pub fn handle_sync_corpus(engine: &mut Engine, args: Value) -> Result<Value> {
    let params: SyncCorpusParams = serde_json::from_value(args).unwrap_or(SyncCorpusParams {
        mode: None,
        batch_size: None,
        resume: None,
        fast: None,
        index_mode: None,
    });
    apply_index_mode_override(engine, params.index_mode.as_deref(), params.fast)?;

    match params.mode.as_deref().unwrap_or("delta") {
        "reembed" => {
            let was_stale = engine.vectors_stale();
            let old_version = engine.stored_model_version().map(|s| s.to_string());
            let chunks_reembedded = engine.reembed()?;
            let new_version = engine.stored_model_version().unwrap_or("unknown").to_string();
            Ok(serde_json::json!({
                "status": "complete",
                "mode": "reembed",
                "chunks_reembedded": chunks_reembedded,
                "was_stale": was_stale,
                "previous_model_version": old_version,
                "current_model_version": new_version,
            }))
        }
        "full" => {
            let batch_size = params.batch_size.unwrap_or(50);
            let resume = params.resume.unwrap_or(true);
            let count = engine.full_reindex_paginated(batch_size, resume)?;
            Ok(serde_json::json!({
                "status": "complete",
                "mode": "full",
                "files_indexed": count,
                "batch_size": batch_size,
                "resumed": resume,
            }))
        }
        _ => {
            let batch_size = params.batch_size.unwrap_or(50);
            let result = engine.delta_scan_paginated(batch_size)?;
            Ok(serde_json::json!({
                "status": "complete",
                "mode": "delta",
                "new_files": result.new_files.len(),
                "modified_files": result.modified_files.len(),
                "deleted_files": result.deleted_files.len(),
                "new": result.new_files,
                "modified": result.modified_files,
                "deleted": result.deleted_files,
            }))
        }
    }
}

/// Get overall system status from the CorpusManager.
pub fn handle_get_status(manager: &CorpusManager) -> Result<Value> {
    let corpora = manager.list_corpora();
    let default_name = manager.default_corpus_name().unwrap_or("none");

    let corpora_info: Vec<Value> = corpora
        .iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "path": c.path,
                "mode": c.mode,
                "index_mode": c.index_mode,
                "file_count": c.file_count,
                "embedder_active": c.embedder_active,
                "vector_count": c.vector_count,
                "graph_node_count": c.graph_node_count,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "corpus_count": manager.corpus_count(),
        "default_corpus": default_name,
        "corpora": corpora_info,
    }))
}

/// Handle `list_corpora` across active and cached corpora.
pub fn handle_list_corpora_manager(manager: &CorpusManager, args: Value) -> Result<Value> {
    let include_cached = args.get("include_cached").and_then(Value::as_bool).unwrap_or(true);
    let loaded = manager.list_corpora();
    let loaded_names: HashSet<String> = loaded.iter().map(|c| c.name.clone()).collect();

    let mut corpora_info: Vec<Value> = loaded
        .into_iter()
        .map(|c| {
            let index_path = ctxvault_common::config::get_corpus_index_dir(&c.name);
            serde_json::json!({
                "name": c.name,
                "path": c.path,
                "index_path": index_path.to_string_lossy().replace('\\', "/"),
                "status": "active",
                "mode": c.mode,
                "index_mode": c.index_mode,
                "file_count": c.file_count,
                "embedder_active": c.embedder_active,
                "vector_count": c.vector_count,
                "graph_node_count": c.graph_node_count,
            })
        })
        .collect();

    if include_cached {
        for cached in manager.discover_cached_corpora() {
            if !loaded_names.contains(&cached) {
                let cache_dir = ctxvault_common::config::get_corpus_index_dir(&cached);
                let source_path =
                    ctxvault_core::corpus_manager::CorpusManager::get_cached_corpus_source_path(
                        &cached,
                    )
                    .unwrap_or_else(|| cache_dir.to_string_lossy().replace('\\', "/"));
                corpora_info.push(serde_json::json!({
                    "name": cached,
                    "status": "cached",
                    "path": source_path,
                    "index_path": cache_dir.to_string_lossy().replace('\\', "/"),
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "corpora": corpora_info,
        "default_corpus": manager.default_corpus_name(),
        "total_active": manager.corpus_count(),
    }))
}

/// Handle `index_corpus` dynamically mounting and indexing a new repository.
pub fn handle_index_corpus_manager(manager: &mut CorpusManager, args: Value) -> Result<Value> {
    let path_str = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("missing required argument 'path'".to_string()))?;

    let corpus_path = PathBuf::from(path_str);
    let name_override = args.get("name").and_then(Value::as_str);
    let name = manager.ensure_corpus_with_name(&corpus_path, name_override)?;

    let do_reindex = args.get("reindex").and_then(Value::as_bool).unwrap_or(false);
    let do_sync = args.get("sync").and_then(Value::as_bool).unwrap_or(true);
    let fast = args.get("fast").and_then(Value::as_bool).unwrap_or(false);
    let batch_size =
        args.get("batch_size").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(50);

    let engine = manager.get_engine_mut(&name)?;

    if fast {
        engine.config_mut().index_mode = ctxvault_common::config::IndexMode::Fast;
    }

    let index_stats = if do_reindex {
        let count = engine.full_reindex_paginated(batch_size, false)?;
        serde_json::json!({ "reindexed_files": count })
    } else if do_sync {
        let delta = engine.delta_scan_paginated(batch_size)?;
        serde_json::json!({
            "new_files": delta.new_files.len(),
            "modified_files": delta.modified_files.len(),
            "deleted_files": delta.deleted_files.len()
        })
    } else {
        serde_json::json!({ "status": "mounted_without_indexing" })
    };

    let file_count = engine.store().list_files().map(|f| f.len()).unwrap_or(0);

    Ok(serde_json::json!({
        "status": "success",
        "corpus": name,
        "path": path_str,
        "file_count": file_count,
        "indexing": index_stats,
    }))
}

/// Handle `unload_corpus` freeing memory from an open engine.
pub fn handle_unload_corpus_manager(manager: &mut CorpusManager, args: Value) -> Result<Value> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("missing required argument 'name'".to_string()))?;

    let unloaded = manager.unload_corpus(name)?;
    Ok(serde_json::json!({
        "status": if unloaded { "unloaded" } else { "not_found" },
        "corpus": name
    }))
}

/// Dummy placeholder for single-engine registration of `list_corpora`.
pub fn handle_list_corpora_dummy(_engine: &Engine, _args: Value) -> Result<Value> {
    Err(Error::Config("list_corpora is a manager-level tool".to_string()))
}

/// Dummy placeholder for single-engine registration of `index_corpus`.
pub fn handle_index_corpus_dummy(_engine: &mut Engine, _args: Value) -> Result<Value> {
    Err(Error::Config("index_corpus is a manager-level tool".to_string()))
}

/// Dummy placeholder for single-engine registration of `unload_corpus`.
pub fn handle_unload_corpus_dummy(_engine: &mut Engine, _args: Value) -> Result<Value> {
    Err(Error::Config("unload_corpus is a manager-level tool".to_string()))
}
