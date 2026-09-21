use std::path::PathBuf;

use groundcontrol_common::ports::{GraphStore, MetadataCatalog};
use groundcontrol_common::{Error, Result};

use super::manager::CorpusManager;
use super::types::CorpusInfo;
use crate::engine::Engine;

impl CorpusManager {
    /// Set the default corpus.
    pub fn set_default(&mut self, name: &str) -> Result<()> {
        if !self.engines.contains_key(name) {
            return Err(Error::NotFound(format!("corpus '{}' not found", name)));
        }
        self.default_corpus = Some(name.to_string());
        Ok(())
    }

    /// Get the name of the default corpus.
    pub fn default_corpus_name(&self) -> Option<&str> {
        self.default_corpus.as_deref()
    }

    /// Get all (corpus_name, path) pairs.
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
            .ok_or_else(|| Error::NotFound(format!("corpus '{}' not found", name)))
    }

    /// Get a reference to an engine by corpus name.
    pub fn get_engine(&self, name: &str) -> Result<&Engine> {
        self.engines
            .get(name)
            .ok_or_else(|| Error::NotFound(format!("corpus '{}' not found", name)))
    }

    /// Get a mutable reference to the default engine.
    pub fn default_engine_mut(&mut self) -> Result<&mut Engine> {
        let name = self
            .default_corpus
            .clone()
            .ok_or_else(|| Error::NotFound("no default corpus configured".to_string()))?;
        self.get_engine_mut(&name)
    }

    /// Get a reference to the default engine.
    pub fn default_engine(&self) -> Result<&Engine> {
        let name = self
            .default_corpus
            .as_deref()
            .ok_or_else(|| Error::NotFound("no default corpus configured".to_string()))?;
        self.get_engine(name)
    }

    /// Resolve an engine: if `corpus` is `Some(name)`, use that corpus;
    /// otherwise fall back to the default corpus.
    pub fn resolve_engine(&self, corpus: Option<&str>) -> Result<&Engine> {
        match corpus {
            Some(name) => self.get_engine(name),
            None => self.default_engine(),
        }
    }

    /// Resolve a mutable engine: if `corpus` is `Some(name)`, use that corpus;
    /// otherwise fall back to the default corpus.
    pub fn resolve_engine_mut(&mut self, corpus: Option<&str>) -> Result<&mut Engine> {
        match corpus {
            Some(name) => self.get_engine_mut(name),
            None => self.default_engine_mut(),
        }
    }

    /// Get status information for all registered corpora.
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

    /// Get the number of registered corpora.
    pub fn corpus_count(&self) -> usize {
        self.engines.len()
    }

    /// Check if a corpus is registered.
    pub fn has_corpus(&self, name: &str) -> bool {
        self.engines.contains_key(name)
    }

    /// Get all corpus names.
    pub fn corpus_names(&self) -> Vec<&str> {
        self.engines.keys().map(|s| s.as_str()).collect()
    }
}
