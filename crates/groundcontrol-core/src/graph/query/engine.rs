//! Parameterized Cypher-Lite execution engine.

use groundcontrol_common::types::{GraphImpactSummary, GraphMatchResult, GraphTreeNode};
use groundcontrol_common::Result;

use crate::storage::sqlite::Store;

use super::ast::{NodePattern, PathPattern, QueryDirection};

/// Threshold beyond which a node's fan-out is capped and flagged as a hub.
const HUB_FANOUT_THRESHOLD: usize = 10;

/// Execution engine that executes a `PathPattern` against the SQLite store.
pub struct QueryEngine<'a> {
    store: &'a Store,
}

impl<'a> QueryEngine<'a> {
    /// Create a new query engine over the SQLite store.
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    /// Execute a pattern match with optional class, where filter, and limit constraints.
    pub fn execute_match(
        &self,
        pattern: &PathPattern,
        edge_class_filter: Option<&str>,
        where_filter: Option<&str>,
        limit: usize,
        max_depth_cap: usize,
    ) -> Result<GraphMatchResult> {
        let limit = limit.clamp(1, 100);
        let max_depth_cap = max_depth_cap.clamp(1, 5);

        // 1. Resolve starting anchor candidate nodes
        let anchor_candidates = self.resolve_node_candidates(&pattern.start_node)?;
        if anchor_candidates.is_empty() {
            return Ok(GraphMatchResult::default());
        }

        let mut matching_anchors: Vec<(String, Vec<GraphTreeNode>, usize)> = Vec::new();
        let mut total_nodes_added = 0;

        for anchor in &anchor_candidates {
            if total_nodes_added >= limit {
                break;
            }

            let mut ancestors = std::collections::HashSet::new();
            ancestors.insert(anchor.clone());

            let (sub_tree, direct_count) = self.expand_node(
                anchor,
                0,
                0,
                0,
                pattern,
                edge_class_filter,
                where_filter,
                max_depth_cap,
                &mut ancestors,
                &mut total_nodes_added,
                limit,
            )?;

            if !sub_tree.is_empty() {
                matching_anchors.push((anchor.clone(), sub_tree, direct_count));
            }
        }

        if matching_anchors.is_empty() {
            return Ok(GraphMatchResult::default());
        }

        if matching_anchors.len() == 1 {
            let (anchor, tree, direct_count) = matching_anchors.into_iter().next().unwrap();
            let root = Some(anchor.clone());
            let file = self.format_node_file_line(&anchor);
            let mut summary = self.compute_summary(&tree, Some(&anchor));
            summary.direct = direct_count;
            let total_matches = self.count_total_nodes(&tree);
            Ok(GraphMatchResult { root, file, summary, tree, total_matches })
        } else {
            let mut tree = Vec::new();
            let mut total_direct = 0;
            for (anchor, sub_tree, direct_count) in matching_anchors {
                total_direct += direct_count;
                let (file, _, line) = self.lookup_node_metadata(&anchor);
                tree.push(GraphTreeNode {
                    node: anchor,
                    rel: None,
                    file,
                    line,
                    hop: 0,
                    branches: sub_tree,
                    suppressed: None,
                    hub: None,
                });
            }
            let mut summary = self.compute_summary(&tree, None);
            summary.direct = total_direct;
            let total_matches = self.count_total_nodes(&tree);
            Ok(GraphMatchResult { root: None, file: None, summary, tree, total_matches })
        }
    }

    /// Resolve candidate node identifiers from node pattern properties.
    fn resolve_node_candidates(&self, node: &NodePattern) -> Result<Vec<String>> {
        // Direct path property
        if let Some(path) = node.properties.get("path") {
            return Ok(vec![path.clone()]);
        }

        // Name property: query code_symbols and exact edge matches
        if let Some(name) = node.properties.get("name") {
            let mut candidates = Vec::new();
            if let Ok(syms) = self.store.find_symbols_by_name(name) {
                for s in syms {
                    candidates.push(s.scope_path);
                }
            }
            if candidates.is_empty() {
                candidates.push(name.clone());
            }
            candidates.sort();
            candidates.dedup();
            return Ok(candidates);
        }

        // Title property for doc nodes
        if let Some(title) = node.properties.get("title") {
            if let Ok(files) = self.store.list_files() {
                let matches: Vec<String> = files
                    .into_iter()
                    .filter(|f| f.title.as_deref() == Some(title))
                    .map(|f| f.path)
                    .collect();
                if !matches.is_empty() {
                    return Ok(matches);
                }
            }
        }

        // If variable or label without properties, fall back to empty or symbol types
        if let Some(label) = &node.label {
            if label == "CodeSymbol" {
                if let Ok(syms) = self.store.get_all_code_symbols() {
                    return Ok(syms.into_iter().take(25).map(|s| s.scope_path).collect());
                }
            }
        }

        // If no properties or unbound variable, resolve distinct sources from edges table
        let endpoints = self.store.distinct_edge_sources(100).unwrap_or_default();
        if !endpoints.is_empty() {
            return Ok(endpoints);
        }

        Ok(Vec::new())
    }

