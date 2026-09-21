use std::collections::{HashMap, HashSet};

use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use groundcontrol_common::config::EdgeClass;
use groundcontrol_common::types::{BrokenLink, CircularDependency, GraphStats, OrphanAdr};

use super::KnowledgeGraph;

impl KnowledgeGraph {
    /// Get graph statistics.
    pub fn stats(&self) -> GraphStats {
        let node_count = self.graph.node_count();
        let edge_count = self.graph.edge_count();

        let mut orphan_count = 0;
        let mut degree_map: Vec<(String, usize)> = Vec::new();

        for idx in self.graph.node_indices() {
            let in_degree = self.graph.edges_directed(idx, Direction::Incoming).count();
            let out_degree = self.graph.edges_directed(idx, Direction::Outgoing).count();
            let total_degree = in_degree + out_degree;

            if total_degree == 0 {
                orphan_count += 1;
            }

            if let Some(node) = self.graph.node_weight(idx) {
                degree_map.push((node.path.clone(), total_degree));
            }
        }

        degree_map.sort_by(|a, b| b.1.cmp(&a.1));
        let most_connected: Vec<(String, usize)> = degree_map.into_iter().take(10).collect();

        let mut edge_type_distribution: HashMap<String, usize> = HashMap::new();
        for edge in self.graph.edge_weights() {
            *edge_type_distribution.entry(edge.edge_type.clone()).or_insert(0) += 1;
        }

        GraphStats { node_count, edge_count, orphan_count, most_connected, edge_type_distribution }
    }

    /// Get paths of all orphan nodes (nodes with no incoming or outgoing edges).
    pub fn orphan_paths(&self) -> Vec<String> {
        let mut orphans = Vec::new();
        for idx in self.graph.node_indices() {
            let in_degree = self.graph.edges_directed(idx, Direction::Incoming).count();
            let out_degree = self.graph.edges_directed(idx, Direction::Outgoing).count();
            if in_degree + out_degree == 0 {
                if let Some(node) = self.graph.node_weight(idx) {
                    orphans.push(node.path.clone());
                }
            }
        }
        orphans
    }

    /// Get per-node degree information: (path, in_degree, out_degree).
    pub fn node_degree_list(&self) -> Vec<(String, usize, usize)> {
        let mut degrees = Vec::new();
        for idx in self.graph.node_indices() {
            let in_degree = self.graph.edges_directed(idx, Direction::Incoming).count();
            let out_degree = self.graph.edges_directed(idx, Direction::Outgoing).count();
            if let Some(node) = self.graph.node_weight(idx) {
                degrees.push((node.path.clone(), in_degree, out_degree));
            }
        }
        degrees
    }

    /// Detect broken structural links (wikilinks or frontmatter references pointing to non-existent notes).
    pub fn detect_broken_links(&self, existing_paths: &HashSet<String>) -> Vec<BrokenLink> {
        let mut broken = Vec::new();
        let mut seen = HashSet::new();

        for edge in self.graph.edge_references() {
            if edge.weight().class.matches(EdgeClass::Structural) {
                let src_node = self.graph.node_weight(edge.source());
                let tgt_node = self.graph.node_weight(edge.target());

                if let (Some(src), Some(tgt)) = (src_node, tgt_node) {
                    let target_str = &tgt.path;
                    let exists = existing_paths.contains(target_str)
                        || existing_paths.contains(&format!("{}.md", target_str))
                        || (target_str.ends_with(".md")
                            && existing_paths.contains(&target_str[..target_str.len() - 3]));

                    if !exists {
                        let key = format!("{}->{}:{}", src.path, tgt.path, edge.weight().edge_type);
                        if !seen.contains(&key) {
                            let _ = seen.insert(key);
                            broken.push(BrokenLink {
                                source: src.path.clone(),
                                target: tgt.path.clone(),
                                edge_type: edge.weight().edge_type.clone(),
                                provenance: edge.weight().provenance.clone(),
                            });
                        }
                    }
                }
            }
        }

        broken
    }

