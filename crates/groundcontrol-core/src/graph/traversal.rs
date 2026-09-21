use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use groundcontrol_common::config::EdgeClass;
use groundcontrol_common::types::LineageNode;

use super::KnowledgeGraph;

impl KnowledgeGraph {
    /// BFS from a starting node, up to `max_depth` hops.
    /// Optionally filter by edge types.
    /// Optionally filter by edge class.
    /// Returns (path, hops_from_start).
    pub fn traverse_bfs(
        &self,
        start: &str,
        max_depth: usize,
        edge_type_filter: Option<&[String]>,
        edge_class_filter: Option<EdgeClass>,
    ) -> Vec<(String, usize)> {
        let Some(&start_idx) = self.node_map.get(start) else {
            return Vec::new();
        };

        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let mut queue: VecDeque<(NodeIndex, usize)> = VecDeque::new();
        let mut results: Vec<(String, usize)> = Vec::new();

        let _ = visited.insert(start_idx);
        queue.push_back((start_idx, 0));

        while let Some((current, depth)) = queue.pop_front() {
            if depth > 0 {
                if let Some(node) = self.graph.node_weight(current) {
                    results.push((node.path.clone(), depth));
                }
            }

            if depth >= max_depth {
                continue;
            }

            for edge in self.graph.edges_directed(current, Direction::Outgoing) {
                let edge_data = edge.weight();
                if let Some(filter) = edge_type_filter {
                    if !filter.contains(&edge_data.edge_type) {
                        continue;
                    }
                }
                if let Some(class_filter) = edge_class_filter {
                    if !edge_data.class.matches(class_filter) {
                        continue;
                    }
                }

                let neighbor = edge.target();
                if !visited.contains(&neighbor) {
                    let _ = visited.insert(neighbor);
                    queue.push_back((neighbor, depth + 1));
                }
            }
        }

        results
    }

    /// DFS from a starting node, up to `max_depth` hops.
    /// Optionally filter by edge types.
    /// Optionally filter by edge class.
    /// Returns (path, hops_from_start).
    pub fn traverse_dfs(
        &self,
        start: &str,
        max_depth: usize,
        edge_type_filter: Option<&[String]>,
        edge_class_filter: Option<EdgeClass>,
    ) -> Vec<(String, usize)> {
        let Some(&start_idx) = self.node_map.get(start) else {
            return Vec::new();
        };

        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let mut stack: Vec<(NodeIndex, usize)> = Vec::new();
        let mut results: Vec<(String, usize)> = Vec::new();

        let _ = visited.insert(start_idx);
        stack.push((start_idx, 0));

        while let Some((current, depth)) = stack.pop() {
            if depth > 0 {
                if let Some(node) = self.graph.node_weight(current) {
                    results.push((node.path.clone(), depth));
                }
            }

            if depth >= max_depth {
                continue;
            }

            for edge in self.graph.edges_directed(current, Direction::Outgoing) {
                let edge_data = edge.weight();
                if let Some(filter) = edge_type_filter {
                    if !filter.contains(&edge_data.edge_type) {
                        continue;
                    }
                }
                if let Some(class_filter) = edge_class_filter {
                    if !edge_data.class.matches(class_filter) {
                        continue;
                    }
                }

                let neighbor = edge.target();
                if !visited.contains(&neighbor) {
                    let _ = visited.insert(neighbor);
                    stack.push((neighbor, depth + 1));
                }
            }
        }

        results
    }

    /// Get all notes that link TO this note, grouped by edge type.
    pub fn backlinks(
        &self,
        path: &str,
        edge_class_filter: Option<EdgeClass>,
    ) -> HashMap<String, Vec<String>> {
        let mut result: HashMap<String, Vec<String>> = HashMap::new();

        let Some(&idx) = self.node_map.get(path) else {
            return result;
        };

        for edge in self.graph.edges_directed(idx, Direction::Incoming) {
            let source_idx = edge.source();
            if let Some(source_node) = self.graph.node_weight(source_idx) {
                let edge_data = edge.weight();
                if let Some(class_filter) = edge_class_filter {
                    if !edge_data.class.matches(class_filter) {
                        continue;
                    }
                }
                result
                    .entry(edge_data.edge_type.clone())
                    .or_default()
                    .push(source_node.path.clone());
            }
        }

        result
    }

    /// Get all notes this note links TO, grouped by edge type.
    pub fn forwardlinks(
        &self,
        path: &str,
        edge_class_filter: Option<EdgeClass>,
    ) -> HashMap<String, Vec<String>> {
        let mut result: HashMap<String, Vec<String>> = HashMap::new();

        let Some(&idx) = self.node_map.get(path) else {
            return result;
        };

        for edge in self.graph.edges_directed(idx, Direction::Outgoing) {
            let target_idx = edge.target();
            if let Some(target_node) = self.graph.node_weight(target_idx) {
                let edge_data = edge.weight();
                if let Some(class_filter) = edge_class_filter {
                    if !edge_data.class.matches(class_filter) {
                        continue;
                    }
                }
                result
                    .entry(edge_data.edge_type.clone())
                    .or_default()
                    .push(target_node.path.clone());
            }
        }

        result
    }

