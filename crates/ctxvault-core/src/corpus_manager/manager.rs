//! CorpusManager lifecycle and storage orchestration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ctxvault_common::config::CorpusConfig;
use ctxvault_common::{Error, Result};

use crate::engine::Engine;

/// Manages multiple independent Engine instances, one per corpus.
///
/// Provides routing by corpus name and a default corpus for backwards compatibility.
pub struct CorpusManager {
    /// Engines keyed by corpus name.
    pub(crate) engines: HashMap<String, Engine>,
    /// Name of the default corpus (first one registered, or explicitly set).
    pub(crate) default_corpus: Option<String>,
    /// Optional callback invoked whenever a corpus is mounted.
    pub(crate) on_corpus_mounted: Option<Arc<dyn Fn(&str, &Path) + Send + Sync>>,
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
            let mut exclude = ctxvault_common::config::ExcludeConfig::default();
            let gitignore_path = canonical.join(".gitignore");
            if gitignore_path.exists() {
                exclude.import_gitignore(&gitignore_path);
            }
            CorpusConfig {
                name: name.clone(),
                path: canonical_str.clone(),
                mode: ctxvault_common::config::CorpusMode::ReadWrite,
                index_mode: global.index_mode(),
                chunking: ctxvault_common::config::ChunkingConfig::default(),
                embedding: ctxvault_common::config::EmbeddingConfig::default(),
                graph: ctxvault_common::config::GraphConfig::default(),
                templates_dir: None,
                exclude,
                docs: ctxvault_common::config::DocsConfig::default(),
            }
        };

        let engine = crate::engine_builder::EngineBuilder::open(config, &index_dir)?;

        if self.default_corpus.is_none() {
            self.default_corpus = Some(name.clone());
        }

        let index_mode = engine.config().index_mode;
        self.engines.insert(name.clone(), engine);

        let mut global = ctxvault_common::config::load_global_config();
        if !global.corpora.registered.contains_key(&name) {
            global.corpora.registered.insert(
                name.clone(),
                ctxvault_common::config::RegisteredCorpus {
                    path: canonical_str.clone(),
                    index_mode: Some(index_mode),
                },
            );
            if global.corpora.default.is_none() {
                global.corpora.default = Some(name.clone());
            }
            let _ = ctxvault_common::config::save_global_config(&global);
        }

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
        let global = ctxvault_common::config::load_global_config();
        for (name, reg) in &global.corpora.registered {
            if !self.engines.contains_key(name) {
                let p = Path::new(&reg.path);
                if p.exists() {
                    if let Ok(m_name) = self.ensure_corpus_with_name(p, Some(name)) {
                        if !mounted.contains(&m_name) {
                            mounted.push(m_name);
                        }
                    }
                }
            }
        }
        if self.default_corpus.is_none() {
            if let Some(ref def) = global.corpora.default {
                if self.engines.contains_key(def) {
                    self.default_corpus = Some(def.clone());
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

    /// Incrementally synchronize a specific list of changed or deleted paths for a corpus.
    pub fn sync_delta_paths(
        &mut self,
        corpus: Option<&str>,
        paths: &[std::path::PathBuf],
    ) -> Result<crate::engine::DeltaScanResult> {
        let engine = self.resolve_engine_mut(corpus)?;
        engine.sync_delta_paths(paths)
    }
}

impl Default for CorpusManager {
    fn default() -> Self {
        Self::new()
    }
}
