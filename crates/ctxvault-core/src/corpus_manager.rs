//! Multi-corpus support: manages multiple independent Engine instances.
//!
//! Each corpus is an independent unit with its own BM25 index, vector index,
//! knowledge graph, and SQLite store. The [`CorpusManager`] provides a unified
//! interface for routing operations to the correct engine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ctxvault_common::config::{CorpusConfig, EdgeClass};
use ctxvault_common::ports::{GraphStore, MetadataCatalog};
use ctxvault_common::types::{
    CodeSymbol, CodeSymbolType, EdgeProvenance, ExternalRefKind, ResolutionConfidence,
};
use ctxvault_common::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::engine::Engine;

/// Which resolver tier produced a cross-corpus match, ordered by trust
/// (highest first).
///
/// The cross-corpus resolver forms a *trust ladder*: SCIP compiler-grade
/// monikers are tried first, then in-engine hybrid-LSP data, then plain
/// qualified-name matching as the always-available fallback. This enum lets the
/// linking pass record *which* tier resolved a reference so the emitted edge can
/// be confidence-banded, and lets tests assert the tier that fired.
///
/// `HybridLsp` is a real classification (SCIP-grade vs LSP-grade vs
/// name-matched) and is kept in the trust order for documentation and future
/// use, but the currently *live* ladder is [`Self::Scip`] → [`Self::QualName`]:
/// the in-engine hybrid-LSP type environment is per-file and holds no
/// cross-corpus symbol table, so wiring a `HybridLsp` runtime tier today would
/// be a do-nothing stub. See [`crate::graph::hybrid_lsp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolverKind {
    /// Resolved via a SCIP moniker (compiler-grade, highest trust).
    Scip,
    /// Reserved for in-engine hybrid-LSP cross-corpus resolution (not yet live).
    HybridLsp,
    /// Resolved via qualified-name matching against the SQLite symbol catalog
    /// (always-available fallback).
    QualName,
}

/// Status information for a single corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusInfo {
    /// Corpus name.
    pub name: String,
    /// Path to the corpus directory.
    pub path: String,
    /// Access mode (read-write or read-only).
    pub mode: String,
    /// Indexing mode (Full, DocsEmbed, or Fast).
    pub index_mode: String,
    /// Number of indexed files.
    pub file_count: usize,
    /// Whether the embedder is active for this corpus.
    pub embedder_active: bool,
    /// Number of vectors in the index.
    pub vector_count: usize,
    /// Number of nodes in the knowledge graph.
    pub graph_node_count: usize,
}

/// One intra-corpus node visited during a [`FederatedTraversal`].
///
/// Tagged with the corpus it lives in and its BFS depth *within that corpus*
/// (`0` for the node the traversal entered the corpus at — the origin start
/// node, or a continuation start node after crossing a cross-corpus edge).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedNode {
    /// Corpus this node lives in.
    pub corpus: String,
    /// Node key (path / scope_path / route key) within `corpus`.
    pub node: String,
    /// BFS depth within `corpus` from the node the traversal entered it at.
    pub depth: usize,
}

/// One cross-corpus hop encountered during a federated traversal.
///
/// Emitted whenever BFS in a corpus reaches an outgoing edge whose
/// `target_corpus` is set. It is always recorded — even when continuation is
/// disabled or the hop budget is exhausted — so the caller sees the full set of
/// seams the traversal touched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusHop {
    /// Corpus the crossed edge originates in.
    pub from_corpus: String,
    /// Origin node (in `from_corpus`) carrying the cross-corpus edge.
    pub from_node: String,
    /// `target_corpus` of the crossed edge.
    pub to_corpus: String,
    /// Resolved real node in `to_corpus` (the edge's `target_symbol`, or the
    /// proxy `target` with its `"<to_corpus>::"` prefix stripped), if any.
    pub to_node: Option<String>,
    /// Edge type crossed (`"calls"`, `"imports"`, …).
    pub edge_type: String,
    /// Free-form kind of the remote endpoint (e.g. `"Symbol"`), if the edge
    /// carried one.
    pub target_kind: Option<String>,
    /// Resolution-confidence band recorded on the crossed edge, if any.
    pub confidence: Option<ResolutionConfidence>,
    /// Hop index in the corpus-hop sequence (`1` = the first cross-corpus hop).
    pub corpus_depth: usize,
}

/// Result of a [`CorpusManager::federated_traverse`] call.
///
/// Bundles every intra-corpus node the traversal visited (tagged with its
/// corpus and per-corpus depth) with every cross-corpus hop it encountered. The
/// traversal is bounded — a per-corpus BFS depth cap plus a global
/// corpus-hop budget — and deterministic (outgoing edges are expanded in a
/// stable sorted order), so the same inputs always yield the same result.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FederatedTraversal {
    /// Intra-corpus nodes visited, tagged with their corpus and hop depth.
    pub nodes: Vec<FederatedNode>,
    /// Cross-corpus hops encountered (always emitted, even when continuation is
    /// off or the hop budget is spent).
    pub hops: Vec<CorpusHop>,
}

/// Manages multiple independent Engine instances, one per corpus.
///
/// Provides routing by corpus name and a default corpus for backwards compatibility.
pub struct CorpusManager {
    /// Engines keyed by corpus name.
    engines: HashMap<String, Engine>,
    /// Name of the default corpus (first one registered, or explicitly set).
    default_corpus: Option<String>,
    /// Optional callback invoked whenever a corpus is mounted.
    on_corpus_mounted: Option<Arc<dyn Fn(&str, &Path) + Send + Sync>>,
}

impl CorpusManager {
    /// Create an empty corpus manager.
    pub fn new() -> Self {
        Self { engines: HashMap::new(), default_corpus: None, on_corpus_mounted: None }
    }

    /// Add a corpus to the manager with an explicit index directory.
    pub fn add_corpus_with_index_dir(
        &mut self,
        config: CorpusConfig,
        index_dir: &Path,
    ) -> Result<()> {
        let name = config.name.clone();
        let corpus_path = PathBuf::from(&config.path);

        // Auto-bootstrap from committed SCM artifact if central cache is empty
        if let Some(scm_bundle) = crate::bundle::detect_bundle(&corpus_path) {
            if !index_dir.join("meta.db").exists() {
                let _ = crate::bundle::import_bundle(&scm_bundle, index_dir, None, None);
            }
        }

        let engine = crate::engine_builder::EngineBuilder::open(config, index_dir)?;

        if self.default_corpus.is_none() {
            self.default_corpus = Some(name.clone());
        }

        let _ = self.engines.insert(name.clone(), engine);

        if let Some(ref cb) = self.on_corpus_mounted {
            cb(&name, &corpus_path);
        }

        Ok(())
    }

    /// Add a corpus to the manager.
    ///
    /// Opens or creates the engine for the given corpus config.
    /// Index artifacts default to central storage (`${CTXV_CACHE_DIR}/corpora/<name>`).
    pub fn add_corpus(&mut self, config: CorpusConfig) -> Result<()> {
        let index_dir = ctxvault_common::config::get_corpus_index_dir(&config.name);
        self.add_corpus_with_index_dir(config, &index_dir)
    }

    /// Dynamically ensure a corpus at `corpus_path` is loaded and mounted.
    ///
    /// If an engine is already mounted for this path, returns its name.
    /// Otherwise, allocates central storage under `${CTXV_CACHE_DIR}/corpora/<name>/`
    /// (or uses local `.index` only if already present on disk), initializes the engine,
    /// and mounts it.
    pub fn ensure_corpus(&mut self, corpus_path: &Path) -> Result<String> {
        self.ensure_corpus_with_name(corpus_path, None)
    }

    /// Dynamically ensure a corpus at `corpus_path` is loaded and mounted with an optional name override.
    pub fn ensure_corpus_with_name(
        &mut self,
        corpus_path: &Path,
        name_override: Option<&str>,
    ) -> Result<String> {
        let abs_path = if corpus_path.is_absolute() {
            corpus_path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(corpus_path)
        };
        let canonical = abs_path.canonicalize().unwrap_or(abs_path);
        let canonical_str = canonical.to_string_lossy().replace('\\', "/");

        // Check if already open
        for (name, engine) in &self.engines {
            let engine_path = PathBuf::from(&engine.config().path);
            let engine_canon = engine_path.canonicalize().unwrap_or(engine_path);
            if engine_canon.to_string_lossy().replace('\\', "/") == canonical_str {
                return Ok(name.clone());
            }
        }

        // Derive name from override or directory
        let base_name = name_override.map(|s| s.to_string()).unwrap_or_else(|| {
            canonical.file_name().and_then(|n| n.to_str()).unwrap_or("corpus").to_string()
        });

        let mut name = base_name.clone();
        let mut counter = 2;
        while self.engines.contains_key(&name) {
            name = format!("{}_{}", base_name, counter);
            counter += 1;
        }

        // Index storage: always central storage in ${CTXV_CACHE_DIR}/corpora/<name>
        let local_config = canonical.join("ctxvault.toml");
        let index_dir = ctxvault_common::config::get_corpus_index_dir(&name);

        // Auto-bootstrap from committed SCM artifact if central cache is empty
        if let Some(scm_bundle) = crate::bundle::detect_bundle(&canonical) {
            if !index_dir.join("meta.db").exists() {
                let _ = crate::bundle::import_bundle(&scm_bundle, &index_dir, None, None);
            }
        }

        let config = if local_config.exists() {
            let content = std::fs::read_to_string(&local_config)?;
            let mut cfg: CorpusConfig =
                toml::from_str(&content).map_err(|e| Error::Config(e.to_string()))?;
            cfg.name = name.clone();
            cfg.path = canonical_str.clone();
            cfg
        } else {
            let global = ctxvault_common::config::load_global_config();
            CorpusConfig {
                name: name.clone(),
                path: canonical_str.clone(),
                mode: ctxvault_common::config::CorpusMode::ReadWrite,
                index_mode: global.index_mode(),
                chunking: ctxvault_common::config::ChunkingConfig::default(),
                embedding: ctxvault_common::config::EmbeddingConfig::default(),
                graph: ctxvault_common::config::GraphConfig::default(),
                templates_dir: None,
                exclude: ctxvault_common::config::ExcludeConfig::default(),
                docs: ctxvault_common::config::DocsConfig::default(),
            }
        };

        let engine = crate::engine_builder::EngineBuilder::open(config, &index_dir)?;

        if self.default_corpus.is_none() {
            self.default_corpus = Some(name.clone());
        }

        self.engines.insert(name.clone(), engine);

        if let Some(ref cb) = self.on_corpus_mounted {
            cb(&name, Path::new(&canonical_str));
        }

        Ok(name)
    }

