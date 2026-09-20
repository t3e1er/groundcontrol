use std::collections::{HashMap, HashSet};

use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use ctxvault_common::types::{Community, CommunityDensity, CommunityDetectionResult};

use super::KnowledgeGraph;

impl KnowledgeGraph {
    /// Detect communities using the Louvain modularity-based algorithm.
    ///
    /// Treats the directed graph as undirected for community detection
    /// (each directed edge contributes weight in both directions).
    /// Returns communities sorted by size (largest first).
    pub fn detect_communities(&self) -> CommunityDetectionResult {
        let node_count = self.graph.node_count();
        if node_count == 0 {
            return CommunityDetectionResult {
                communities: Vec::new(),
                modularity: 0.0,
                iterations: 0,
            };
        }

        let indices: Vec<NodeIndex> = self.graph.node_indices().collect();
        let n = indices.len();
        let mut idx_to_compact: HashMap<NodeIndex, usize> = HashMap::new();
        for (i, &idx) in indices.iter().enumerate() {
            let _ = idx_to_compact.insert(idx, i);
        }

        let mut adj: HashMap<(usize, usize), f64> = HashMap::new();
        let mut m: f64 = 0.0;

        for edge in self.graph.edge_references() {
            let src = *idx_to_compact.get(&edge.source()).unwrap();
            let tgt = *idx_to_compact.get(&edge.target()).unwrap();
            let w = edge.weight().weight as f64;

            if src != tgt {
                *adj.entry((src, tgt)).or_insert(0.0) += w;
                *adj.entry((tgt, src)).or_insert(0.0) += w;
                m += w;
            }
        }

        if m == 0.0 {
            let communities: Vec<Community> = indices
                .iter()
                .enumerate()
                .map(|(i, &idx)| {
                    let path = self.graph.node_weight(idx).unwrap().path.clone();
                    Community { id: i, members: vec![path], modularity_contribution: 0.0 }
                })
                .collect();
            return CommunityDetectionResult { communities, modularity: 0.0, iterations: 0 };
        }

        let two_m = 2.0 * m;

        let mut k: Vec<f64> = vec![0.0; n];
        for (&(src, _tgt), &w) in &adj {
            k[src] += w;
        }

        let mut neighbors: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
        for (&(src, tgt), &w) in &adj {
            neighbors[src].push((tgt, w));
        }

        let mut community: Vec<usize> = (0..n).collect();
        let mut sigma_tot: Vec<f64> = k.clone();
        let mut sigma_in: Vec<f64> = vec![0.0; n];

        let mut iterations = 0;
        let max_iterations = 100;

        loop {
            iterations += 1;
            let mut improved = false;

            for i in 0..n {
                let ci = community[i];
                let ki = k[i];

                let mut community_weights: HashMap<usize, f64> = HashMap::new();
                for &(j, w) in &neighbors[i] {
                    let cj = community[j];
                    *community_weights.entry(cj).or_insert(0.0) += w;
                }

                let ki_in_own = *community_weights.get(&ci).unwrap_or(&0.0);

                sigma_in[ci] -= ki_in_own;
                sigma_tot[ci] -= ki;

                let mut best_community = ci;
                let mut best_delta_q = 0.0f64;

                for (&cj, &ki_in_cj) in &community_weights {
                    let delta_q = ki_in_cj / two_m - (sigma_tot[cj] * ki) / (two_m * two_m);
                    if delta_q > best_delta_q {
                        best_delta_q = delta_q;
                        best_community = cj;
                    }
                }

                if best_delta_q <= 0.0 {
                    best_community = ci;
                }

                community[i] = best_community;
                let ki_in_best = *community_weights.get(&best_community).unwrap_or(&0.0);
                sigma_in[best_community] += ki_in_best;
                sigma_tot[best_community] += ki;

                if best_community != ci {
                    improved = true;
                }
            }

            if !improved || iterations >= max_iterations {
                break;
            }
        }

        let mut community_members: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, &c) in community.iter().enumerate() {
            community_members.entry(c).or_default().push(i);
        }

        let mut q: f64 = 0.0;
        let mut communities_out: Vec<Community> = Vec::new();
        let mut community_id = 0;

        for (_c, members) in &community_members {
            let member_set: HashSet<usize> = members.iter().copied().collect();

            let mut s_in: f64 = 0.0;
            let mut s_tot: f64 = 0.0;

            for &i in members {
                s_tot += k[i];
                for &(j, w) in &neighbors[i] {
                    if member_set.contains(&j) {
                        s_in += w;
                    }
                }
            }

            let modularity_contribution = (s_in / two_m) - (s_tot / two_m).powi(2);
            q += modularity_contribution;

            let member_paths: Vec<String> = members
                .iter()
                .map(|&i| {
                    let idx = indices[i];
                    self.graph.node_weight(idx).unwrap().path.clone()
                })
                .collect();

            communities_out.push(Community {
                id: community_id,
                members: member_paths,
                modularity_contribution,
            });
            community_id += 1;
        }

        communities_out.sort_by(|a, b| b.members.len().cmp(&a.members.len()));
        for (i, c) in communities_out.iter_mut().enumerate() {
            c.id = i;
        }

