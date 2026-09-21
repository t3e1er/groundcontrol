//! Query-Time Personalized PageRank (HippoRAG Diffusion) over Petgraph.
//!
//! Propagates activation from lexical/binary seed candidates across AST and wikilink
//! graph edges in 2 power iterations (<1.5ms) without persisting synthetic edges or
//! causing graph bloat.

use std::collections::HashMap;

use groundcontrol_common::config::EdgeClass;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use super::KnowledgeGraph;

/// Default restart probability / teleport damping factor for code diffusion (0.5).
pub const PPR_DEFAULT_ALPHA: f64 = 0.5;

/// Default number of power iterations for query-time diffusion (2 hops).
pub const PPR_DEFAULT_ITERATIONS: usize = 2;

/// A score entry produced by Personalized PageRank diffusion.
#[derive(Debug, Clone, PartialEq)]
pub struct PprScore {
    /// Document or code entity path / scoped identifier.
    pub path: String,
    /// Stationary probability mass accumulated during diffusion.
    pub score: f64,
}

/// Execute query-time Personalized PageRank over the knowledge graph.
///
/// - `graph`: The authoritative knowledge graph.
/// - `seeds`: Initial activated candidates `(path, relevance_score)`.
/// - `alpha`: Teleport/restart probability back to seed distribution (default 0.5).
/// - `iterations`: Number of power-iteration steps (default 2).
/// - `edge_class_filter`: Optional edge class filter (`Code`, `Semantic`, `Structural`, etc.).
pub fn personalized_pagerank(
    graph: &KnowledgeGraph,
    seeds: &[(String, f64)],
    alpha: f64,
    iterations: usize,
    edge_class_filter: Option<EdgeClass>,
) -> Vec<PprScore> {
    if seeds.is_empty() {
        return Vec::new();
    }

    // 1. Map seed paths to NodeIndexes and calculate total seed score
    let mut seed_map: HashMap<NodeIndex, f64> = HashMap::new();
    let mut total_seed_score = 0.0f64;

    for (path, score) in seeds {
        if let Some(&node_idx) = graph.node_map().get(path) {
            let s = score.max(0.001);
            *seed_map.entry(node_idx).or_insert(0.0) += s;
            total_seed_score += s;
        }
    }

    if seed_map.is_empty() || total_seed_score <= 0.0 {
        return Vec::new();
    }

    // 2. Form initial preference probability vector p0 (sums to 1.0)
    let mut p0: HashMap<NodeIndex, f64> = HashMap::new();
    for (&node_idx, &score) in &seed_map {
        p0.insert(node_idx, score / total_seed_score);
    }

    let mut current_p = p0.clone();
    let petgraph = graph.inner_graph();

    // 3. Sparse power iterations
    for _ in 0..iterations {
        let mut diffused: HashMap<NodeIndex, f64> = HashMap::new();

        // Push probability from active nodes to their neighbors
        for (&u, &prob) in &current_p {
            if prob < 1e-9 {
                continue;
            }

            // Collect qualifying neighbors across outgoing edges (and incoming for bidirectional AST flow)
            let mut neighbors = Vec::new();

            for edge in petgraph.edges_directed(u, Direction::Outgoing) {
                if let Some(ec) = edge_class_filter {
                    if edge.weight().class != ec {
                        continue;
                    }
                }
                neighbors.push(edge.target());
            }

            for edge in petgraph.edges_directed(u, Direction::Incoming) {
                if let Some(ec) = edge_class_filter {
                    if edge.weight().class != ec {
                        continue;
                    }
                }
                neighbors.push(edge.source());
            }

            if neighbors.is_empty() {
                // Dangling node: keep probability at self
                *diffused.entry(u).or_insert(0.0) += prob;
            } else {
                let share = prob / (neighbors.len() as f64);
                for v in neighbors {
                    *diffused.entry(v).or_insert(0.0) += share;
                }
            }
        }

        // Apply damping: p^(t+1) = (1 - alpha) * diffused + alpha * p0
        let mut next_p: HashMap<NodeIndex, f64> = HashMap::new();

        // Diffused component
        for (node, prob) in diffused {
            *next_p.entry(node).or_insert(0.0) += (1.0 - alpha) * prob;
        }

        // Teleport back to seeds
        for (&node, &prob) in &p0 {
            *next_p.entry(node).or_insert(0.0) += alpha * prob;
        }

        current_p = next_p;
    }

    // 4. Map back to node paths and sort descending
    let mut results: Vec<PprScore> = current_p
        .into_iter()
        .filter_map(|(node_idx, score)| {
            petgraph.node_weight(node_idx).map(|node| PprScore { path: node.path.clone(), score })
        })
        .collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use groundcontrol_common::types::EdgeProvenance;

    #[test]
    fn test_personalized_pagerank_diffusion() {
        let mut kg = KnowledgeGraph::new();
        kg.add_node("src/main.rs", Some("Main"));
        kg.add_node("src/router.rs", Some("Router"));
        kg.add_node("src/handler.rs", Some("Handler"));

        // main -> router -> handler
        kg.add_edge(
            "src/main.rs",
            "src/router.rs",
            "calls",
            1.0,
            EdgeProvenance::CodeCalls,
            EdgeClass::Code,
        );

        kg.add_edge(
            "src/router.rs",
            "src/handler.rs",
            "calls",
            1.0,
            EdgeProvenance::CodeCalls,
            EdgeClass::Code,
        );

        // Seed: main.rs
        let seeds = vec![("src/main.rs".to_string(), 1.0)];
        let ppr = personalized_pagerank(&kg, &seeds, 0.5, 2, Some(EdgeClass::Code));

        assert!(!ppr.is_empty());
        // Probability mass should diffuse to router and handler
        assert!(ppr.iter().any(|p| p.path == "src/router.rs" && p.score > 0.0));
        assert!(ppr.iter().any(|p| p.path == "src/handler.rs" && p.score > 0.0));
    }
}