    /// Register a callback invoked whenever a corpus is mounted.
    pub fn set_on_corpus_mounted(&mut self, callback: Arc<dyn Fn(&str, &Path) + Send + Sync>) {
        self.on_corpus_mounted = Some(callback);
    }

    /// Automatically discover and mount all cached corpora from central storage (`${CTXV_CACHE_DIR}/corpora`).
    ///
    /// Reads the stored `corpus_config` from each corpus's `meta.db` and loads its engine.
    /// Returns the names of all corpora that were successfully mounted.
    pub fn mount_all_cached_corpora(&mut self) -> Result<Vec<String>> {
        let cache_dir = ctxvault_common::config::get_corpora_cache_dir();
        let mut mounted = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let meta_db = path.join("meta.db");
                if path.is_dir() && meta_db.exists() {
                    if let Some(folder_name) = entry.file_name().to_str() {
                        if self.engines.contains_key(folder_name) {
                            continue;
                        }
                        if let Ok(store) = crate::persistence::Store::open(&meta_db) {
                            if let Ok(Some(cfg_str)) = store.get_config("corpus_config") {
                                if let Ok(mut cfg) = serde_json::from_str::<CorpusConfig>(&cfg_str)
                                {
                                    if Path::new(&cfg.path).exists() {
                                        cfg.name = folder_name.to_string();
                                        let corpus_src = PathBuf::from(&cfg.path);
                                        if let Ok(engine) =
                                            crate::engine_builder::EngineBuilder::open(cfg, &path)
                                        {
                                            if self.default_corpus.is_none() {
                                                self.default_corpus = Some(folder_name.to_string());
                                            }
                                            self.engines.insert(folder_name.to_string(), engine);
                                            mounted.push(folder_name.to_string());
                                            if let Some(ref cb) = self.on_corpus_mounted {
                                                cb(folder_name, &corpus_src);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        mounted.sort();
        Ok(mounted)
    }

    /// Retrieve the source repository path for a cached corpus from its central `meta.db` without loading the engine.
    pub fn get_cached_corpus_source_path(name: &str) -> Option<String> {
        let cache_dir = ctxvault_common::config::get_corpus_index_dir(name);
        let meta_db = cache_dir.join("meta.db");
        if meta_db.exists() {
            if let Ok(store) = crate::persistence::Store::open(&meta_db) {
                if let Ok(Some(cfg_str)) = store.get_config("corpus_config") {
                    if let Ok(cfg) = serde_json::from_str::<CorpusConfig>(&cfg_str) {
                        return Some(cfg.path);
                    }
                }
            }
        }
        None
    }

    /// Unload an open corpus from memory.
    pub fn unload_corpus(&mut self, name: &str) -> Result<bool> {
        let removed = self.engines.remove(name).is_some();
        if removed && self.default_corpus.as_deref() == Some(name) {
            self.default_corpus = self.engines.keys().next().cloned();
        }
        Ok(removed)
    }

    /// Export a mounted corpus index into a portable zstd-compressed tar bundle.
    pub fn export_corpus(
        &mut self,
        name: &str,
        out_path: &Path,
        source_commit: Option<String>,
    ) -> Result<crate::bundle::BundleManifest> {
        let engine = self.get_engine_mut(name)?;
        crate::bundle::export_bundle(engine, out_path, source_commit)
    }

    /// Import an index bundle into central storage (`${CTXV_CACHE_DIR}/corpora/<name>`) and mount it.
    pub fn import_corpus(
        &mut self,
        bundle_path: &Path,
        target_corpus_dir: &Path,
    ) -> Result<crate::bundle::BundleManifest> {
        let manifest = crate::bundle::validate_bundle(bundle_path, None, None)?;
        let target_index_dir = ctxvault_common::config::get_corpus_index_dir(&manifest.corpus_name);
        let manifest = crate::bundle::import_bundle(bundle_path, &target_index_dir, None, None)?;

        let config = CorpusConfig {
            name: manifest.corpus_name.clone(),
            path: target_corpus_dir.to_string_lossy().to_string(),
            mode: ctxvault_common::config::CorpusMode::ReadWrite,
            index_mode: ctxvault_common::config::IndexMode::Full,
            chunking: ctxvault_common::config::ChunkingConfig::default(),
            embedding: ctxvault_common::config::EmbeddingConfig {
                model: manifest.embedding_model.clone(),
            },
            graph: ctxvault_common::config::GraphConfig::default(),
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        };

        self.add_corpus_with_index_dir(config, &target_index_dir)?;
        Ok(manifest)
    }

    /// Evict engines that haven't been accessed within `_timeout`.
    pub fn evict_idle_engines(&mut self, _timeout: std::time::Duration) -> usize {
        0
    }

    /// Discover all dormant corpora in the central cache.
    pub fn discover_cached_corpora(&self) -> Vec<String> {
        let cache_dir = ctxvault_common::config::get_corpora_cache_dir();
        let mut names = Vec::new();
        if let Ok(entries) = std::fs::read_dir(cache_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() && entry.path().join("meta.db").exists() {
                    if let Some(name) = entry.file_name().to_str() {
                        names.push(name.to_string());
                    }
                }
            }
        }
        names.sort();
        names
    }

    /// Set the default corpus by name.
    pub fn set_default(&mut self, name: &str) -> Result<()> {
        if !self.engines.contains_key(name) {
            return Err(Error::NotFound(format!("corpus not found: {}", name)));
        }
        self.default_corpus = Some(name.to_string());
        Ok(())
    }

    /// Get the default corpus name.
    pub fn default_corpus_name(&self) -> Option<&str> {
        self.default_corpus.as_deref()
    }

    /// Return `(name, root_path)` pairs for all mounted corpora.
    pub fn corpus_paths(&self) -> Vec<(String, PathBuf)> {
        self.engines
            .iter()
            .map(|(name, engine)| (name.clone(), PathBuf::from(&engine.config().path)))
            .collect()
    }

    /// Get a mutable reference to an engine by corpus name.
    pub fn get_engine_mut(&mut self, name: &str) -> Result<&mut Engine> {
        self.engines
            .get_mut(name)
            .ok_or_else(|| Error::NotFound(format!("corpus not found: {}", name)))
    }

    /// Get an immutable reference to an engine by corpus name.
    pub fn get_engine(&self, name: &str) -> Result<&Engine> {
        self.engines.get(name).ok_or_else(|| Error::NotFound(format!("corpus not found: {}", name)))
    }

    /// Get a mutable reference to the default engine.
    pub fn default_engine_mut(&mut self) -> Result<&mut Engine> {
        let name = self
            .default_corpus
            .as_ref()
            .ok_or_else(|| Error::NotFound("no default corpus configured".to_string()))?
            .clone();
        self.get_engine_mut(&name)
    }

    /// Get an immutable reference to the default engine.
    pub fn default_engine(&self) -> Result<&Engine> {
        let name = self
            .default_corpus
            .as_ref()
            .ok_or_else(|| Error::NotFound("no default corpus configured".to_string()))?;
        self.get_engine(name)
    }

    /// Resolve a corpus name: if provided, use it; otherwise use default (immutable).
    pub fn resolve_engine(&self, corpus: Option<&str>) -> Result<&Engine> {
        match corpus {
            Some(name) => self.get_engine(name),
            None => self.default_engine(),
        }
    }

    /// Resolve a corpus name: if provided, use it; otherwise use default (mutable).
    pub fn resolve_engine_mut(&mut self, corpus: Option<&str>) -> Result<&mut Engine> {
        match corpus {
            Some(name) => self.get_engine_mut(name),
            None => self.default_engine_mut(),
        }
    }

    /// Incrementally synchronize a specific list of changed or deleted paths for a corpus.
    pub fn sync_delta_paths(
        &mut self,
        corpus: Option<&str>,
        paths: &[std::path::PathBuf],
    ) -> Result<crate::engine::DeltaScanResult> {
        let engine = self.resolve_engine_mut(corpus)?;
        engine.sync_delta_paths(paths)
    }

    /// List all configured corpora with their status.
    pub fn list_corpora(&self) -> Vec<CorpusInfo> {
        self.engines
            .iter()
            .map(|(name, engine)| {
                let file_count = engine.store().list_files().map(|f| f.len()).unwrap_or(0);
                let mode = format!("{:?}", engine.config().mode);

                CorpusInfo {
                    name: name.clone(),
                    path: engine.config().path.clone(),
                    mode,
                    index_mode: format!("{:?}", engine.config().index_mode),
                    file_count,
                    embedder_active: engine.embedder_active(),
                    vector_count: engine.vector_count(),
                    graph_node_count: engine.graph().node_count(),
                }
            })
            .collect()
    }

    /// Number of corpora managed.
    pub fn corpus_count(&self) -> usize {
        self.engines.len()
    }

    /// Check if a corpus exists by name.
    pub fn has_corpus(&self, name: &str) -> bool {
        self.engines.contains_key(name)
    }

    /// Get all corpus names.
    pub fn corpus_names(&self) -> Vec<&str> {
        self.engines.keys().map(|s| s.as_str()).collect()
    }

    // ─── Cross-corpus symbol linking ─────────────────────────────────────────

    /// Resolve a fully qualified symbol name across every managed corpus.
    ///
    /// Queries each engine's store for an exact `scope_path` match and returns
    /// `(corpus_name, symbol)` for every match found across all corpora. An empty
    /// result means the name is unknown; more than one result means the name is
    /// ambiguous and must NOT be linked.
    pub fn resolve_symbol_across_corpora(&self, qualified_name: &str) -> Vec<(String, CodeSymbol)> {
        let mut matches = Vec::new();
        for (corpus_name, engine) in &self.engines {
            match engine.store().find_symbols_by_qualified_name(qualified_name) {
                Ok(symbols) => {
                    for sym in symbols {
                        matches.push((corpus_name.clone(), sym));
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        corpus = %corpus_name,
                        qualified_name,
                        error = %e,
                        "cross-corpus symbol lookup failed"
                    );
                }
            }
        }
        matches
    }

    /// Resolve `raw_target` (a caller's unresolved call/import target) to a
    /// unique symbol in a DIFFERENT corpus, walking the resolver trust ladder:
    /// SCIP monikers first, then qualified-name matching. Returns the resolving
    /// [`ResolverKind`] so the emitted cross edge can be confidence-banded.
    ///
    /// A tier "wins" only on a *unique cross match*: exactly one candidate whose
    /// corpus differs from `source_corpus` (mirroring the ambiguity gate used by
    /// the doc-linking pass). The first tier that yields such a match wins;
    /// otherwise resolution falls through to the next tier, and `None` is
    /// returned when no tier yields exactly one cross match.
    ///
    /// SCIP absence is not an error (invariant I5): a corpus with no ingested
    /// SCIP monikers simply contributes no SCIP candidates, so the SCIP tier
    /// finds nothing and control falls through to qualified-name matching.
    ///
    /// The live ladder is SCIP → qualified-name. [`ResolverKind::HybridLsp`] is
    /// reserved for when in-engine LSP-grade symbol data becomes queryable
    /// across corpora; it is intentionally *not* a live tier today because the
    /// hybrid-LSP type environment is per-file and carries no cross-corpus symbol
    /// table, so a runtime `HybridLsp` tier would be a do-nothing stub.
    /// Build an in-memory SCIP moniker index: leaf identifier -> list of (corpus_name, moniker).
    ///
    /// Scans graph nodes across all engines ONCE instead of rescanning for every
    /// candidate reference, keeping resolution bounded and preventing quadratic allocations.
    fn build_scip_index(&self) -> std::collections::HashMap<String, Vec<(String, String)>> {
        let mut scip_by_leaf: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();
        for (corpus_name, engine) in &self.engines {
            for node in engine.graph().node_paths() {
                if crate::graph::scip::looks_like_moniker(&node) {
                    if let Some(leaf) = crate::graph::scip::moniker_leaf(&node) {
                        scip_by_leaf.entry(leaf).or_default().push((corpus_name.clone(), node));
                    }
                }
            }
        }
        scip_by_leaf
    }

    fn resolve_ref_via_scip_index(
        scip_index: &std::collections::HashMap<String, Vec<(String, String)>>,
        source_corpus: &str,
        leaf: &str,
    ) -> Option<(String, CodeSymbol)> {
        let candidates = scip_index.get(leaf)?;
        let mut matches = candidates.iter().filter(|(c, _)| c != source_corpus);
        let (target_corpus, moniker) = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        let symbol = CodeSymbol {
            file_path: String::new(),
            name: leaf.to_string(),
            scope_path: moniker.clone(),
            symbol_type: CodeSymbolType::Function,
            language: String::new(),
            signature: String::new(),
            docstring: None,
            start_line: 0,
            end_line: 0,
        };
        Some((target_corpus.clone(), symbol))
    }

    fn resolve_ref_with_scip_index(
        &self,
        scip_index: &std::collections::HashMap<String, Vec<(String, String)>>,
        source_corpus: &str,
        raw_target: &str,
    ) -> Option<(String, CodeSymbol, ResolverKind)> {
        // Clean the raw target to its leaf identifier, mirroring the code
        // extractor's `resolve_callee` cleaning (last segment after "::" then ".").
        let leaf = raw_target.rsplit("::").next().unwrap_or(raw_target);
        let leaf = leaf.rsplit('.').next().unwrap_or(leaf);

        // Tier 1 (highest trust): SCIP monikers via pre-indexed lookup.
        if let Some((corpus, symbol)) =
            Self::resolve_ref_via_scip_index(scip_index, source_corpus, leaf)
        {
            return Some((corpus, symbol, ResolverKind::Scip));
        }

        // Tier 2 (fallback): qualified-name matching (Phase 3 path).
        // Try the raw target directly first (preserves qualified infra resource names
        // like "aws_s3_bucket.b"), falling back to the leaf identifier.
        let mut resolved = self.resolve_symbol_across_corpora(raw_target);
        if resolved.len() != 1 && raw_target != leaf {
            resolved = self.resolve_symbol_across_corpora(leaf);
        }
        if resolved.len() == 1 {
            let (target_corpus, symbol) = &resolved[0];
            if target_corpus != source_corpus {
                return Some((target_corpus.clone(), symbol.clone(), ResolverKind::QualName));
            }
        }

        None
    }

    /// Resolve `raw_target` (a caller's unresolved call/import target) to a
    /// unique symbol in a DIFFERENT corpus, walking the resolver trust ladder.
    pub fn resolve_ref_across_corpora(
        &self,
        source_corpus: &str,
        raw_target: &str,
    ) -> Option<(String, CodeSymbol, ResolverKind)> {
        let scip_index = self.build_scip_index();
        self.resolve_ref_with_scip_index(&scip_index, source_corpus, raw_target)
    }

    /// Post-index linking pass that injects cross-corpus doc→code edges.
    ///
    /// For each corpus (the "doc side"), every document's outgoing
    /// frontmatter-provenance edge targets are treated as candidate doc→code
    /// links. A candidate is linked only when it:
    ///
    /// 1. does NOT already resolve to a node within the same corpus's graph, and
    /// 2. resolves to EXACTLY ONE `(corpus, symbol)` across all corpora.
    ///
    /// When both hold, an edge is injected into the doc's corpus graph pointing at
    /// a distinct cross-corpus node keyed `"<corpus>::<scope_path>"`, tagged with
    /// [`EdgeProvenance::DocumentsCode`], `target_corpus`, and
    /// [`ResolutionConfidence::High`]. Ambiguous (>1) or unresolved (0) candidates
    /// produce no edge, so no false or dangling edges are ever created.
    ///
    /// The pass is idempotent: re-running relies on the graph's same-type edge
    /// de-duplication, so repeated invocations neither duplicate edges nor grow
    /// the graph unbounded. Returns the number of cross-corpus edges created.
    pub fn link_cross_corpus_symbols(&mut self) -> Result<usize> {
        // Phase 1: gather link decisions using immutable access (no borrow conflict).
        // Each decision: (doc_corpus, doc_path, edge_type, target_corpus, node_key, title).
        struct CrossLink {
            doc_corpus: String,
            doc_path: String,
            edge_type: String,
            target_corpus: String,
            node_key: String,
            title: Option<String>,
            /// Repo-relative path of the resolved symbol in `target_corpus`.
            target_path: String,
            /// Fully qualified scope path of the resolved symbol.
            target_symbol: String,
        }

        let mut decisions: Vec<CrossLink> = Vec::new();

        for (doc_corpus, engine) in &self.engines {
            let graph = engine.graph();
            for doc_path in graph.node_paths() {
                for (edge_type, raw_target) in graph.outgoing_frontmatter_targets(&doc_path) {
                    // Skip candidates that already resolve within the same corpus.
                    if graph.contains_node(&raw_target)
                        && raw_target != doc_path
                        && Self::is_intra_corpus_symbol(engine, &raw_target)
                    {
                        continue;
                    }

                    let resolved = self.resolve_symbol_across_corpora(&raw_target);
                    // Only unambiguous, single, cross-corpus matches are linked.
                    if resolved.len() != 1 {
                        continue;
                    }
                    let (target_corpus, symbol) = &resolved[0];
                    // Must be a DIFFERENT corpus (intra-corpus already handled by key match).
                    if target_corpus == doc_corpus {
                        continue;
                    }

                    let node_key = format!("{}::{}", target_corpus, symbol.scope_path);
                    decisions.push(CrossLink {
                        doc_corpus: doc_corpus.clone(),
                        doc_path: doc_path.clone(),
                        edge_type,
                        target_corpus: target_corpus.clone(),
                        node_key,
                        title: Some(symbol.name.clone()),
                        target_path: symbol.file_path.clone(),
                        target_symbol: symbol.scope_path.clone(),
                    });
                }
            }
        }

        // Phase 2: apply decisions with mutable access to each doc corpus graph.
        let mut created = 0usize;
        for link in decisions {
            let engine = self.get_engine_mut(&link.doc_corpus)?;
            let graph = engine.graph_mut();
            graph.add_node(&link.node_key, link.title.as_deref());
            graph.add_cross_corpus_edge(
                &link.doc_path,
                &link.node_key,
                &link.edge_type,
                1.0,
                EdgeProvenance::DocumentsCode,
                EdgeClass::Structural,
                Some(link.target_corpus),
                Some(ResolutionConfidence::High),
                Some(link.target_path),
                Some(link.target_symbol),
                Some("Symbol".to_string()),
            );
            created += 1;
        }

        Ok(created)
    }

    /// Post-index linking pass that resolves code [`ExternalRef`](ctxvault_common::types::ExternalRef)s across corpora.
    ///
    /// Where [`Self::link_cross_corpus_symbols`] links the *doc* side
    /// (frontmatter-provenance targets), this pass links the *code* side. Every
    /// corpus captures, at index time, the call/import targets that failed to
    /// resolve against its own symbol index as
    /// [`ExternalRef`](ctxvault_common::types::ExternalRef)s. For each such
    /// reference this pass:
    ///
    /// 1. cleans `raw_target` to its leaf identifier (the last segment after `::`
    ///    then after `.`, matching the code extractor's `resolve_callee` cleaning),
    /// 2. walks the resolver trust ladder via
    ///    [`Self::resolve_ref_across_corpora`] — SCIP monikers first, then
    ///    qualified-name matching as the always-available fallback — recording
    ///    which [`ResolverKind`] tier fired, and
    /// 3. links it only when a tier yields EXACTLY ONE `(corpus, symbol)` in a
    ///    DIFFERENT corpus.
    ///
    /// The shared ambiguity gate is identical to the doc pass: `0` matches
    /// (unresolved) or `> 1` matches (ambiguous) produce no edge, so no false or
    /// dangling cross-corpus edges are ever created. A unique cross-corpus match
    /// is a genuine resolution and therefore carries
    /// [`ResolutionConfidence::High`] — the `Speculative` confidence recorded on
    /// the local `ExternalRef` reflects only that the target was *locally*
    /// unresolved and is never emitted as a hard cross edge on its own.
    ///
    /// # Emitted edges (bidirectional)
    ///
    /// For a resolved reference the pass emits two edges so a federated traversal
    /// can hop across the seam from either side:
    ///
    /// * **Forward** (into the SOURCE corpus graph): `caller_scope_path` →
    ///   `"<target_corpus>::<scope_path>"`. Edge type `calls`
    ///   ([`ExternalRefKind::Call`]) or `imports` ([`ExternalRefKind::Import`]);
    ///   provenance [`EdgeProvenance::CodeCalls`] / [`EdgeProvenance::CodeImports`];
    ///   class [`EdgeClass::Code`]; `target_corpus` = target, `target_path` = the
    ///   resolved symbol's file, `target_symbol` = its `scope_path`,
    ///   `target_kind` = `"Symbol"`, `confidence` = `High`.
    /// * **Reverse** (into the TARGET corpus graph): the resolved symbol's real
    ///   node `scope_path` → the proxy `"<source_corpus>::<caller_scope_path>"`.
    ///   Same edge type/provenance/class/confidence; `target_corpus` = the source
    ///   corpus, `target_symbol` = `caller_scope_path`, `target_kind` = `"Symbol"`.
    ///   `target_path` is `None` because the caller's file is not carried on an
    ///   `ExternalRef`.
    ///
    /// # Idempotency
    ///
    /// The pass relies solely on [`crate::graph::KnowledgeGraph::add_cross_corpus_edge`]'s
    /// same-type in-place de-duplication (the exact mechanism the doc pass uses):
    /// re-emitting the same `(source, target, edge_type)` updates the edge in
    /// place rather than adding a parallel one, so the cross-edge count is stable
    /// across re-runs. No explicit delete-rebuild is performed, so intra-repo
    /// edges (which this pass never touches) can never be removed (invariant I2).
    ///
    /// Mutates the in-memory graphs only; persistence is the caller's
    /// responsibility, matching [`Self::link_cross_corpus_symbols`]. Returns the
    /// number of forward cross-corpus edges created/updated (reverse edges are
    /// mirrored 1:1).
    pub fn resolve_external_refs(&mut self) -> Result<usize> {
        // Phase 1: gather link decisions using immutable access (no borrow conflict).
        struct CrossRef {
            /// Corpus whose ExternalRef this is (owns the forward edge source).
            source_corpus: String,
            /// Scope path of the caller/importer node in the source graph.
            caller_scope_path: String,
            /// Kind of reference (drives edge type + provenance).
            kind: ExternalRefKind,
            /// Corpus the target symbol was uniquely resolved in.
            target_corpus: String,
            /// The uniquely resolved target symbol.
            symbol: CodeSymbol,
            /// Resolver tier that produced the match (drives edge confidence).
            resolver: ResolverKind,
        }

        let mut decisions: Vec<CrossRef> = Vec::new();
        let scip_index = self.build_scip_index();

        for (source_corpus, engine) in &self.engines {
            let refs = engine.store().get_external_refs()?;
            // Cache resolution per raw_target string within this source corpus to avoid
            // redundant cross-corpus lookups and repeated SQLite queries on identical targets.
            let mut memo: std::collections::HashMap<
                String,
                Option<(String, CodeSymbol, ResolverKind)>,
            > = std::collections::HashMap::new();

            for ext in refs {
                let resolved = match memo.get(&ext.raw_target) {
                    Some(res) => res.clone(),
                    None => {
                        let res = self.resolve_ref_with_scip_index(
                            &scip_index,
                            source_corpus,
                            &ext.raw_target,
                        );
                        memo.insert(ext.raw_target.clone(), res.clone());
                        res
                    }
                };

                let Some((target_corpus, symbol, resolver)) = resolved else {
                    continue;
                };

                decisions.push(CrossRef {
                    source_corpus: source_corpus.clone(),
                    caller_scope_path: ext.caller_scope_path,
                    kind: ext.kind,
                    target_corpus,
                    symbol,
                    resolver,
                });
            }
        }

        // Phase 2: apply decisions with mutable access to each corpus graph.
        let mut created = 0usize;
        for dec in decisions {
            // Only `Call`/`Import` decisions are pushed above; imports map to the
            // `imports` edge, every other (call) kind to `calls`.
            let (edge_type, provenance) = match dec.kind {
                ExternalRefKind::Import => ("imports", EdgeProvenance::CodeImports),
                _ => ("calls", EdgeProvenance::CodeCalls),
            };

            // Map the resolving tier to the emitted edge's confidence. SCIP is
            // compiler-grade; a unique qualified-name cross match is also treated
            // as High (consistent with the Phase-3 doc pass, where a unique cross
            // match is a genuine resolution). HybridLsp is reserved (not a live
            // tier) but would likewise be compiler/type-grade High.
            let confidence = match dec.resolver {
                ResolverKind::Scip | ResolverKind::HybridLsp | ResolverKind::QualName => {
                    ResolutionConfidence::High
                }
            };

            // Infra resources (HCL/Terraform, Bicep) emit target_kind "Resource";
            // code symbols emit "Symbol".
            let target_kind = if dec.symbol.language == "hcl" || dec.symbol.language == "bicep" {
                "Resource".to_string()
            } else {
                "Symbol".to_string()
            };

            // Forward edge: caller (source corpus) -> "<target_corpus>::<scope_path>".
            let forward_target = format!("{}::{}", dec.target_corpus, dec.symbol.scope_path);
            {
                let engine = self.get_engine_mut(&dec.source_corpus)?;
                let graph = engine.graph_mut();
                graph.add_node(&dec.caller_scope_path, None);
                graph.add_node(&forward_target, Some(&dec.symbol.name));
                graph.add_cross_corpus_edge(
                    &dec.caller_scope_path,
                    &forward_target,
                    edge_type,
                    1.0,
                    provenance.clone(),
                    EdgeClass::Code,
                    Some(dec.target_corpus.clone()),
                    Some(confidence),
                    Some(dec.symbol.file_path.clone()),
                    Some(dec.symbol.scope_path.clone()),
                    Some(target_kind.clone()),
                );
            }

            // Reverse edge: resolved symbol (target corpus) -> proxy caller node.
            let reverse_target = format!("{}::{}", dec.source_corpus, dec.caller_scope_path);
            {
                let engine = self.get_engine_mut(&dec.target_corpus)?;
                let graph = engine.graph_mut();
                graph.add_node(&dec.symbol.scope_path, Some(&dec.symbol.name));
                graph.add_node(&reverse_target, None);
                graph.add_cross_corpus_edge(
                    &dec.symbol.scope_path,
                    &reverse_target,
                    edge_type,
                    1.0,
                    provenance,
                    EdgeClass::Code,
                    Some(dec.source_corpus.clone()),
                    Some(confidence),
                    None,
                    Some(dec.caller_scope_path.clone()),
                    Some(target_kind),
                );
            }

            created += 1;
        }

        Ok(created)
    }

    /// Federated (cross-corpus continuation) traversal.
    ///
    /// Runs a breadth-first traversal that starts at `start_node` in
    /// `start_corpus` and, whenever it reaches a cross-corpus edge
    /// (`target_corpus.is_some()`), records a [`CorpusHop`]. When
    /// `continue_across` is set and the hop budget still has room, it resolves
    /// the real node on the far side and continues the BFS *live* inside the
    /// target corpus's graph via the [`GraphStore`] port.
    ///
    /// # Crossing the boundary
    ///
    /// A cross-corpus edge points at a *proxy* node `"<to_corpus>::<remote_key>"`
    /// that lives in the origin graph as a stub. The real node in `to_corpus` is
    /// the edge's `target_symbol` (the remote scope_path / route / channel key);
    /// when that is absent we fall back to stripping the `"<to_corpus>::"` prefix
    /// off the proxy `target`. Continuation only happens when the resolved node
    /// actually exists in `to_corpus` (checked with
    /// [`GraphStore::contains_node`]); otherwise the hop is still recorded but no
    /// continuation node is enqueued.
    ///
    /// # Bounded work (invariant I3)
    ///
    /// Two caps bound the traversal:
    ///
    /// * `per_corpus_depth` — the maximum BFS depth explored *within* any single
    ///   corpus (reset to `0` each time the traversal enters a new corpus).
    /// * `max_corpus_hops` — the global budget on how many cross-corpus hops may
    ///   be *followed*. A hop is always *recorded*, but it is only *entered*
    ///   while `corpus_hops_used < max_corpus_hops`.
    ///
    /// A `visited: HashSet<(corpus, node)>` guard prevents revisiting a node in a
    /// corpus, so cycles (including the reverse mirror edges Phase 3/5 emit) can
    /// never loop. Total work is therefore bounded by
    /// `max_corpus_hops` corpus-entries × a `per_corpus_depth`-capped BFS each.
    ///
    /// # Determinism
    ///
    /// Graph edge / `HashMap` iteration order is not stable, so the outgoing
    /// edges of every node are sorted by `(edge_type, target)` before expansion
    /// and the work queue is FIFO. This makes both the visited set and the point
    /// at which the hop budget cuts off the traversal deterministic.
    ///
    /// Returns [`Error::NotFound`] when `start_corpus` is not mounted. A missing
    /// `start_node` yields an empty traversal (no nodes, no hops).
    pub fn federated_traverse(
        &self,
        start_corpus: &str,
        start_node: &str,
        per_corpus_depth: usize,
        max_corpus_hops: usize,
        continue_across: bool,
    ) -> Result<FederatedTraversal> {
        use std::collections::{HashSet, VecDeque};

        // Fail fast if the origin corpus is not mounted.
        let start_engine = self.get_engine(start_corpus)?;

        let mut result = FederatedTraversal::default();

        // A missing start node is not an error — it simply has nothing to walk.
        if !start_engine.graph().contains_node(start_node) {
            return Ok(result);
        }

        // Cycle guard across corpora: (corpus, node) already enqueued.
        let mut visited: HashSet<(String, String)> = HashSet::new();
        // Work items: (corpus, node, intra_depth, corpus_hops_used).
        let mut queue: VecDeque<(String, String, usize, usize)> = VecDeque::new();

        let _ = visited.insert((start_corpus.to_string(), start_node.to_string()));
        queue.push_back((start_corpus.to_string(), start_node.to_string(), 0, 0));
        result.nodes.push(FederatedNode {
            corpus: start_corpus.to_string(),
            node: start_node.to_string(),
            depth: 0,
        });

        while let Some((corpus, node, intra_depth, corpus_hops_used)) = queue.pop_front() {
            // The corpus is guaranteed mounted: the origin was checked up front
            // and continuation only enqueues mounted corpora.
            let Ok(engine) = self.get_engine(&corpus) else {
                continue;
            };

            // Deterministic expansion order (edge/HashMap order is unstable).
            let mut edges = engine.graph().outgoing_edges(&node);
            edges.sort_by(|a, b| {
                a.edge_type.cmp(&b.edge_type).then_with(|| a.target.cmp(&b.target))
            });

            for edge in edges {
                match &edge.target_corpus {
                    // ── Intra-corpus edge: continue BFS within this corpus. ──
                    None => {
                        if intra_depth >= per_corpus_depth {
                            continue;
                        }
                        let key = (corpus.clone(), edge.target.clone());
                        if visited.contains(&key) {
                            continue;
                        }
                        let _ = visited.insert(key);
                        result.nodes.push(FederatedNode {
                            corpus: corpus.clone(),
                            node: edge.target.clone(),
                            depth: intra_depth + 1,
                        });
                        queue.push_back((
                            corpus.clone(),
                            edge.target.clone(),
                            intra_depth + 1,
                            corpus_hops_used,
                        ));
                    }
                    // ── Cross-corpus edge: always record a hop; maybe continue. ──
                    Some(to_corpus) => {
                        // Resolve the real node on the far side: prefer the
                        // carried target_symbol, else strip the "<to_corpus>::"
                        // proxy prefix off the edge target.
                        let to_node = edge.target_symbol.clone().or_else(|| {
                            edge.target
                                .strip_prefix(&format!("{}::", to_corpus))
                                .map(|s| s.to_string())
                        });

                        result.hops.push(CorpusHop {
                            from_corpus: corpus.clone(),
                            from_node: node.clone(),
                            to_corpus: to_corpus.clone(),
                            to_node: to_node.clone(),
                            edge_type: edge.edge_type.clone(),
                            target_kind: edge.target_kind.clone(),
                            confidence: edge.confidence,
                            corpus_depth: corpus_hops_used + 1,
                        });

                        // Continue live into the target corpus, budget permitting.
                        if !continue_across || corpus_hops_used >= max_corpus_hops {
                            continue;
                        }
                        let Some(to_node) = to_node else {
                            continue;
                        };
                        let Ok(to_engine) = self.get_engine(to_corpus) else {
                            continue;
                        };
                        if !to_engine.graph().contains_node(&to_node) {
                            continue;
                        }
                        let key = (to_corpus.clone(), to_node.clone());
                        if visited.contains(&key) {
                            continue;
                        }
                        let _ = visited.insert(key);
                        result.nodes.push(FederatedNode {
                            corpus: to_corpus.clone(),
                            node: to_node.clone(),
                            depth: 0,
                        });
                        // Reset intra-corpus depth in the newly entered corpus.
                        queue.push_back((to_corpus.clone(), to_node, 0, corpus_hops_used + 1));
                    }
                }
            }
        }

        Ok(result)
    }

    /// Whether a graph node keyed by `scope_path` corresponds to a code symbol
    /// defined in this engine's own corpus (as opposed to a bare doc target that
    /// merely happens to share the string).
    fn is_intra_corpus_symbol(engine: &Engine, scope_path: &str) -> bool {
        engine
            .store()
            .find_symbols_by_qualified_name(scope_path)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }
}

impl Default for CorpusManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxvault_common::config::{
        ChunkingConfig, CorpusMode, EmbeddingConfig, GraphConfig, IndexMode,
    };
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn test_config(name: &str, corpus_path: &Path) -> CorpusConfig {
        let _ = fs::create_dir_all(corpus_path.join(".index"));
        CorpusConfig {
            name: name.to_string(),
            path: corpus_path.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Full,
            chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig { edge_types: Vec::new() },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        }
    }

    fn add_test_corpus(manager: &mut CorpusManager, config: CorpusConfig) {
        let index_dir = PathBuf::from(&config.path).join(".index");
        manager.add_corpus_with_index_dir(config, &index_dir).unwrap();
    }

    #[test]
    fn test_create_empty_manager() {
        let manager = CorpusManager::new();
        assert_eq!(manager.corpus_count(), 0);
        assert!(manager.default_corpus_name().is_none());
    }

    #[test]
    fn test_add_corpus_sets_default() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("wiki");
        fs::create_dir_all(&corpus_dir).unwrap();

        let mut manager = CorpusManager::new();
        let config = test_config("wiki", &corpus_dir);
        add_test_corpus(&mut manager, config);

        assert_eq!(manager.corpus_count(), 1);
        assert_eq!(manager.default_corpus_name(), Some("wiki"));
        assert!(manager.has_corpus("wiki"));
    }

    #[test]
    fn test_multiple_corpora_isolation() {
        let tmp = TempDir::new().unwrap();
        let wiki_dir = tmp.path().join("wiki");
        let docs_dir = tmp.path().join("docs");
        fs::create_dir_all(&wiki_dir).unwrap();
        fs::create_dir_all(&docs_dir).unwrap();

        let mut manager = CorpusManager::new();
        add_test_corpus(&mut manager, test_config("wiki", &wiki_dir));
        add_test_corpus(&mut manager, test_config("docs", &docs_dir));

        assert_eq!(manager.corpus_count(), 2);

        // Index a file in "wiki".
        {
            let wiki_engine = manager.get_engine_mut("wiki").unwrap();
            wiki_engine.index_file("test.md", "# Wiki Note\n\nContent for wiki.\n").unwrap();
            wiki_engine.commit().unwrap();
        }

        // Index a different file in "docs".
        {
            let docs_engine = manager.get_engine_mut("docs").unwrap();
            docs_engine.index_file("guide.md", "# Guide\n\nDocumentation guide.\n").unwrap();
            docs_engine.commit().unwrap();
        }

        // Wiki should have test.md but not guide.md.
        let wiki_engine = manager.get_engine("wiki").unwrap();
        assert!(wiki_engine.store().get_file("test.md").unwrap().is_some());
        assert!(wiki_engine.store().get_file("guide.md").unwrap().is_none());

        // Docs should have guide.md but not test.md.
        let docs_engine = manager.get_engine("docs").unwrap();
        assert!(docs_engine.store().get_file("guide.md").unwrap().is_some());
        assert!(docs_engine.store().get_file("test.md").unwrap().is_none());
    }

    #[test]
    fn test_resolve_engine_with_corpus_param() {
        let tmp = TempDir::new().unwrap();
        let wiki_dir = tmp.path().join("wiki");
        let docs_dir = tmp.path().join("docs");
        fs::create_dir_all(&wiki_dir).unwrap();
        fs::create_dir_all(&docs_dir).unwrap();

        let mut manager = CorpusManager::new();
        add_test_corpus(&mut manager, test_config("wiki", &wiki_dir));
        add_test_corpus(&mut manager, test_config("docs", &docs_dir));

        // None resolves to default (wiki, since it was added first).
        {
            let engine = manager.resolve_engine_mut(None).unwrap();
            assert_eq!(engine.config().name, "wiki");
        }

        // Explicit name resolves correctly.
        {
            let engine = manager.resolve_engine_mut(Some("docs")).unwrap();
            assert_eq!(engine.config().name, "docs");
        }

        // Non-existent corpus returns error.
        assert!(manager.resolve_engine_mut(Some("nope")).is_err());
    }

    #[test]
    fn test_list_corpora() {
        let tmp = TempDir::new().unwrap();
        let wiki_dir = tmp.path().join("wiki");
        fs::create_dir_all(&wiki_dir).unwrap();

        let mut manager = CorpusManager::new();
        add_test_corpus(&mut manager, test_config("wiki", &wiki_dir));

        let list = manager.list_corpora();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "wiki");
        assert_eq!(list[0].index_mode, "Full");
        assert_eq!(list[0].file_count, 0);
    }

    // ─── Cross-corpus symbol linking ─────────────────────────────────────────

    use ctxvault_common::config::{EdgeSource, EdgeTypeConfig};

    /// Fast-mode corpus config with a single frontmatter `implements` edge type.
    /// Fast mode skips embeddings, so no ONNX model is required.
    fn linking_config(name: &str, corpus_path: &Path) -> CorpusConfig {
        let _ = fs::create_dir_all(corpus_path.join(".index"));
        let implements = EdgeTypeConfig {
            name: "implements".to_string(),
            source: EdgeSource::Frontmatter,
            weight: 1.0,
            bidirectional: false,
            field: Some("implements".to_string()),
            direction: None,
            max_frequency: None,
            class: None,
            description: None,
            allowed_source_templates: None,
            allowed_target_templates: None,
        };
        CorpusConfig {
            name: name.to_string(),
            path: corpus_path.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Fast,
            chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig { edge_types: vec![implements] },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        }
    }

    /// Rust source defining exactly one top-level symbol with the given name.
    /// The extracted `scope_path` for a top-level function equals its bare name.
    fn rust_symbol_source(name: &str) -> String {
        format!("pub fn {name}() -> u32 {{\n    42\n}}\n")
    }

    /// Markdown doc whose frontmatter `implements` a code symbol scope_path.
    fn doc_implementing(target_scope: &str) -> String {
        format!(
            "---\nimplements: \"{target_scope}\"\n---\n\n# Design Note\n\nDescribes the impl.\n"
        )
    }

    fn add_fast_corpus(manager: &mut CorpusManager, name: &str, root: &Path) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        let config = linking_config(name, &dir);
        let index_dir = dir.join(".index");
        manager.add_corpus_with_index_dir(config, &index_dir).unwrap();
    }

    #[test]
    fn test_cross_corpus_unique_match_links() {
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp.path());
        add_fast_corpus(&mut manager, "B", tmp.path());

        // Corpus B uniquely defines a symbol `WidgetEngine`.
        {
            let b = manager.get_engine_mut("B").unwrap();
            b.index_file("src/widget.rs", &rust_symbol_source("WidgetEngine")).unwrap();
            b.commit().unwrap();
        }
        // Corpus A has a doc whose frontmatter implements that scope_path.
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("design.md", &doc_implementing("WidgetEngine")).unwrap();
            a.commit().unwrap();
        }

        let created = manager.link_cross_corpus_symbols().unwrap();
        assert_eq!(created, 1, "exactly one cross-corpus edge should be created");

        // The doc's forward links must include the cross-corpus node.
        let a = manager.get_engine("A").unwrap();
        let node_key = "B::WidgetEngine";
        assert!(a.graph().contains_node(node_key), "cross-corpus node must exist");

        let edge = a
            .graph()
            .get_all_edges()
            .into_iter()
            .find(|e| e.source == "design.md" && e.target == node_key)
            .expect("cross-corpus edge must exist");
        assert_eq!(edge.target_corpus.as_deref(), Some("B"));
        assert_eq!(edge.confidence, Some(ResolutionConfidence::High));
        assert_eq!(edge.provenance, EdgeProvenance::DocumentsCode);
        assert_eq!(edge.edge_type, "implements");

        // Idempotent: re-running creates no additional edges.
        let created_again = manager.link_cross_corpus_symbols().unwrap();
        assert_eq!(
            created_again, 1,
            "re-run resolves the same single candidate (deduped in graph)"
        );
        let a = manager.get_engine("A").unwrap();
        let dup_count = a
            .graph()
            .get_all_edges()
            .into_iter()
            .filter(|e| e.source == "design.md" && e.target == node_key)
            .count();
        assert_eq!(dup_count, 1, "no duplicate cross-corpus edge after re-run");
    }

    #[test]
    fn test_cross_corpus_ambiguous_match_no_edge() {
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp.path());
        add_fast_corpus(&mut manager, "B", tmp.path());
        add_fast_corpus(&mut manager, "C", tmp.path());

        // The SAME scope_path is defined in BOTH B and C => ambiguous.
        {
            let b = manager.get_engine_mut("B").unwrap();
            b.index_file("src/dup.rs", &rust_symbol_source("Dup")).unwrap();
            b.commit().unwrap();
        }
        {
            let c = manager.get_engine_mut("C").unwrap();
            c.index_file("src/dup.rs", &rust_symbol_source("Dup")).unwrap();
            c.commit().unwrap();
        }
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("design.md", &doc_implementing("Dup")).unwrap();
            a.commit().unwrap();
        }

