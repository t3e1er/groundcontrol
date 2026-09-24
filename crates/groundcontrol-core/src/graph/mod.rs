//! Knowledge graph: typed directed edges, traversal, PPR, subgraph extraction.

/// Graph analysis, broken links, circular dependencies, and orphan detection.
pub mod analysis;
/// Document edge builder (wikilinks, tags, frontmatter).
pub mod builder;
/// Polyglot code AST relationship extraction.
pub mod code;
/// Community detection algorithms (Louvain & Leiden).
pub mod communities;
/// Personalized PageRank and graph diffusion.
pub mod diffusion;
/// Hybrid LSP and type resolution heuristics.
pub mod hybrid_lsp;
/// Graph serialization and deserialization.
pub mod persistence;
/// Graph queries and Cypher-Lite matching.
pub mod query;
/// SCIP index integration and code intelligence.
pub mod scip;
/// BFS/DFS graph traversals and lineage tracing.
pub mod traversal;
/// Core graph node and edge types.
pub mod types;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use groundcontrol_common::config::{EdgeClass, EdgeTypeConfig};
use groundcontrol_common::types::{
    BrokenLink, CircularDependency, CommunityDensity, CommunityDetectionResult, Document,
    EdgeProvenance, GraphStats, LineageNode, OrphanAdr, ResolutionConfidence,
};
use groundcontrol_common::Result;

pub use types::{GraphEdge, GraphNode, GRAPH_SCHEMA_VERSION};

/// Knowledge graph with typed, weighted, directed edges.
pub struct KnowledgeGraph {
    graph: DiGraph<GraphNode, GraphEdge>,
    /// Map from document path to NodeIndex for O(1) lookup.
    node_map: HashMap<String, NodeIndex>,
}

impl KnowledgeGraph {
    /// Create an empty knowledge graph.
    pub fn new() -> Self {
        Self { graph: DiGraph::new(), node_map: HashMap::new() }
    }

    /// Add or update a node. Returns the NodeIndex.
    pub fn add_node(&mut self, path: &str, title: Option<&str>) -> NodeIndex {
        if let Some(&idx) = self.node_map.get(path) {
            // Update existing node data.
            if let Some(node) = self.graph.node_weight_mut(idx) {
                node.title = title.map(|t| t.to_string());
            }
            idx
        } else {
            let node = GraphNode { path: path.to_string(), title: title.map(|t| t.to_string()) };
            let idx = self.graph.add_node(node);
            let _ = self.node_map.insert(path.to_string(), idx);
            idx
        }
    }

    /// Remove a node and all its edges.
    pub fn remove_node(&mut self, path: &str) -> Result<()> {
        let idx = self.node_map.remove(path).ok_or_else(|| {
            groundcontrol_common::Error::Graph(format!("node not found: {}", path))
        })?;
        let _ = self.graph.remove_node(idx);

        // petgraph may swap the last node into the removed index.
        // Rebuild node_map to stay consistent.
        self.rebuild_node_map();
        Ok(())
    }