    /// Recursively expand nodes along pattern steps, capping at HUB_FANOUT_THRESHOLD.
    #[allow(clippy::too_many_arguments)]
    fn expand_node(
        &self,
        current_node: &str,
        step_idx: usize,
        hop_in_step: usize,
        cumulative_hop: usize,
        pattern: &PathPattern,
        edge_class_filter: Option<&str>,
        where_filter: Option<&str>,
        max_depth_cap: usize,
        ancestors: &mut std::collections::HashSet<String>,
        total_nodes_added: &mut usize,
        limit: usize,
    ) -> Result<(Vec<GraphTreeNode>, usize)> {
        if step_idx >= pattern.steps.len() {
            return Ok((Vec::new(), 0));
        }
        if cumulative_hop >= max_depth_cap || *total_nodes_added >= limit {
            return Ok((Vec::new(), 0));
        }

        let (edge_pat, next_node_pat) = &pattern.steps[step_idx];
        let step_max_depth = edge_pat.max_hops.min(max_depth_cap.saturating_sub(cumulative_hop));
        let step_min_depth = edge_pat.min_hops.max(1);

        if step_max_depth == 0 {
            return Ok((Vec::new(), 0));
        }

        let raw_neighbors = self.get_immediate_neighbors(
            current_node,
            edge_pat.direction,
            &edge_pat.edge_types,
            edge_class_filter,
        )?;

        let mut filtered: Vec<(String, String)> = Vec::new();
        for (target_node, rel) in raw_neighbors {
            if ancestors.contains(&target_node) {
                continue;
            }
            if let Some(wf) = where_filter {
                if !self.eval_where_filter(&target_node, wf) {
                    continue;
                }
            }
            filtered.push((target_node, rel));
        }

        let total_direct_neighbors = filtered.len();
        let is_hub = total_direct_neighbors > HUB_FANOUT_THRESHOLD;
        let take_count = if is_hub { HUB_FANOUT_THRESHOLD } else { total_direct_neighbors };

        let mut tree_nodes = Vec::new();

        for (target_node, rel) in filtered.into_iter().take(take_count) {
            if *total_nodes_added >= limit {
                break;
            }

            let next_hop = cumulative_hop + 1;
            let (file, _, line) = self.lookup_node_metadata(&target_node);
            let matches_constraint = self.matches_node_constraints(&target_node, next_node_pat)?;

            let mut branches = Vec::new();
            let mut child_suppressed = None;
            let mut child_hub = None;

            if next_hop < max_depth_cap && !is_hub {
                ancestors.insert(target_node.clone());

                if hop_in_step + 1 < step_max_depth {
                    let (sub, sub_direct) = self.expand_node(
                        &target_node,
                        step_idx,
                        hop_in_step + 1,
                        next_hop,
                        pattern,
                        edge_class_filter,
                        where_filter,
                        max_depth_cap,
                        ancestors,
                        total_nodes_added,
                        limit,
                    )?;
                    if sub_direct > HUB_FANOUT_THRESHOLD {
                        child_suppressed = Some(sub_direct - HUB_FANOUT_THRESHOLD);
                        child_hub = Some(true);
                    }
                    branches.extend(sub);
                }

                if hop_in_step + 1 >= step_min_depth
                    && step_idx + 1 < pattern.steps.len()
                    && matches_constraint
                {
                    let (sub, sub_direct) = self.expand_node(
                        &target_node,
                        step_idx + 1,
                        0,
                        next_hop,
                        pattern,
                        edge_class_filter,
                        where_filter,
                        max_depth_cap,
                        ancestors,
                        total_nodes_added,
                        limit,
                    )?;
                    if sub_direct > HUB_FANOUT_THRESHOLD {
                        child_suppressed = Some(sub_direct - HUB_FANOUT_THRESHOLD);
                        child_hub = Some(true);
                    }
                    branches.extend(sub);
                }

                ancestors.remove(&target_node);
            }

            if !matches_constraint && branches.is_empty() {
                continue;
            }

            *total_nodes_added += 1;

            tree_nodes.push(GraphTreeNode {
                node: target_node,
                rel: Some(rel),
                file,
                line,
                hop: next_hop,
                branches,
                suppressed: child_suppressed,
                hub: child_hub,
            });
        }

        Ok((tree_nodes, total_direct_neighbors))
    }

    /// Query immediate 1-hop neighbors according to direction and edge types.
    fn get_immediate_neighbors(
        &self,
        node: &str,
        direction: QueryDirection,
        edge_types: &[String],
        edge_class_filter: Option<&str>,
    ) -> Result<Vec<(String, String)>> {
        let (out, inc) = match direction {
            QueryDirection::Outgoing => (true, false),
            QueryDirection::Incoming => (false, true),
            QueryDirection::Undirected => (false, false),
        };
        self.store.expand_step_edges(node, edge_types, edge_class_filter, out, inc)
    }