        // Resolution should find two matches across corpora.
        assert_eq!(manager.resolve_symbol_across_corpora("Dup").len(), 2);

        let created = manager.link_cross_corpus_symbols().unwrap();
        assert_eq!(created, 0, "ambiguous target must not create an edge");

        let a = manager.get_engine("A").unwrap();
        assert!(!a.graph().contains_node("B::Dup"));
        assert!(!a.graph().contains_node("C::Dup"));
    }

    #[test]
    fn test_cross_corpus_unresolved_no_edge() {
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp.path());
        add_fast_corpus(&mut manager, "B", tmp.path());

        // B defines something, but the doc implements a symbol nobody defines.
        {
            let b = manager.get_engine_mut("B").unwrap();
            b.index_file("src/other.rs", &rust_symbol_source("SomethingElse")).unwrap();
            b.commit().unwrap();
        }
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("design.md", &doc_implementing("NoSuchSymbol")).unwrap();
            a.commit().unwrap();
        }

        assert!(manager.resolve_symbol_across_corpora("NoSuchSymbol").is_empty());

        let created = manager.link_cross_corpus_symbols().unwrap();
        assert_eq!(created, 0, "unresolved target must not create an edge");
    }

    #[test]
    fn test_intra_corpus_doc_to_code_edge_aligns_on_scope_path() {
        // A single corpus containing both the doc and the code symbol it implements.
        // The frontmatter edge target string equals the code symbol scope_path, so
        // build_frontmatter_edges lands the edge directly on the symbol node.
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "mono", tmp.path());

        {
            let m = manager.get_engine_mut("mono").unwrap();
            m.index_file("src/thing.rs", &rust_symbol_source("Thing")).unwrap();
            m.index_file("design.md", &doc_implementing("Thing")).unwrap();
            m.commit().unwrap();
        }

        let m = manager.get_engine("mono").unwrap();
        // The symbol node exists (from the code `defines` pass) and the doc's
        // frontmatter `implements` edge points straight at it.
        assert!(m.graph().contains_node("Thing"));
        let fwd = m.graph().forwardlinks("design.md", None);
        let implements_targets = fwd.get("implements").expect("implements edge must exist");
        assert!(
            implements_targets.iter().any(|t| t == "Thing"),
            "intra-corpus doc->code edge must land on the symbol node"
        );

        // Single corpus => cross-corpus linking is a no-op.
        let created = manager.link_cross_corpus_symbols().unwrap();
        assert_eq!(created, 0);
    }

    #[test]
    fn test_ensure_and_unload_corpus() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("dynamic_repo");
        fs::create_dir_all(&repo_dir).unwrap();
        fs::create_dir_all(repo_dir.join(".index")).unwrap();

        let mut manager = CorpusManager::new();
        let name = manager.ensure_corpus(&repo_dir).unwrap();
        assert_eq!(name, "dynamic_repo");
        assert!(manager.has_corpus("dynamic_repo"));
        assert_eq!(manager.default_corpus_name(), Some("dynamic_repo"));

        // Calling ensure_corpus again on same path returns existing name
        let name2 = manager.ensure_corpus(&repo_dir).unwrap();
        assert_eq!(name2, "dynamic_repo");
        assert_eq!(manager.corpus_count(), 1);

        // Unloading removes corpus
        assert!(manager.unload_corpus("dynamic_repo").unwrap());
        assert!(!manager.has_corpus("dynamic_repo"));
        assert_eq!(manager.corpus_count(), 0);
    }

    // ─── Cross-corpus ExternalRef resolution ─────────────────────────────────

    /// Rust source for a top-level `caller` fn that calls an out-of-corpus fn.
    /// `callee` does not resolve locally, so the extractor records an
    /// `ExternalRef` with `caller_scope_path == "caller"` and
    /// `raw_target == "<callee>"`.
    fn rust_caller_source(callee: &str) -> String {
        format!("pub fn caller() -> u32 {{\n    {callee}()\n}}\n")
    }

    /// Count cross-corpus code edges (`target_corpus.is_some()` with a code
    /// provenance) in a graph.
    fn cross_code_edge_count(engine: &Engine) -> usize {
        engine
            .graph()
            .get_all_edges()
            .into_iter()
            .filter(|e| {
                e.target_corpus.is_some()
                    && matches!(
                        e.provenance,
                        EdgeProvenance::CodeCalls | EdgeProvenance::CodeImports
                    )
            })
            .count()
    }

    #[test]
    fn test_resolve_external_refs_unique_match_links_both_graphs() {
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp.path());
        add_fast_corpus(&mut manager, "B", tmp.path());

        // B uniquely defines `targetfn`.
        {
            let b = manager.get_engine_mut("B").unwrap();
            b.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
            b.commit().unwrap();
        }
        // A has a caller that calls `targetfn` (unresolved locally => ExternalRef).
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
            a.commit().unwrap();
        }

        let created = manager.resolve_external_refs().unwrap();
        assert!(created >= 1, "a unique cross-corpus ref must create an edge");

        // Forward edge lives in A: caller -> "B::targetfn".
        let a = manager.get_engine("A").unwrap();
        let node_key = "B::targetfn";
        assert!(a.graph().contains_node(node_key));
        let fwd = a
            .graph()
            .get_all_edges()
            .into_iter()
            .find(|e| e.source == "caller" && e.target == node_key)
            .expect("forward cross-corpus edge must exist in A");
        assert_eq!(fwd.target_corpus.as_deref(), Some("B"));
        assert_eq!(fwd.target_kind.as_deref(), Some("Symbol"));
        assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));
        assert_eq!(fwd.edge_type, "calls");
        assert_eq!(fwd.provenance, EdgeProvenance::CodeCalls);

        // Reverse (mirror) edge lives in B: targetfn -> "A::caller".
        let b = manager.get_engine("B").unwrap();
        let reverse_key = "A::caller";
        assert!(b.graph().contains_node(reverse_key));
        let rev = b
            .graph()
            .get_all_edges()
            .into_iter()
            .find(|e| e.source == "targetfn" && e.target == reverse_key)
            .expect("reverse cross-corpus edge must exist in B");
        assert_eq!(rev.target_corpus.as_deref(), Some("A"));
        assert_eq!(rev.target_symbol.as_deref(), Some("caller"));
        assert_eq!(rev.confidence, Some(ResolutionConfidence::High));
        assert_eq!(rev.edge_type, "calls");
    }

    #[test]
    fn test_resolve_external_refs_ambiguous_no_edge() {
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp.path());
        add_fast_corpus(&mut manager, "B", tmp.path());
        add_fast_corpus(&mut manager, "C", tmp.path());

        // Both B and C define `targetfn` => ambiguous.
        {
            let b = manager.get_engine_mut("B").unwrap();
            b.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
            b.commit().unwrap();
        }
        {
            let c = manager.get_engine_mut("C").unwrap();
            c.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
            c.commit().unwrap();
        }
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
            a.commit().unwrap();
        }

        assert_eq!(manager.resolve_symbol_across_corpora("targetfn").len(), 2);

        let created = manager.resolve_external_refs().unwrap();
        assert_eq!(created, 0, "ambiguous cross-corpus ref must not create an edge");

        // No cross edge to any "*::targetfn" node in A.
        let a = manager.get_engine("A").unwrap();
        assert!(!a.graph().contains_node("B::targetfn"));
        assert!(!a.graph().contains_node("C::targetfn"));
        assert_eq!(cross_code_edge_count(a), 0);
    }

    #[test]
    fn test_resolve_external_refs_scip_tier_resolves_first() {
        use ctxvault_common::ports::GraphStore;
        use ctxvault_common::types::Edge;

        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp.path());
        add_fast_corpus(&mut manager, "B", tmp.path());

        // Corpus B has NO qualified-name `search` symbol in code_symbols — its
        // only knowledge of `search` is a SCIP moniker node in the graph. This
        // proves the SCIP tier (not qualified-name) resolved the ref.
        assert!(
            manager.resolve_symbol_across_corpora("search").is_empty(),
            "no qualified-name `search` symbol may exist for this test"
        );
        {
            let b = manager.get_engine_mut("B").unwrap();
            // Add a SCIP `defines` edge directly (hermetic — no .scip protobuf
            // needed): src/search.rs -> "scip-rust cargo b 0.0.1 search().",
            // which registers the moniker as a graph node.
            let moniker = "scip-rust cargo b 0.0.1 search().".to_string();
            let edge = Edge {
                source: "src/search.rs".to_string(),
                target: moniker,
                edge_type: "defines".to_string(),
                weight: 1.0,
                provenance: EdgeProvenance::CodeDefines,
                target_corpus: None,
                confidence: Some(ResolutionConfidence::High),
                target_path: None,
                target_symbol: None,
                target_kind: None,
            };
            b.graph_mut().add_code_edge(&edge);
            b.commit().unwrap();
        }
        // Corpus A calls `search` (unresolved locally => ExternalRef).
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("src/main.rs", &rust_caller_source("search")).unwrap();
            a.commit().unwrap();
        }

        let created = manager.resolve_external_refs().unwrap();
        assert!(created >= 1, "the SCIP moniker must resolve the cross-repo call");

        // Forward edge lives in A: caller -> "B::scip-rust cargo b 0.0.1 search().".
        let a = manager.get_engine("A").unwrap();
        let node_key = "B::scip-rust cargo b 0.0.1 search().";
        assert!(a.graph().contains_node(node_key), "SCIP-resolved cross node must exist");
        let fwd = a
            .graph()
            .get_all_edges()
            .into_iter()
            .find(|e| e.source == "caller" && e.target == node_key)
            .expect("forward SCIP cross-corpus edge must exist in A");
        assert_eq!(fwd.target_corpus.as_deref(), Some("B"));
        assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));
        assert_eq!(fwd.edge_type, "calls");
        // target_symbol is the moniker itself (proves SCIP tier, not qual-name).
        assert_eq!(fwd.target_symbol.as_deref(), Some("scip-rust cargo b 0.0.1 search()."));

        // The ladder must directly report the Scip tier for this ref.
        assert_eq!(
            manager.resolve_ref_across_corpora("A", "search").map(|(_, _, k)| k),
            Some(ResolverKind::Scip)
        );
    }

    #[test]
    fn test_resolve_external_refs_falls_back_to_qualname() {
        // No SCIP anywhere: B defines `targetfn` as a real CodeSymbol, so the
        // ladder must fall through the (empty) SCIP tier to qualified-name.
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp.path());
        add_fast_corpus(&mut manager, "B", tmp.path());

        {
            let b = manager.get_engine_mut("B").unwrap();
            b.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
            b.commit().unwrap();
        }
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
            a.commit().unwrap();
        }

        // The ladder reports the QualName tier (SCIP found nothing, fell through).
        assert_eq!(
            manager.resolve_ref_across_corpora("A", "targetfn").map(|(_, _, k)| k),
            Some(ResolverKind::QualName)
        );

        let created = manager.resolve_external_refs().unwrap();
        assert!(created >= 1, "qualified-name fallback must resolve the cross-repo call");

        let a = manager.get_engine("A").unwrap();
        let fwd = a
            .graph()
            .get_all_edges()
            .into_iter()
            .find(|e| e.source == "caller" && e.target == "B::targetfn")
            .expect("qual-name forward cross edge must exist");
        assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));
    }

    #[test]
    fn test_resolve_external_refs_idempotent() {
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp.path());
        add_fast_corpus(&mut manager, "B", tmp.path());

        {
            let b = manager.get_engine_mut("B").unwrap();
            b.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
            b.commit().unwrap();
        }
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
            a.commit().unwrap();
        }

        manager.resolve_external_refs().unwrap();
        let first = cross_code_edge_count(manager.get_engine("A").unwrap());
        assert!(first >= 1);

        manager.resolve_external_refs().unwrap();
        let second = cross_code_edge_count(manager.get_engine("A").unwrap());
        assert_eq!(first, second, "re-run must keep the cross-edge count stable");
    }

    /// Phase-4 acceptance: the resolver trust ladder tries SCIP monikers first
    /// and falls through to qualified-name matching when no moniker is present.
    ///
    /// Both branches share one caller shape (A's `caller` calls `targetfn`, an
    /// out-of-corpus ref) so the only variable is what corpus B knows:
    ///
    /// * SCIP branch — B's graph carries a SCIP moniker node whose
    ///   [`crate::graph::scip::moniker_leaf`] is `targetfn` but has NO
    ///   qualified-name `targetfn` symbol. Resolution must win at the SCIP tier,
    ///   observable as the forward cross edge targeting the *moniker string*
    ///   (`target_symbol == "scip-rust cargo b 0.0.1 targetfn()."`) at
    ///   [`ResolutionConfidence::High`].
    /// * QualName branch — a fresh manager where B instead defines a real
    ///   `targetfn` code symbol and carries no moniker. The (empty) SCIP tier
    ///   falls through and qualified-name matching wins, observable as the
    ///   forward cross edge targeting the *symbol's scope_path* (`targetfn`,
    ///   NOT a moniker) at [`ResolutionConfidence::High`] (invariant I5: SCIP
    ///   absence never fails resolution — it simply falls through).
    #[test]
    fn test_resolver_trust_ladder_scip_then_qualname() {
        use ctxvault_common::types::Edge;

        let moniker = "scip-rust cargo b 0.0.1 targetfn().";

        // ── SCIP branch ──────────────────────────────────────────────────────
        {
            let tmp = TempDir::new().unwrap();
            let mut manager = CorpusManager::new();
            add_fast_corpus(&mut manager, "A", tmp.path());
            add_fast_corpus(&mut manager, "B", tmp.path());

            // B's ONLY knowledge of `targetfn` is a SCIP moniker graph node
            // (added via a `defines` edge, exactly like the scip.rs ingester
            // emits). There is deliberately no qualified-name symbol.
            {
                let b = manager.get_engine_mut("B").unwrap();
                let edge = Edge {
                    source: "src/lib.rs".to_string(),
                    target: moniker.to_string(),
                    edge_type: "defines".to_string(),
                    weight: 1.0,
                    provenance: EdgeProvenance::CodeDefines,
                    target_corpus: None,
                    confidence: Some(ResolutionConfidence::High),
                    target_path: None,
                    target_symbol: None,
                    target_kind: None,
                };
                b.graph_mut().add_code_edge(&edge);
                b.commit().unwrap();
            }
            assert!(
                manager.resolve_symbol_across_corpora("targetfn").is_empty(),
                "SCIP branch must have no qualified-name `targetfn` symbol"
            );
            {
                let a = manager.get_engine_mut("A").unwrap();
                a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
                a.commit().unwrap();
            }

            let created = manager.resolve_external_refs().unwrap();
            assert!(created >= 1, "SCIP moniker must resolve the cross-repo call");

            // Observable proof SCIP won: the forward edge targets the MONIKER.
            let a = manager.get_engine("A").unwrap();
            let node_key = format!("B::{moniker}");
            assert!(a.graph().contains_node(&node_key), "SCIP-resolved cross node must exist");
            let fwd = a
                .graph()
                .get_all_edges()
                .into_iter()
                .find(|e| e.source == "caller" && e.target == node_key)
                .expect("forward SCIP cross edge must exist in A");
            assert_eq!(fwd.target_symbol.as_deref(), Some(moniker), "SCIP tier => moniker target");
            assert_eq!(fwd.target_corpus.as_deref(), Some("B"));
            assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));
            assert_eq!(fwd.edge_type, "calls");
        }

        // ── QualName fallback branch ─────────────────────────────────────────
        {
            let tmp = TempDir::new().unwrap();
            let mut manager = CorpusManager::new();
            add_fast_corpus(&mut manager, "A", tmp.path());
            add_fast_corpus(&mut manager, "B", tmp.path());

            // No moniker anywhere: B defines a REAL `targetfn` code symbol. The
            // empty SCIP tier falls through to qualified-name matching.
            {
                let b = manager.get_engine_mut("B").unwrap();
                b.index_file("src/lib.rs", &rust_symbol_source("targetfn")).unwrap();
                b.commit().unwrap();
            }
            {
                let a = manager.get_engine_mut("A").unwrap();
                a.index_file("src/main.rs", &rust_caller_source("targetfn")).unwrap();
                a.commit().unwrap();
            }

            let created = manager.resolve_external_refs().unwrap();
            assert!(created >= 1, "qualified-name fallback must resolve the cross-repo call");

            // Observable proof qual-name won: the forward edge targets the
            // symbol's scope_path (`targetfn`), NOT a moniker.
            let a = manager.get_engine("A").unwrap();
            let node_key = "B::targetfn";
            assert!(a.graph().contains_node(node_key), "qual-name cross node must exist");
            let fwd = a
                .graph()
                .get_all_edges()
                .into_iter()
                .find(|e| e.source == "caller" && e.target == node_key)
                .expect("forward qual-name cross edge must exist in A");
            assert_eq!(
                fwd.target_symbol.as_deref(),
                Some("targetfn"),
                "qual-name tier => scope_path"
            );
            assert_ne!(
                fwd.target_symbol.as_deref(),
                Some(moniker),
                "qual-name target must not be a moniker"
            );
            assert_eq!(fwd.target_corpus.as_deref(), Some("B"));
            assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));
            assert_eq!(fwd.edge_type, "calls");
        }
    }

    // ─── Phase 5: infra resource matching (grammar-pure) ─────────────────────

    #[test]
    fn test_resolve_external_refs_links_infra_resource_bidirectional() {
        let tmp = TempDir::new().unwrap();
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "service_b", tmp.path());
        add_fast_corpus(&mut manager, "infra_d", tmp.path());

        // Infra corpus D defines an HCL resource:
        // `resource "aws_s3_bucket" "b" { bucket = "my-bucket" }`
        // which extracts as CodeSymbol name="aws_s3_bucket.b", scope_path="aws_s3_bucket.b".
        {
            let d = manager.get_engine_mut("infra_d").unwrap();
            let tf_content = "resource \"aws_s3_bucket\" \"b\" {\n  bucket = \"my-bucket\"\n}\n";
            d.index_file("main.tf", tf_content).unwrap();
            d.commit().unwrap();
        }

        // Service corpus B calls/references `aws_s3_bucket.b` (unresolved locally -> ExternalRef).
        {
            let b = manager.get_engine_mut("service_b").unwrap();
            let caller_src = "pub fn upload() {\n    aws_s3_bucket.b();\n}\n";
            b.index_file("src/upload.rs", caller_src).unwrap();
            b.commit().unwrap();
        }

        let created = manager.resolve_external_refs().unwrap();
        assert!(created >= 1, "unresolved reference to infra resource must create a cross edge");

        // Forward edge in service_b: upload -> "infra_d::aws_s3_bucket.b" with target_kind="Resource"
        let b = manager.get_engine("service_b").unwrap();
        let node_key = "infra_d::aws_s3_bucket.b";
        assert!(b.graph().contains_node(node_key), "proxy node for infra resource must exist");
        let fwd = b
            .graph()
            .get_all_edges()
            .into_iter()
            .find(|e| e.source == "upload" && e.target == node_key)
            .expect("forward cross edge to infra resource must exist");
        assert_eq!(fwd.target_corpus.as_deref(), Some("infra_d"));
        assert_eq!(fwd.target_symbol.as_deref(), Some("aws_s3_bucket.b"));
        assert_eq!(fwd.target_kind.as_deref(), Some("Resource"));
        assert_eq!(fwd.confidence, Some(ResolutionConfidence::High));

        // Reverse mirror edge in infra_d: aws_s3_bucket.b -> "service_b::upload" with target_kind="Resource"
        let d = manager.get_engine("infra_d").unwrap();
        let rev_target = "service_b::upload";
        assert!(
            d.graph().contains_node(rev_target),
            "reverse proxy caller node must exist in infra_d"
        );
        let rev = d
            .graph()
            .get_all_edges()
            .into_iter()
            .find(|e| e.source == "aws_s3_bucket.b" && e.target == rev_target)
            .expect("reverse mirror edge from infra resource must exist");
        assert_eq!(rev.target_corpus.as_deref(), Some("service_b"));
        assert_eq!(rev.target_symbol.as_deref(), Some("upload"));
        assert_eq!(rev.target_kind.as_deref(), Some("Resource"));
        assert_eq!(rev.confidence, Some(ResolutionConfidence::High));
    }

    // ─── Phase 6: federated (cross-corpus continuation) traversal ────────────

    /// Rust source for a named top-level fn that calls an out-of-corpus fn.
    /// The extractor records the caller's `scope_path == caller_name` and an
    /// `ExternalRef` `raw_target == "<callee>"` (unresolved locally).
    fn rust_named_caller_source(caller_name: &str, callee: &str) -> String {
        format!("pub fn {caller_name}() -> u32 {{\n    {callee}()\n}}\n")
    }

    /// Build a linked A->B->C code-call chain via `resolve_external_refs`:
    /// C defines `leaf`; B's `mid` calls `leaf`; A's `top` calls `mid`. After
    /// resolution, A holds a forward `top -> "B::mid"` cross edge and B holds a
    /// forward `mid -> "C::leaf"` cross edge (plus reverse mirrors).
    fn build_abc_chain(tmp: &Path) -> CorpusManager {
        let mut manager = CorpusManager::new();
        add_fast_corpus(&mut manager, "A", tmp);
        add_fast_corpus(&mut manager, "B", tmp);
        add_fast_corpus(&mut manager, "C", tmp);

        {
            let c = manager.get_engine_mut("C").unwrap();
            c.index_file("src/leaf.rs", &rust_symbol_source("leaf")).unwrap();
            c.commit().unwrap();
        }
        {
            let b = manager.get_engine_mut("B").unwrap();
            b.index_file("src/mid.rs", &rust_named_caller_source("mid", "leaf")).unwrap();
            b.commit().unwrap();
        }
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("src/top.rs", &rust_named_caller_source("top", "mid")).unwrap();
            a.commit().unwrap();
        }

        let created = manager.resolve_external_refs().unwrap();
        assert!(created >= 2, "both A->B and B->C cross edges must be created");
        manager
    }

    #[test]
    fn test_federated_traverse_continues_across_abc_chain() {
        let tmp = TempDir::new().unwrap();
        let manager = build_abc_chain(tmp.path());

        let result = manager
            .federated_traverse("A", "top", 4, 3, true)
            .expect("federated traversal from a mounted corpus must succeed");

        // Hops name B then C, in order (deterministic).
        let hop_corpora: Vec<&str> = result.hops.iter().map(|h| h.to_corpus.as_str()).collect();
        assert!(
            hop_corpora.contains(&"B") && hop_corpora.contains(&"C"),
            "hops must name both target corpora B and C: {hop_corpora:?}"
        );
        let first_b = hop_corpora.iter().position(|c| *c == "B");
        let first_c = hop_corpora.iter().position(|c| *c == "C");
        assert!(first_b < first_c, "the B hop must precede the C hop: {hop_corpora:?}");

        // The first hop is the A->B cross with corpus_depth 1.
        let ab = result.hops.iter().find(|h| h.to_corpus == "B").unwrap();
        assert_eq!(ab.from_corpus, "A");
        assert_eq!(ab.to_node.as_deref(), Some("mid"));
        assert_eq!(ab.edge_type, "calls");
        assert_eq!(ab.corpus_depth, 1);

        // Live continuation reached real nodes in BOTH B and C.
        assert!(
            result.nodes.iter().any(|n| n.corpus == "B" && n.node == "mid"),
            "continuation must reach B's `mid` node"
        );
        assert!(
            result.nodes.iter().any(|n| n.corpus == "C" && n.node == "leaf"),
            "continuation must reach C's `leaf` node"
        );
    }

    #[test]
    fn test_federated_traverse_hop_budget_caps_deterministically() {
        let tmp = TempDir::new().unwrap();
        let manager = build_abc_chain(tmp.path());

        // Budget of 1 corpus hop: enter B, but NEVER continue into C.
        let result = manager.federated_traverse("A", "top", 4, 1, true).unwrap();

        assert!(
            result.nodes.iter().any(|n| n.corpus == "B" && n.node == "mid"),
            "a budget of 1 still enters B"
        );
        assert!(
            result.nodes.iter().all(|n| n.corpus != "C"),
            "a corpus-hop budget of 1 must NOT enqueue any C node: {:?}",
            result.nodes
        );

        // Determinism: repeated calls yield identical node/hop sets.
        let again = manager.federated_traverse("A", "top", 4, 1, true).unwrap();
        let nodes_one: Vec<(String, String)> =
            result.nodes.iter().map(|n| (n.corpus.clone(), n.node.clone())).collect();
        let nodes_two: Vec<(String, String)> =
            again.nodes.iter().map(|n| (n.corpus.clone(), n.node.clone())).collect();
        assert_eq!(nodes_one, nodes_two, "traversal order + result must be deterministic");
    }

    #[test]
    fn test_federated_traverse_records_hops_without_continuing() {
        let tmp = TempDir::new().unwrap();
        let manager = build_abc_chain(tmp.path());

        // continue_across = false: hop RECORDS are still emitted, but no
        // cross-corpus node is ever entered (only A-tagged nodes appear).
        let result = manager.federated_traverse("A", "top", 4, 3, false).unwrap();

        assert!(
            result.hops.iter().any(|h| h.to_corpus == "B"),
            "the A->B hop must still be recorded when continuation is off"
        );
        assert!(
            result.nodes.iter().all(|n| n.corpus == "A"),
            "with continuation off, no cross-corpus node may be entered: {:?}",
            result.nodes
        );
    }

    #[test]
    fn test_federated_traverse_missing_corpus_and_node() {
        let tmp = TempDir::new().unwrap();
        let manager = build_abc_chain(tmp.path());

        // Unmounted origin corpus => NotFound error.
        assert!(manager.federated_traverse("ZZ", "top", 4, 3, true).is_err());

        // Missing start node => empty (non-error) traversal.
        let empty = manager.federated_traverse("A", "no_such_node", 4, 3, true).unwrap();
        assert!(empty.nodes.is_empty() && empty.hops.is_empty());
    }
}