    /// Add a directed intra-corpus edge between two nodes. Creates target node if missing.
    pub fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        class: EdgeClass,
    ) {
        self.add_edge_full(source, target, edge_type, weight, provenance, class, None, None);
    }

    /// Add a directed edge carrying optional cross-corpus resolution metadata.
    pub fn add_edge_full(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        class: EdgeClass,
        target_corpus: Option<String>,
        confidence: Option<ResolutionConfidence>,
    ) {
        self.insert_or_update_edge(
            source,
            target,
            edge_type,
            weight,
            provenance,
            class,
            target_corpus,
            confidence,
            None,
            None,
            None,
        );
    }

    /// Add a cross-corpus edge carrying the full remote-endpoint payload.
    pub fn add_cross_corpus_edge(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        class: EdgeClass,
        target_corpus: Option<String>,
        confidence: Option<ResolutionConfidence>,
        target_path: Option<String>,
        target_symbol: Option<String>,
        target_kind: Option<String>,
    ) {
        self.insert_or_update_edge(
            source,
            target,
            edge_type,
            weight,
            provenance,
            class,
            target_corpus,
            confidence,
            target_path,
            target_symbol,
            target_kind,
        );
    }

    /// Insert a new edge or update the existing same-type edge in place.
    fn insert_or_update_edge(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        class: EdgeClass,
        target_corpus: Option<String>,
        confidence: Option<ResolutionConfidence>,
        target_path: Option<String>,
        target_symbol: Option<String>,
        target_kind: Option<String>,
    ) {
        let src_idx = self.add_node(source, None);
        let tgt_idx = self.add_node(target, None);

        // De-duplicate parallel edges of the same type.
        if let Some(edge_ref) = self
            .graph
            .edges_connecting(src_idx, tgt_idx)
            .find(|e| e.weight().edge_type == edge_type)
        {
            let edge_id = edge_ref.id();
            if let Some(edge_mut) = self.graph.edge_weight_mut(edge_id) {
                edge_mut.weight = weight;
                edge_mut.provenance = provenance;
                edge_mut.class = class;
                edge_mut.target_corpus = target_corpus;
                edge_mut.confidence = confidence;
                edge_mut.target_path = target_path;
                edge_mut.target_symbol = target_symbol;
                edge_mut.target_kind = target_kind;
            }
            return;
        }

        let edge = GraphEdge {
            edge_type: edge_type.to_string(),
            weight,
            provenance,
            class,
            target_corpus,
            confidence,
            target_path,
            target_symbol,
            target_kind,
        };
        let _ = self.graph.add_edge(src_idx, tgt_idx, edge);
    }

    /// Add a code edge into the graph with appropriate EdgeClass.
    pub fn add_code_edge(&mut self, edge: &groundcontrol_common::types::Edge) {
        self.add_cross_corpus_edge(
            &edge.source,
            &edge.target,
            &edge.edge_type,
            edge.weight,
            edge.provenance.clone(),
            EdgeClass::Code,
            edge.target_corpus.clone(),
            edge.confidence,
            edge.target_path.clone(),
            edge.target_symbol.clone(),
            edge.target_kind.clone(),
        );
    }

    /// Remove all edges where the given path is source or target.
    pub fn remove_edges_for_node(&mut self, path: &str) {
        let Some(&idx) = self.node_map.get(path) else {
            return;
        };

        let edge_indices: Vec<_> = self
            .graph
            .edges_directed(idx, Direction::Outgoing)
            .map(|e| e.id())
            .chain(self.graph.edges_directed(idx, Direction::Incoming).map(|e| e.id()))
            .collect();

        for edge_id in edge_indices.into_iter().rev() {
            let _ = self.graph.remove_edge(edge_id);
        }
    }

    /// Get the NodeIndex for a path.
    pub fn get_node(&self, path: &str) -> Option<NodeIndex> {
        self.node_map.get(path).copied()
    }

    /// Access the internal node map mapping paths to NodeIndex.
    pub fn node_map(&self) -> &HashMap<String, NodeIndex> {
        &self.node_map
    }

    /// Access the underlying directed Petgraph instance.
    pub fn inner_graph(&self) -> &DiGraph<GraphNode, GraphEdge> {
        &self.graph
    }

    /// Whether a node with the given path exists in the graph.
    pub fn contains_node(&self, path: &str) -> bool {
        self.node_map.contains_key(path)
    }

    /// Enumerate a node's outgoing frontmatter-provenance edges as
    /// `(edge_type, raw_target)` pairs.
    pub fn outgoing_frontmatter_targets(&self, path: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let Some(&idx) = self.node_map.get(path) else {
            return out;
        };
        for edge in self.graph.edges_directed(idx, Direction::Outgoing) {
            let edge_data = edge.weight();
            if edge_data.provenance != EdgeProvenance::Frontmatter {
                continue;
            }
            if let Some(target_node) = self.graph.node_weight(edge.target()) {
                out.push((edge_data.edge_type.clone(), target_node.path.clone()));
            }
        }
        out
    }

    /// Enumerate all node paths currently in the graph.
    pub fn node_paths(&self) -> Vec<String> {
        self.node_map.keys().cloned().collect()
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of edges.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Retrieve all edges currently in the knowledge graph.
    pub fn get_all_edges(&self) -> Vec<groundcontrol_common::types::Edge> {
        let mut edges = Vec::new();
        for edge in self.graph.edge_references() {
            let source_idx = edge.source();
            let target_idx = edge.target();
            if let (Some(source_node), Some(target_node)) =
                (self.graph.node_weight(source_idx), self.graph.node_weight(target_idx))
            {
                let weight_data = edge.weight();
                edges.push(groundcontrol_common::types::Edge {
                    source: source_node.path.clone(),
                    target: target_node.path.clone(),
                    edge_type: weight_data.edge_type.clone(),
                    weight: weight_data.weight,
                    provenance: weight_data.provenance.clone(),
                    target_corpus: weight_data.target_corpus.clone(),
                    confidence: weight_data.confidence,
                    target_path: weight_data.target_path.clone(),
                    target_symbol: weight_data.target_symbol.clone(),
                    target_kind: weight_data.target_kind.clone(),
                });
            }
        }
        edges
    }

    /// Retrieve a single node's outgoing edges, carrying the full cross-corpus payload.
    pub fn outgoing_edges(&self, path: &str) -> Vec<groundcontrol_common::types::Edge> {
        let mut edges = Vec::new();
        let Some(&idx) = self.node_map.get(path) else {
            return edges;
        };
        for edge in self.graph.edges_directed(idx, Direction::Outgoing) {
            let target_idx = edge.target();
            if let (Some(source_node), Some(target_node)) =
                (self.graph.node_weight(idx), self.graph.node_weight(target_idx))
            {
                let weight_data = edge.weight();
                edges.push(groundcontrol_common::types::Edge {
                    source: source_node.path.clone(),
                    target: target_node.path.clone(),
                    edge_type: weight_data.edge_type.clone(),
                    weight: weight_data.weight,
                    provenance: weight_data.provenance.clone(),
                    target_corpus: weight_data.target_corpus.clone(),
                    confidence: weight_data.confidence,
                    target_path: weight_data.target_path.clone(),
                    target_symbol: weight_data.target_symbol.clone(),
                    target_kind: weight_data.target_kind.clone(),
                });
            }
        }
        edges
    }

    /// Retrieve all edges as relational EdgeRecords for SQLite persistence.
    pub fn get_all_edge_records(&self) -> Vec<groundcontrol_common::types::EdgeRecord> {
        let mut edges = Vec::new();
        for edge in self.graph.edge_references() {
            let source_idx = edge.source();
            let target_idx = edge.target();
            if let (Some(source_node), Some(target_node)) =
                (self.graph.node_weight(source_idx), self.graph.node_weight(target_idx))
            {
                let weight_data = edge.weight();
                edges.push(groundcontrol_common::types::EdgeRecord {
                    id: None,
                    source: source_node.path.clone(),
                    target: target_node.path.clone(),
                    edge_type: weight_data.edge_type.clone(),
                    edge_class: weight_data.class.as_str().to_string(),
                    weight: weight_data.weight,
                    confidence: 1.0,
                    metadata: None,
                });
            }
        }
        edges
    }

    /// Return all active distinct edge types present in the graph, optionally filtered by EdgeClass.
    pub fn active_edge_types(&self, class_filter: Option<EdgeClass>) -> Vec<String> {
        let mut types = std::collections::BTreeSet::new();
        for edge in self.graph.edge_weights() {
            if let Some(cf) = class_filter {
                if edge.class == cf {
                    types.insert(edge.edge_type.clone());
                }
            } else {
                types.insert(edge.edge_type.clone());
            }
        }
        types.into_iter().collect()
    }

    /// Access the underlying Petgraph `DiGraph`.
    pub fn inner(&self) -> &DiGraph<GraphNode, GraphEdge> {
        &self.graph
    }

    /// Ensure a node exists (add it if not present). Used for testing.
    pub fn ensure_node(&mut self, path: &str) {
        let _ = self.add_node(path, None);
    }

    /// Rebuild node_map from the graph.
    fn rebuild_node_map(&mut self) {
        self.node_map.clear();
        for idx in self.graph.node_indices() {
            if let Some(node) = self.graph.node_weight(idx) {
                let _ = self.node_map.insert(node.path.clone(), idx);
            }
        }
    }
}