    /// Find shortest path between two nodes (optionally filtered by edge types).
    /// Returns the path as a list of document paths, or None if no path exists.
    pub fn shortest_path(
        &self,
        from: &str,
        to: &str,
        edge_type_filter: Option<&[String]>,
        edge_class_filter: Option<EdgeClass>,
    ) -> Option<Vec<String>> {
        let start_idx = *self.node_map.get(from)?;
        let end_idx = *self.node_map.get(to)?;

        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let mut queue: VecDeque<NodeIndex> = VecDeque::new();
        let mut parents: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        let _ = visited.insert(start_idx);
        queue.push_back(start_idx);

        let mut found = false;

        while let Some(current) = queue.pop_front() {
            if current == end_idx {
                found = true;
                break;
            }

            for edge in self.graph.edges_directed(current, Direction::Outgoing) {
                let edge_data = edge.weight();
                if let Some(filter) = edge_type_filter {
                    if !filter.contains(&edge_data.edge_type) {
                        continue;
                    }
                }
                if let Some(class_filter) = edge_class_filter {
                    if !edge_data.class.matches(class_filter) {
                        continue;
                    }
                }

                let neighbor = edge.target();
                if !visited.contains(&neighbor) {
                    let _ = visited.insert(neighbor);
                    let _ = parents.insert(neighbor, current);
                    queue.push_back(neighbor);
                }
            }
        }

        if !found {
            return None;
        }

        let mut path_indices = Vec::new();
        let mut current = end_idx;
        while current != start_idx {
            path_indices.push(current);
            current = *parents.get(&current)?;
        }
        path_indices.push(start_idx);
        path_indices.reverse();

        let path: Vec<String> = path_indices
            .iter()
            .filter_map(|&idx| self.graph.node_weight(idx).map(|n| n.path.clone()))
            .collect();

        Some(path)
    }

    /// Compute graph degree affordances for a node in Petgraph (O(deg) direct lookup).
    pub fn compute_affordances(&self, path: &str) -> groundcontrol_common::types::GraphAffordances {
        let Some(&idx) = self.node_map.get(path) else {
            return groundcontrol_common::types::GraphAffordances::default();
        };

        let mut affordances = groundcontrol_common::types::GraphAffordances::default();

        for edge_ref in self.graph.edges_directed(idx, Direction::Outgoing) {
            let weight = edge_ref.weight();
            match weight.edge_type.as_str() {
                "calls" => affordances.calls_out = Some(affordances.calls_out.unwrap_or(0) + 1),
                "implements" | "implements_trait" => {
                    affordances.implements = Some(affordances.implements.unwrap_or(0) + 1)
                }
                "imports" => affordances.imports = Some(affordances.imports.unwrap_or(0) + 1),
                "wikilink" => {
                    affordances.wikilinks_out = Some(affordances.wikilinks_out.unwrap_or(0) + 1)
                }
                "documents" => {
                    affordances.documents_code = Some(affordances.documents_code.unwrap_or(0) + 1)
                }
                other => {
                    *affordances.edge_counts.entry(other.to_string()).or_insert(0) += 1;
                }
            }
        }

        for edge_ref in self.graph.edges_directed(idx, Direction::Incoming) {
            let weight = edge_ref.weight();
            match weight.edge_type.as_str() {
                "calls" => affordances.calls_in = Some(affordances.calls_in.unwrap_or(0) + 1),
                "wikilink" => {
                    affordances.wikilinks_in = Some(affordances.wikilinks_in.unwrap_or(0) + 1)
                }
                "documents" => {
                    let existing = affordances.documents_code.unwrap_or(0);
                    affordances.documents_code = Some(existing + 1);
                }
                other => {
                    let key = format!("{}_in", other);
                    *affordances.edge_counts.entry(key).or_insert(0) += 1;
                }
            }
        }

        affordances
    }