    /// Detect circular dependencies in specified directed acyclic relations (e.g. "supersedes", "depends_on").
    pub fn detect_circular_dependencies(&self, edge_types: &[&str]) -> Vec<CircularDependency> {
        let mut cycles = Vec::new();
        let mut seen_cycles: HashSet<String> = HashSet::new();

        for &edge_type in edge_types {
            let mut adj: HashMap<NodeIndex, Vec<NodeIndex>> = HashMap::new();
            for edge in self.graph.edge_references() {
                if edge.weight().edge_type.eq_ignore_ascii_case(edge_type) {
                    adj.entry(edge.source()).or_default().push(edge.target());
                }
            }

            let mut visited: HashSet<NodeIndex> = HashSet::new();
            let mut rec_stack: Vec<NodeIndex> = Vec::new();

            for &start_idx in adj.keys() {
                if !visited.contains(&start_idx) {
                    self.dfs_find_cycles(
                        start_idx,
                        &adj,
                        &mut visited,
                        &mut rec_stack,
                        edge_type,
                        &mut cycles,
                        &mut seen_cycles,
                    );
                }
            }
        }

        cycles
    }

    fn dfs_find_cycles(
        &self,
        node: NodeIndex,
        adj: &HashMap<NodeIndex, Vec<NodeIndex>>,
        visited: &mut HashSet<NodeIndex>,
        rec_stack: &mut Vec<NodeIndex>,
        edge_type: &str,
        cycles: &mut Vec<CircularDependency>,
        seen_cycles: &mut HashSet<String>,
    ) {
        let _ = visited.insert(node);
        rec_stack.push(node);

        if let Some(neighbors) = adj.get(&node) {
            for &next in neighbors {
                if let Some(pos) = rec_stack.iter().position(|&n| n == next) {
                    let mut cycle_nodes = Vec::new();
                    for &n in &rec_stack[pos..] {
                        if let Some(w) = self.graph.node_weight(n) {
                            cycle_nodes.push(w.path.clone());
                        }
                    }
                    if let Some(w) = self.graph.node_weight(next) {
                        cycle_nodes.push(w.path.clone());
                    }

                    let cycle_key = format!("{}:{:?}", edge_type, cycle_nodes);
                    if !seen_cycles.contains(&cycle_key) {
                        let _ = seen_cycles.insert(cycle_key);
                        cycles.push(CircularDependency {
                            edge_type: edge_type.to_string(),
                            cycle: cycle_nodes,
                        });
                    }
                } else if !visited.contains(&next) {
                    self.dfs_find_cycles(
                        next,
                        adj,
                        visited,
                        rec_stack,
                        edge_type,
                        cycles,
                        seen_cycles,
                    );
                }
            }
        }

        let _ = rec_stack.pop();
    }

    /// Detect unattached orphan ADR notes (ADR notes with no inbound or outbound structural links).
    pub fn detect_orphan_adrs(&self, adr_paths: &[String]) -> Vec<OrphanAdr> {
        let mut orphans = Vec::new();

        for path in adr_paths {
            if let Some(&idx) = self.node_map.get(path) {
                let structural_in = self
                    .graph
                    .edges_directed(idx, Direction::Incoming)
                    .filter(|e| e.weight().class.matches(EdgeClass::Structural))
                    .count();
                let structural_out = self
                    .graph
                    .edges_directed(idx, Direction::Outgoing)
                    .filter(|e| e.weight().class.matches(EdgeClass::Structural))
                    .count();

                if structural_in + structural_out == 0 {
                    let title = self.graph.node_weight(idx).and_then(|n| n.title.clone());
                    orphans.push(OrphanAdr {
                        path: path.clone(),
                        title,
                        reason: "ADR note has no inbound or outbound structural links".to_string(),
                    });
                }
            } else {
                orphans.push(OrphanAdr {
                    path: path.clone(),
                    title: None,
                    reason: "ADR note is not connected to the knowledge graph".to_string(),
                });
            }
        }

        orphans
    }
}