        CommunityDetectionResult { communities: communities_out, modularity: q, iterations }
    }

    /// Detect communities with a Leiden-style connectivity refinement pass.
    pub fn detect_communities_leiden(&self) -> CommunityDetectionResult {
        let base = self.detect_communities();
        if base.communities.is_empty() {
            return base;
        }

        let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
        let mut degree: HashMap<String, f64> = HashMap::new();
        let mut m: f64 = 0.0;
        for edge in self.graph.edge_references() {
            let src = &self.graph.node_weight(edge.source()).unwrap().path;
            let tgt = &self.graph.node_weight(edge.target()).unwrap().path;
            if src == tgt {
                continue;
            }
            let w = edge.weight().weight as f64;
            adjacency.entry(src.clone()).or_default().push((tgt.clone(), w));
            adjacency.entry(tgt.clone()).or_default().push((src.clone(), w));
            *degree.entry(src.clone()).or_insert(0.0) += w;
            *degree.entry(tgt.clone()).or_insert(0.0) += w;
            m += w;
        }

        let mut refined: Vec<Vec<String>> = Vec::new();
        for comm in &base.communities {
            let member_set: HashSet<&str> = comm.members.iter().map(|s| s.as_str()).collect();
            let mut members = comm.members.clone();
            members.sort();

            let mut visited: HashSet<String> = HashSet::new();
            for start in &members {
                if visited.contains(start) {
                    continue;
                }
                let mut component: Vec<String> = Vec::new();
                let mut queue: std::collections::VecDeque<String> =
                    std::collections::VecDeque::new();
                queue.push_back(start.clone());
                let _ = visited.insert(start.clone());
                while let Some(node) = queue.pop_front() {
                    component.push(node.clone());
                    if let Some(neigh) = adjacency.get(&node) {
                        let mut ns: Vec<&(String, f64)> = neigh.iter().collect();
                        ns.sort_by(|a, b| a.0.cmp(&b.0));
                        for (next, _w) in ns {
                            if member_set.contains(next.as_str()) && !visited.contains(next) {
                                let _ = visited.insert(next.clone());
                                queue.push_back(next.clone());
                            }
                        }
                    }
                }
                component.sort();
                refined.push(component);
            }
        }

        let two_m = 2.0 * m;
        let mut q = 0.0;
        if two_m > 0.0 {
            for component in &refined {
                let member_set: HashSet<&str> = component.iter().map(|s| s.as_str()).collect();
                let mut s_in = 0.0;
                let mut s_tot = 0.0;
                for node in component {
                    s_tot += degree.get(node).copied().unwrap_or(0.0);
                    if let Some(neigh) = adjacency.get(node) {
                        for (next, w) in neigh {
                            if member_set.contains(next.as_str()) {
                                s_in += w;
                            }
                        }
                    }
                }
                q += (s_in / two_m) - (s_tot / two_m).powi(2);
            }
        }

        refined.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.first().cmp(&b.first())));

        let communities_out: Vec<Community> = refined
            .into_iter()
            .enumerate()
            .map(|(id, members)| {
                let member_set: HashSet<&str> = members.iter().map(|s| s.as_str()).collect();
                let mut s_in = 0.0;
                let mut s_tot = 0.0;
                for node in &members {
                    s_tot += degree.get(node).copied().unwrap_or(0.0);
                    if let Some(neigh) = adjacency.get(node) {
                        for (next, w) in neigh {
                            if member_set.contains(next.as_str()) {
                                s_in += w;
                            }
                        }
                    }
                }
                let modularity_contribution =
                    if two_m > 0.0 { (s_in / two_m) - (s_tot / two_m).powi(2) } else { 0.0 };
                Community { id, members, modularity_contribution }
            })
            .collect();

        CommunityDetectionResult {
            communities: communities_out,
            modularity: q,
            iterations: base.iterations,
        }
    }

    /// Compute per-community density statistics.
    pub fn community_densities(&self) -> Vec<CommunityDensity> {
        let result = self.detect_communities();
        let mut densities = Vec::new();

        for community in &result.communities {
            let node_count = community.members.len();
            if node_count <= 1 {
                densities.push(CommunityDensity {
                    community_id: community.id,
                    node_count,
                    internal_edges: 0,
                    density: 0.0,
                });
                continue;
            }

            let member_set: HashSet<&str> = community.members.iter().map(|s| s.as_str()).collect();

            let mut internal_edges = 0usize;
            for member in &community.members {
                if let Some(&idx) = self.node_map.get(member) {
                    for edge in self.graph.edges_directed(idx, Direction::Outgoing) {
                        let target_idx = edge.target();
                        if let Some(target_node) = self.graph.node_weight(target_idx) {
                            if member_set.contains(target_node.path.as_str()) {
                                internal_edges += 1;
                            }
                        }
                    }
                }
            }

            let max_edges = node_count * (node_count - 1);
            let density =
                if max_edges > 0 { internal_edges as f64 / max_edges as f64 } else { 0.0 };

            densities.push(CommunityDensity {
                community_id: community.id,
                node_count,
                internal_edges,
                density,
            });
        }

        densities
    }
}