    /// Compute high-signal cardinality summary (direct, transitive, unique files, max depth).
    fn compute_summary(&self, tree: &[GraphTreeNode], root: Option<&str>) -> GraphImpactSummary {
        let direct = tree.len();
        let mut transitive_nodes = std::collections::HashSet::new();
        let mut all_files = std::collections::HashSet::new();
        let mut max_depth = 0;

        if let Some(r) = root {
            if let (Some(f), _, _) = self.lookup_node_metadata(r) {
                all_files.insert(f);
            }
        }

        fn collect_stats(
            nodes: &[GraphTreeNode],
            transitive_nodes: &mut std::collections::HashSet<String>,
            all_files: &mut std::collections::HashSet<String>,
            max_depth: &mut usize,
        ) {
            for n in nodes {
                if n.hop > *max_depth {
                    *max_depth = n.hop;
                }
                if n.hop >= 2 {
                    transitive_nodes.insert(n.node.clone());
                }
                if let Some(f) = &n.file {
                    all_files.insert(f.clone());
                }
                collect_stats(&n.branches, transitive_nodes, all_files, max_depth);
            }
        }

        collect_stats(tree, &mut transitive_nodes, &mut all_files, &mut max_depth);

        GraphImpactSummary {
            direct,
            transitive: transitive_nodes.len(),
            files: all_files.len(),
            max_depth,
        }
    }

    /// Count all nodes across the entire hierarchical tree.
    fn count_total_nodes(&self, tree: &[GraphTreeNode]) -> usize {
        let mut count = 0;
        fn count_rec(nodes: &[GraphTreeNode], count: &mut usize) {
            for n in nodes {
                *count += 1;
                count_rec(&n.branches, count);
            }
        }
        count_rec(tree, &mut count);
        count
    }

    /// Check if target node conforms to subsequent pattern constraints.
    fn matches_node_constraints(&self, node_id: &str, pattern: &NodePattern) -> Result<bool> {
        if let Some(prop_name) = pattern.properties.get("name") {
            if !node_id.ends_with(prop_name) && node_id != prop_name {
                return Ok(false);
            }
        }
        if let Some(prop_path) = pattern.properties.get("path") {
            if node_id != prop_path {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Evaluate simple `where` clause predicates on candidate path.
    fn eval_where_filter(&self, node_id: &str, where_filter: &str) -> bool {
        let lower_filter = where_filter.to_lowercase();
        let lower_node = node_id.to_lowercase();

        if lower_filter.contains("not") && lower_filter.contains("contains") {
            for part in lower_filter.split("and") {
                if let Some(idx) = part.find("contains") {
                    let target = part[idx + "contains".len()..]
                        .trim()
                        .trim_matches(|c| c == '\'' || c == '"' || c == ' ');
                    if lower_node.contains(target) {
                        return false;
                    }
                }
            }
            return true;
        }

        if lower_filter.contains("contains") {
            for part in lower_filter.split("and") {
                if let Some(idx) = part.find("contains") {
                    let target = part[idx + "contains".len()..]
                        .trim()
                        .trim_matches(|c| c == '\'' || c == '"' || c == ' ');
                    if !lower_node.contains(target) {
                        return false;
                    }
                }
            }
            return true;
        }

        true
    }

    /// Look up metadata (file path, symbol type, start line) for a node.
    fn lookup_node_metadata(
        &self,
        node_id: &str,
    ) -> (Option<String>, Option<String>, Option<usize>) {
        if let Ok(syms) = self.store.find_symbols_by_qualified_name(node_id) {
            if let Some(s) = syms.first() {
                return (
                    Some(s.file_path.clone()),
                    Some(format!("{:?}", s.symbol_type)),
                    Some(s.start_line),
                );
            }
        }
        if let Ok(syms) = self.store.find_symbols_by_name(node_id) {
            if let Some(s) = syms.first() {
                return (
                    Some(s.file_path.clone()),
                    Some(format!("{:?}", s.symbol_type)),
                    Some(s.start_line),
                );
            }
        }
        if let Ok(Some(file)) = self.store.get_file(node_id) {
            return (Some(file.path), Some("DocNode".to_string()), Some(1));
        }
        (None, None, None)
    }

    /// Format node file and line (e.g. "path/to/file.rs:42").
    fn format_node_file_line(&self, node_id: &str) -> Option<String> {
        let (file, _, line) = self.lookup_node_metadata(node_id);
        match (file, line) {
            (Some(f), Some(l)) => Some(format!("{}:{}", f, l)),
            (Some(f), None) => Some(f),
            _ => None,
        }
    }
}