impl Default for KnowledgeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl groundcontrol_common::ports::GraphStore for KnowledgeGraph {
    fn add_node(&mut self, path: &str, title: Option<&str>) {
        let _ = KnowledgeGraph::add_node(self, path, title);
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        class: EdgeClass,
    ) {
        KnowledgeGraph::add_edge(self, source, target, edge_type, weight, provenance, class)
    }

    fn add_edge_full(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        class: EdgeClass,
        target_corpus: Option<String>,
        confidence: Option<ResolutionConfidence>,
    ) {
        KnowledgeGraph::add_edge_full(
            self,
            source,
            target,
            edge_type,
            weight,
            provenance,
            class,
            target_corpus,
            confidence,
        )
    }

    fn add_cross_corpus_edge(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        class: EdgeClass,
        target_corpus: Option<String>,
        confidence: Option<ResolutionConfidence>,
        target_path: Option<String>,
        target_symbol: Option<String>,
        target_kind: Option<String>,
    ) {
        KnowledgeGraph::add_cross_corpus_edge(
            self,
            source,
            target,
            edge_type,
            weight,
            provenance,
            class,
            target_corpus,
            confidence,
            target_path,
            target_symbol,
            target_kind,
        )
    }

    fn add_code_edge(&mut self, edge: &groundcontrol_common::types::Edge) {
        KnowledgeGraph::add_code_edge(self, edge)
    }

    fn remove_node(&mut self, path: &str) -> Result<()> {
        KnowledgeGraph::remove_node(self, path)
    }