    /// Format immediate 1-hop graph neighborhood as a compact Cypher-Lite ASCII expression.
    pub fn format_cypher_affordances(&self, path: &str, max_neighbors: usize) -> Option<String> {
        let &idx = self.node_map.get(path)?;

        let mut incoming_by_type: HashMap<&str, Vec<String>> = HashMap::new();
        let mut outgoing_by_type: HashMap<&str, Vec<String>> = HashMap::new();

        for edge_ref in self.graph.edges_directed(idx, Direction::Incoming) {
            let edge_type = edge_ref.weight().edge_type.as_str();
            let source_name = clean_node_name(&self.graph[edge_ref.source()].path);
            incoming_by_type.entry(edge_type).or_default().push(source_name);
        }

        for edge_ref in self.graph.edges_directed(idx, Direction::Outgoing) {
            let edge_type = edge_ref.weight().edge_type.as_str();
            let target_name = clean_node_name(&self.graph[edge_ref.target()].path);
            outgoing_by_type.entry(edge_type).or_default().push(target_name);
        }

        if incoming_by_type.is_empty() && outgoing_by_type.is_empty() {
            return None;
        }

        let mut clauses = Vec::new();

        // Format incoming: <-[:rel*N (suppressed M)]-(nodes)
        let mut in_keys: Vec<_> = incoming_by_type.keys().copied().collect();
        in_keys.sort();
        for edge_type in in_keys {
            let mut neighbors = incoming_by_type.remove(edge_type).unwrap_or_default();
            neighbors.sort();
            neighbors.dedup();
            let total = neighbors.len();
            if total > max_neighbors {
                let suppressed = total - max_neighbors;
                let preview =
                    neighbors.into_iter().take(max_neighbors).collect::<Vec<_>>().join(", ");
                clauses.push(format!(
                    "<-[:{}*{} (suppressed {})]-({})",
                    edge_type, total, suppressed, preview
                ));
            } else if total > 1 {
                let preview = neighbors.join(", ");
                clauses.push(format!("<-[:{}*{}]-({})", edge_type, total, preview));
            } else if let Some(single) = neighbors.first() {
                clauses.push(format!("<-[:{}]-({})", edge_type, single));
            }
        }

        // Format outgoing: -[:rel*N (suppressed M)]->(nodes)
        let mut out_keys: Vec<_> = outgoing_by_type.keys().copied().collect();
        out_keys.sort();
        for edge_type in out_keys {
            let mut neighbors = outgoing_by_type.remove(edge_type).unwrap_or_default();
            neighbors.sort();
            neighbors.dedup();
            let total = neighbors.len();
            if total > max_neighbors {
                let suppressed = total - max_neighbors;
                let preview =
                    neighbors.into_iter().take(max_neighbors).collect::<Vec<_>>().join(", ");
                clauses.push(format!(
                    "-[:{}*{} (suppressed {})]->({})",
                    edge_type, total, suppressed, preview
                ));
            } else if total > 1 {
                let preview = neighbors.join(", ");
                clauses.push(format!("-[:{}*{}]->({})", edge_type, total, preview));
            } else if let Some(single) = neighbors.first() {
                clauses.push(format!("-[:{}]->({})", edge_type, single));
            }
        }

        Some(clauses.join(", "))
    }

    /// Return the total in-degree of a node directly without allocating affordance maps.
    pub fn in_degree(&self, path: &str) -> usize {
        let Some(&idx) = self.node_map.get(path) else {
            return 0;
        };
        self.graph.edges_directed(idx, Direction::Incoming).count()
    }

