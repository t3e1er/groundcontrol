//! Structural knowledge graph and HippoRAG PPR retrieval algorithm component.

use std::path::{Path, PathBuf};

use groundcontrol_common::ports::RetrievalAlgorithm;
use groundcontrol_common::types::{EntityKind, Modality, ParsedArtifact, SearchResult};
use groundcontrol_common::Result;

use crate::graph::KnowledgeGraph;

#[cfg(test)]
pub mod tests;
pub mod types;

pub use types::GraphConfig;

/// Graph retrieval algorithm backed by Petgraph knowledge graph and HippoRAG PPR diffusion.
pub struct GraphAlgorithm {
    graph: KnowledgeGraph,
    config: GraphConfig,
    index_path: Option<PathBuf>,
}

impl GraphAlgorithm {
    /// Create a new graph algorithm wrapping an existing `KnowledgeGraph`.
    pub fn new(graph: KnowledgeGraph) -> Self {
        Self { graph, config: GraphConfig::default(), index_path: None }
    }

    /// Access the underlying `KnowledgeGraph`.
    pub fn graph(&self) -> &KnowledgeGraph {
        &self.graph
    }

    /// Access the mutable underlying `KnowledgeGraph`.
    pub fn graph_mut(&mut self) -> &mut KnowledgeGraph {
        &mut self.graph
    }

    /// Add an AST code edge directly to the underlying knowledge graph.
    pub fn add_code_edge(&mut self, edge: &groundcontrol_common::types::Edge) {
        self.graph.add_code_edge(edge);
    }

    /// Build tag edges across all documents.
    pub fn build_all_tag_edges(
        &mut self,
        tag_configs: &[groundcontrol_common::config::EdgeTypeConfig],
        docs: &[groundcontrol_common::types::Document],
    ) {
        self.graph.build_all_tag_edges(tag_configs, docs);
    }

    /// Clear all nodes and edges from the graph.
    pub fn clear(&mut self) -> Result<()> {
        RetrievalAlgorithm::clear(self)
    }

    /// Commit the graph state to disk.
    pub fn commit(&mut self) -> Result<()> {
        RetrievalAlgorithm::commit(self)
    }
}

impl std::ops::Deref for GraphAlgorithm {
    type Target = KnowledgeGraph;

    fn deref(&self) -> &Self::Target {
        &self.graph
    }
}

impl std::ops::DerefMut for GraphAlgorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.graph
    }
}

impl RetrievalAlgorithm for GraphAlgorithm {
    fn name(&self) -> &'static str {
        "ppr"
    }

    fn open(&mut self, index_dir: &Path) -> Result<()> {
        let bin_path = index_dir.join("graph.bin");
        self.index_path = Some(bin_path.clone());
        if bin_path.exists() {
            self.graph = KnowledgeGraph::load(&bin_path)?;
        }
        Ok(())
    }

    fn index_document(&mut self, doc: &ParsedArtifact) -> Result<()> {
        self.graph.remove_edges_for_node(&doc.path);
        if let Some(ref md) = doc.doc_metadata {
            self.graph.build_edges_for_document(md, &[], &[]);
        }
        for edge in &doc.graph_edges {
            self.graph.add_code_edge(edge);
        }
        Ok(())
    }

    fn remove_document(&mut self, path: &str) -> Result<()> {
        let _ = self.graph.remove_node(path);
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        if let Some(ref path) = self.index_path {
            self.graph.save(path)?;
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        self.graph = KnowledgeGraph::new();
        Ok(())
    }

    fn search(&self, query: &str, limit: usize, modality: Modality) -> Result<Vec<SearchResult>> {
        let query_lower = query.to_lowercase();
        let terms: Vec<&str> = query_lower.split_whitespace().collect();

        let mut seeds = Vec::new();
        for node in self.graph.node_map().keys() {
            let n_lower = node.to_lowercase();
            let matches = terms.iter().any(|t| n_lower.contains(t));
            if matches {
                seeds.push((node.clone(), 1.0));
            }
        }

        if seeds.is_empty() {
            return Ok(Vec::new());
        }

        let ppr_scores = crate::graph::diffusion::personalized_pagerank(
            &self.graph,
            &seeds,
            self.config.alpha,
            self.config.iterations,
            None,
        );

        let mut results = Vec::new();
        for item in ppr_scores.into_iter().take(limit) {
            let mut path = item.path.as_str();
            let mut symbol = None;
            if let Some(idx) = path.find('#') {
                symbol = Some(path[idx + 1..].to_string());
                path = &path[..idx];
            }

            let is_code = path.ends_with(".rs")
                || path.ends_with(".ts")
                || path.ends_with(".js")
                || path.ends_with(".py")
                || path.ends_with(".go")
                || path.ends_with(".java")
                || path.ends_with(".cpp")
                || path.ends_with(".c");

            if modality == Modality::Code && !is_code {
                continue;
            }
            if modality == Modality::Docs && is_code {
                continue;
            }

            let mut res = SearchResult::new(path, item.score).with_symbol(symbol);
            if is_code {
                res = res.with_entity_kind(EntityKind::CodeFile { language: String::new() });
            } else {
                res = res.with_entity_kind(EntityKind::Documentation);
            }
            results.push(res);
        }

        Ok(results)
    }
}