    fn remove_edges_for_node(&mut self, path: &str) {
        KnowledgeGraph::remove_edges_for_node(self, path)
    }

    fn contains_node(&self, path: &str) -> bool {
        KnowledgeGraph::contains_node(self, path)
    }

    fn outgoing_frontmatter_targets(&self, path: &str) -> Vec<(String, String)> {
        KnowledgeGraph::outgoing_frontmatter_targets(self, path)
    }

    fn node_paths(&self) -> Vec<String> {
        KnowledgeGraph::node_paths(self)
    }

    fn node_count(&self) -> usize {
        KnowledgeGraph::node_count(self)
    }

    fn edge_count(&self) -> usize {
        KnowledgeGraph::edge_count(self)
    }

    fn get_all_edges(&self) -> Vec<groundcontrol_common::types::Edge> {
        KnowledgeGraph::get_all_edges(self)
    }

    fn outgoing_edges(&self, path: &str) -> Vec<groundcontrol_common::types::Edge> {
        KnowledgeGraph::outgoing_edges(self, path)
    }

    fn build_edges_for_document(
        &mut self,
        doc: &Document,
        edge_configs: &[EdgeTypeConfig],
        all_docs: &[Document],
    ) {
        KnowledgeGraph::build_edges_for_document(self, doc, edge_configs, all_docs)
    }

    fn build_all_tag_edges(&mut self, configs: &[EdgeTypeConfig], all_docs: &[Document]) {
        KnowledgeGraph::build_all_tag_edges(self, configs, all_docs)
    }

    fn traverse_bfs(
        &self,
        start: &str,
        max_depth: usize,
        edge_type_filter: Option<&[String]>,
        edge_class_filter: Option<EdgeClass>,
    ) -> Vec<(String, usize)> {
        KnowledgeGraph::traverse_bfs(self, start, max_depth, edge_type_filter, edge_class_filter)
    }

    fn backlinks(
        &self,
        path: &str,
        edge_class_filter: Option<EdgeClass>,
    ) -> HashMap<String, Vec<String>> {
        KnowledgeGraph::backlinks(self, path, edge_class_filter)
    }

    fn forwardlinks(
        &self,
        path: &str,
        edge_class_filter: Option<EdgeClass>,
    ) -> HashMap<String, Vec<String>> {
        KnowledgeGraph::forwardlinks(self, path, edge_class_filter)
    }

    fn shortest_path(
        &self,
        from: &str,
        to: &str,
        edge_type_filter: Option<&[String]>,
        edge_class_filter: Option<EdgeClass>,
    ) -> Option<Vec<String>> {
        KnowledgeGraph::shortest_path(self, from, to, edge_type_filter, edge_class_filter)
    }

    fn stats(&self) -> GraphStats {
        KnowledgeGraph::stats(self)
    }

    fn compute_affordances(&self, path: &str) -> groundcontrol_common::types::GraphAffordances {
        KnowledgeGraph::compute_affordances(self, path)
    }

    fn in_degree(&self, path: &str) -> usize {
        KnowledgeGraph::in_degree(self, path)
    }

    fn traverse_lineage(
        &self,
        start: &str,
        edge_type: &str,
        direction: &str,
        max_depth: usize,
    ) -> Vec<LineageNode> {
        KnowledgeGraph::traverse_lineage(self, start, edge_type, direction, max_depth)
    }

    fn extract_lineage_for_node(
        &self,
        path: &str,
    ) -> Option<groundcontrol_common::types::LineageAnnotation> {
        KnowledgeGraph::extract_lineage_for_node(self, path)
    }

    fn detect_broken_links(&self, existing_paths: &HashSet<String>) -> Vec<BrokenLink> {
        KnowledgeGraph::detect_broken_links(self, existing_paths)
    }

    fn detect_circular_dependencies(&self, edge_types: &[&str]) -> Vec<CircularDependency> {
        KnowledgeGraph::detect_circular_dependencies(self, edge_types)
    }

    fn detect_orphan_adrs(&self, adr_paths: &[String]) -> Vec<OrphanAdr> {
        KnowledgeGraph::detect_orphan_adrs(self, adr_paths)
    }

    fn detect_communities(&self) -> CommunityDetectionResult {
        KnowledgeGraph::detect_communities(self)
    }

    fn detect_communities_leiden(&self) -> CommunityDetectionResult {
        KnowledgeGraph::detect_communities_leiden(self)
    }

    fn community_densities(&self) -> Vec<CommunityDensity> {
        KnowledgeGraph::community_densities(self)
    }

    fn save(&self, path: &Path) -> Result<()> {
        KnowledgeGraph::save(self, path)
    }
}