    /// Deterministically traverse the graph along a specified structural edge type.
    pub fn traverse_lineage(
        &self,
        start: &str,
        edge_type: &str,
        direction: &str,
        max_depth: usize,
    ) -> Vec<LineageNode> {
        let mut results = Vec::new();

        let Some(&start_idx) = self.node_map.get(start) else {
            return results;
        };

        let start_title = self.graph.node_weight(start_idx).and_then(|n| n.title.clone());
        results.push(LineageNode {
            path: start.to_string(),
            title: start_title,
            depth: 0,
            edge_type: edge_type.to_string(),
            direction: "start".to_string(),
        });

        if max_depth == 0 {
            return results;
        }

        let dir_lower = direction.to_lowercase();
        let allow_outgoing = dir_lower == "outgoing" || dir_lower == "both";
        let allow_incoming = dir_lower == "incoming" || dir_lower == "both";

        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let _ = visited.insert(start_idx);

        let mut queue: VecDeque<(NodeIndex, usize)> = VecDeque::new();
        queue.push_back((start_idx, 0));

        while let Some((curr, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            if allow_outgoing {
                for edge in self.graph.edges_directed(curr, Direction::Outgoing) {
                    if edge.weight().edge_type.eq_ignore_ascii_case(edge_type) {
                        let neighbor = edge.target();
                        if !visited.contains(&neighbor) {
                            let _ = visited.insert(neighbor);
                            let title =
                                self.graph.node_weight(neighbor).and_then(|n| n.title.clone());
                            let path = self
                                .graph
                                .node_weight(neighbor)
                                .map(|n| n.path.clone())
                                .unwrap_or_default();
                            results.push(LineageNode {
                                path,
                                title,
                                depth: depth + 1,
                                edge_type: edge.weight().edge_type.clone(),
                                direction: "outgoing".to_string(),
                            });
                            queue.push_back((neighbor, depth + 1));
                        }
                    }
                }
            }

            if allow_incoming {
                for edge in self.graph.edges_directed(curr, Direction::Incoming) {
                    if edge.weight().edge_type.eq_ignore_ascii_case(edge_type) {
                        let neighbor = edge.source();
                        if !visited.contains(&neighbor) {
                            let _ = visited.insert(neighbor);
                            let title =
                                self.graph.node_weight(neighbor).and_then(|n| n.title.clone());
                            let path = self
                                .graph
                                .node_weight(neighbor)
                                .map(|n| n.path.clone())
                                .unwrap_or_default();
                            results.push(LineageNode {
                                path,
                                title,
                                depth: depth + 1,
                                edge_type: edge.weight().edge_type.clone(),
                                direction: "incoming".to_string(),
                            });
                            queue.push_back((neighbor, depth + 1));
                        }
                    }
                }
            }
        }

        results
    }

    /// Extract active structural lineage metadata for a node.
    pub fn extract_lineage_for_node(
        &self,
        path: &str,
    ) -> Option<groundcontrol_common::types::LineageAnnotation> {
        let &idx = self.node_map.get(path)?;

        let mut ann = groundcontrol_common::types::LineageAnnotation::default();

        for edge in self.graph.edges_directed(idx, Direction::Incoming) {
            let src_idx = edge.source();
            if let Some(src_node) = self.graph.node_weight(src_idx) {
                let et = edge.weight().edge_type.to_lowercase();
                match et.as_str() {
                    "supersedes" => ann.superseded_by.push(src_node.path.clone()),
                    "implements" => ann.implemented_by.push(src_node.path.clone()),
                    "depends_on" | "dependson" | "depends-on" => {
                        ann.depended_on_by.push(src_node.path.clone())
                    }
                    "adr_for" | "adrfor" | "adr-for" => ann.has_adr.push(src_node.path.clone()),
                    "parent_of" | "parentof" | "parent-of" => {
                        ann.child_of.push(src_node.path.clone())
                    }
                    _ => {
                        if edge.weight().class.matches(EdgeClass::Structural) {
                            ann.incoming
                                .entry(edge.weight().edge_type.clone())
                                .or_default()
                                .push(src_node.path.clone());
                        }
                    }
                }
            }
        }

        for edge in self.graph.edges_directed(idx, Direction::Outgoing) {
            let tgt_idx = edge.target();
            if let Some(tgt_node) = self.graph.node_weight(tgt_idx) {
                let et = edge.weight().edge_type.to_lowercase();
                match et.as_str() {
                    "supersedes" => ann.supersedes.push(tgt_node.path.clone()),
                    "implements" => ann.implements.push(tgt_node.path.clone()),
                    "depends_on" | "dependson" | "depends-on" => {
                        ann.depends_on.push(tgt_node.path.clone())
                    }
                    "adr_for" | "adrfor" | "adr-for" => ann.adr_for.push(tgt_node.path.clone()),
                    "parent_of" | "parentof" | "parent-of" => {
                        ann.parent_of.push(tgt_node.path.clone())
                    }
                    _ => {
                        if edge.weight().class.matches(EdgeClass::Structural) {
                            ann.outgoing
                                .entry(edge.weight().edge_type.clone())
                                .or_default()
                                .push(tgt_node.path.clone());
                        }
                    }
                }
            }
        }

        // Sort and deduplicate vectors for determinism
        ann.superseded_by.sort();
        ann.superseded_by.dedup();
        ann.supersedes.sort();
        ann.supersedes.dedup();
        ann.implements.sort();
        ann.implements.dedup();
        ann.implemented_by.sort();
        ann.implemented_by.dedup();
        ann.depends_on.sort();
        ann.depends_on.dedup();
        ann.depended_on_by.sort();
        ann.depended_on_by.dedup();
        ann.adr_for.sort();
        ann.adr_for.dedup();
        ann.has_adr.sort();
        ann.has_adr.dedup();
        ann.parent_of.sort();
        ann.parent_of.dedup();
        ann.child_of.sort();
        ann.child_of.dedup();

        for list in ann.incoming.values_mut() {
            list.sort();
            list.dedup();
        }
        for list in ann.outgoing.values_mut() {
            list.sort();
            list.dedup();
        }

        if ann.is_empty() {
            None
        } else {
            Some(ann)
        }
    }
}

fn clean_node_name(path: &str) -> String {
    if path.contains('/') || path.contains('\\') {
        if let Some(pos) = path.rfind(['/', '\\']) {
            return path[pos + 1..].to_string();
        }
    }
    if let Some(pos) = path.rfind(" > ") {
        return path[pos + 3..].to_string();
    }
    path.to_string()
}
