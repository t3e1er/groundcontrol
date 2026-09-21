//! Read-only snapshot loader for `groundcontrol` corpus indices on disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use groundcontrol_common::config::{get_corpora_cache_dir, get_corpus_index_dir};
use groundcontrol_core::graph::KnowledgeGraph;
use tracing::{info, warn};

use crate::error::{GraphViewError, Result};

/// In-memory read-only snapshot of a single corpus graph.
pub struct CorpusSnapshot {
    /// Name of the corpus.
    pub name: String,
    /// Absolute path to corpus index directory.
    pub index_dir: PathBuf,
    /// Loaded petgraph knowledge graph.
    pub graph: KnowledgeGraph,
    /// Authoritative AST-derived symbol types from meta.db (key -> symbol_type).
    pub ast_types: Arc<HashMap<String, String>>,
}

impl CorpusSnapshot {
    /// Load a corpus snapshot from disk in read-only mode.
    pub fn load_from_dir(name: &str, index_dir: &Path) -> Result<Self> {
        let graph_path = index_dir.join("graph.bin");
        if !graph_path.exists() {
            return Err(GraphViewError::NotFound(format!(
                "No graph.bin found in {}",
                index_dir.display()
            )));
        }

        let graph = KnowledgeGraph::load(&graph_path)
            .map_err(|e| GraphViewError::GraphLoad(format!("{}: {}", name, e)))?;

        // Extract AST-derived entity classes from SQLite catalog meta.db
        let mut ast_types = HashMap::new();
        let meta_path = index_dir.join("meta.db");
        if meta_path.exists() {
            if let Ok(conn) = rusqlite::Connection::open_with_flags(
                &meta_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
            ) {
                if let Ok(mut stmt) = conn
                    .prepare("SELECT name, scope_path, file_path, symbol_type FROM code_symbols")
                {
                    if let Ok(rows) = stmt.query_map([], |row| {
                        let name: String = row.get(0)?;
                        let scope: String = row.get(1)?;
                        let file: String = row.get(2)?;
                        let sym_type: String = row.get(3)?;
                        Ok((name, scope, file, sym_type))
                    }) {
                        for row in rows.flatten() {
                            let (name, scope, file, sym_type) = row;
                            let norm_file = file.replace('\\', "/");
                            if !name.is_empty() {
                                ast_types.insert(name.clone(), sym_type.clone());
                            }
                            if !scope.is_empty() {
                                let full_scope = format!("{}::{}", scope, name);
                                ast_types.insert(scope.clone(), sym_type.clone());
                                ast_types.insert(full_scope.clone(), sym_type.clone());
                                ast_types.insert(
                                    format!("{}#{}", norm_file, full_scope),
                                    sym_type.clone(),
                                );
                                ast_types
                                    .insert(format!("{}#{}", norm_file, scope), sym_type.clone());
                                ast_types
                                    .insert(format!("{}:{}", norm_file, scope), sym_type.clone());
                            }
                            ast_types.insert(format!("{}#{}", norm_file, name), sym_type.clone());
                            ast_types.insert(format!("{}:{}", norm_file, name), sym_type.clone());
                            ast_types.insert(norm_file.clone(), "Module".to_string());
                        }
                    }
                }
                if let Ok(mut doc_stmt) = conn.prepare("SELECT path FROM documents") {
                    if let Ok(rows) = doc_stmt.query_map([], |row| row.get::<_, String>(0)) {
                        for path in rows.flatten() {
                            let norm = path.replace('\\', "/");
                            ast_types.insert(norm.clone(), "DocNode".to_string());
                            let base = norm.split('/').next_back().unwrap_or(&norm);
                            ast_types.insert(base.to_string(), "DocNode".to_string());
                        }
                    }
                }
            }
        }

        info!(
            corpus = %name,
            nodes = graph.node_count(),
            edges = graph.edge_count(),
            ast_symbols = ast_types.len(),
            "Loaded read-only corpus graph snapshot"
        );

        Ok(Self {
            name: name.to_string(),
            index_dir: index_dir.to_path_buf(),
            graph,
            ast_types: Arc::new(ast_types),
        })
    }
}

/// Catalog holding read-only snapshots across all detected corpora.
pub struct CorpusCatalog {
    base_dir: PathBuf,
    snapshots: HashMap<String, Arc<CorpusSnapshot>>,
}

impl CorpusCatalog {
    /// Scan and load all corpora from the default or specified cache directory.
    pub fn load_all(custom_dir: Option<PathBuf>) -> Result<Self> {
        let base_dir = custom_dir.unwrap_or_else(get_corpora_cache_dir);
        let mut snapshots = HashMap::new();

        if base_dir.exists() && base_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&base_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let graph_path = path.join("graph.bin");
                        if graph_path.exists() {
                            match CorpusSnapshot::load_from_dir(&name, &path) {
                                Ok(snapshot) => {
                                    snapshots.insert(name, Arc::new(snapshot));
                                }
                                Err(err) => {
                                    warn!(corpus = %name, error = %err, "Failed to load corpus snapshot");
                                }
                            }
                        }
                    }
                }
            }
        }

        info!(
            corpora_count = snapshots.len(),
            base_dir = %base_dir.display(),
            "Corpus catalog initialized"
        );

        Ok(Self { base_dir, snapshots })
    }

    /// Load or reload a single specific corpus by name.
    pub fn reload_corpus(&mut self, name: &str) -> Result<Arc<CorpusSnapshot>> {
        let index_dir = get_corpus_index_dir(name);
        let snapshot = Arc::new(CorpusSnapshot::load_from_dir(name, &index_dir)?);
        self.snapshots.insert(name.to_string(), snapshot.clone());
        Ok(snapshot)
    }

    /// Get an in-memory snapshot of a corpus.
    pub fn get_corpus(&self, name: &str) -> Option<Arc<CorpusSnapshot>> {
        self.snapshots.get(name).cloned()
    }

    /// List all loaded corpus names.
    pub fn corpus_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.snapshots.keys().cloned().collect();
        names.sort();
        names
    }

    /// Number of loaded corpora.
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Whether any corpora are loaded.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Reference to base directory.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}
