//! Tiered Level-of-Detail (LOD) progressive disclosure layout pipelines.
//!
//! Enforces visual progressive disclosure:
//! - Tier 0: Galaxy Overview (~1,000 hubs and community centroids)
//! - Tier 1: Corpus Shell (budget-capped at 25,000–50,000 nodes)
//! - Tier 2: Local Ego Subgraph (1–3 hops around clicked entity)

use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::visit::EdgeRef;
use petgraph::Direction;

use crate::layout::{
    assign_color_and_type, compute_force_layout, ClusterMode, EdgeLayout, GraphLayout,
    LayoutConfig, NodeLayout,
};
use crate::loader::{CorpusCatalog, CorpusSnapshot};

/// Maximum default nodes allowed in Tier 0 Galaxy view.
pub const TIER_0_BUDGET: usize = 50000;
/// Default node budget for Tier 1 Corpus view.
pub const TIER_1_DEFAULT_BUDGET: usize = 50000;

/// Compute 3D celestial coordinates for all corpus clouds in Galaxy view.
/// Distributes corpora across 3D space, grouping connected corpora together
/// based on cojoining cross-corpus edges while guaranteeing spatial sovereignty (no overlapping clouds)
/// for N >= 10 corpora.
pub fn compute_corpus_centers(catalog: &CorpusCatalog) -> HashMap<String, [f32; 3]> {
    let names = catalog.corpus_names();
    let num_corpora = names.len();
    if num_corpora == 0 {
        return HashMap::new();
    }
    if num_corpora == 1 {
        let mut map = HashMap::new();
        map.insert(names[0].clone(), [0.0, 0.0, 0.0]);
        return map;
    }

    // 1. Build lookup of node paths per corpus to measure inter-corpus connectivity
    let mut corpus_paths: Vec<HashSet<String>> = Vec::with_capacity(num_corpora);
    for name in &names {
        let mut paths = HashSet::new();
        if let Some(snap) = catalog.get_corpus(name) {
            let pet = snap.graph.inner();
            for n_idx in pet.node_indices() {
                let p = pet[n_idx].path.replace('\\', "/");
                paths.insert(p.clone());
                if let Some(sub) = p.split('#').nth(1) {
                    paths.insert(sub.to_string());
                }
            }
        }
        corpus_paths.push(paths);
    }

    // 2. Count cross-corpus edge references between each pair of corpora (i, j)
    let mut inter_links: HashMap<(usize, usize), usize> = HashMap::new();
    for (i, name) in names.iter().enumerate() {
        if let Some(snap) = catalog.get_corpus(name) {
            let pet = snap.graph.inner();
            for edge_ref in pet.edge_references() {
                let tgt_p = pet[edge_ref.target()].path.replace('\\', "/");
                let tgt_sub = tgt_p.split('#').nth(1).unwrap_or("");

                for (j, other_paths) in corpus_paths.iter().enumerate() {
                    if i != j
                        && (other_paths.contains(&tgt_p)
                            || (!tgt_sub.is_empty() && other_paths.contains(tgt_sub)))
                    {
                        let key = if i < j { (i, j) } else { (j, i) };
                        *inter_links.entry(key).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let mut positions = Vec::with_capacity(num_corpora);

    // If 2 corpora:
    if num_corpora == 2 {
        let links = inter_links.get(&(0, 1)).copied().unwrap_or(0);
        // If cojoining edges exist, pull closer (1800 units total distance), else separate (2800 units)
        let sep = if links > 0 { 900.0 } else { 1400.0 };
        positions.push([-sep, 0.0, 0.0]);
        positions.push([sep, 0.0, 0.0]);
    } else {
        // N >= 3: Initialize on 3D spherical Fibonacci lattice
        let n_f = num_corpora as f32;
        let phi = (1.0 + 5.0_f32.sqrt()) / 2.0;
        let sphere_radius = 1200.0 + n_f * 250.0;

        for i in 0..num_corpora {
            let idx_f = i as f32;
            let y = 1.0 - (idx_f / (n_f - 1.0)) * 2.0;
            let radius_at_y = (1.0 - y * y).max(0.0).sqrt();
            let theta = 2.0 * std::f32::consts::PI * idx_f / phi;
            let x = theta.cos() * radius_at_y * sphere_radius;
            let y_pos = y * (sphere_radius * 0.6);
            let z = theta.sin() * radius_at_y * sphere_radius;
            positions.push([x, y_pos, z]);
        }

        // Run 50 iterations of spring-electrical relaxation to group cojoined corpora
        let min_sep = 1600.0_f32; // Minimum distance between any two corpus clouds
        for _ in 0..50 {
            let mut forces = vec![[0.0_f32; 3]; num_corpora];

            // All-pairs repulsion (prevent overlap)
            for i in 0..num_corpora {
                for j in (i + 1)..num_corpora {
                    let dx = positions[j][0] - positions[i][0];
                    let dy = positions[j][1] - positions[i][1];
                    let dz = positions[j][2] - positions[i][2];
                    let mut dist = (dx * dx + dy * dy + dz * dz).sqrt();
                    if dist < 1.0 {
                        dist = 1.0;
                    }
                    let nx = dx / dist;
                    let ny = dy / dist;
                    let nz = dz / dist;

                    let rep = if dist < min_sep {
                        (min_sep - dist) * 1.8 + 400000.0 / (dist * dist).max(100.0)
                    } else {
                        250000.0 / (dist * dist)
                    };

                    forces[i][0] -= nx * rep;
                    forces[i][1] -= ny * rep;
                    forces[i][2] -= nz * rep;

                    forces[j][0] += nx * rep;
                    forces[j][1] += ny * rep;
                    forces[j][2] += nz * rep;

                    // Attraction along cojoining edges
                    let key = (i, j);
                    let links = inter_links.get(&key).copied().unwrap_or(0);
                    if links > 0 {
                        let target_dist = 1800.0_f32;
                        if dist > target_dist {
                            let pull = ((dist - target_dist)
                                * (0.04 + 0.02 * (links as f32).ln_1p()))
                            .min(180.0);
                            forces[i][0] += nx * pull;
                            forces[i][1] += ny * pull;
                            forces[i][2] += nz * pull;

                            forces[j][0] -= nx * pull;
                            forces[j][1] -= ny * pull;
                            forces[j][2] -= nz * pull;
                        }
                    }
                }
            }

            // Apply dampening and radial boundary pull
            for i in 0..num_corpora {
                let d_center =
                    (positions[i][0].powi(2) + positions[i][1].powi(2) + positions[i][2].powi(2))
                        .sqrt();
                let center_pull = (d_center - sphere_radius) * 0.02;
                if d_center > 1.0 {
                    forces[i][0] -= (positions[i][0] / d_center) * center_pull;
                    forces[i][1] -= (positions[i][1] / d_center) * center_pull;
                    forces[i][2] -= (positions[i][2] / d_center) * center_pull;
                }

                positions[i][0] += forces[i][0].clamp(-150.0, 150.0) * 0.3;
                positions[i][1] += forces[i][1].clamp(-150.0, 150.0) * 0.3;
                positions[i][2] += forces[i][2].clamp(-150.0, 150.0) * 0.3;
            }
        }
    }

    let mut result = HashMap::with_capacity(num_corpora);
    for (i, name) in names.into_iter().enumerate() {
        result.insert(name, positions[i]);
    }
    result
}

/// Fallback single-index celestial coordinate calculation.
pub fn compute_corpus_center(c_idx: usize, num_corpora: usize) -> [f32; 3] {
    if num_corpora <= 1 {
        return [0.0, 0.0, 0.0];
    }
    if num_corpora == 2 {
        let sep = 1350.0;
        return if c_idx == 0 { [-sep, 0.0, 0.0] } else { [sep, 0.0, 0.0] };
    }

    let n = num_corpora as f32;
    let i = c_idx as f32;
    let phi = (1.0 + 5.0_f32.sqrt()) / 2.0;
    let y = 1.0 - (i / (n - 1.0)) * 2.0;
    let radius_at_y = (1.0 - y * y).max(0.0).sqrt();
    let theta = 2.0 * std::f32::consts::PI * i / phi;

    let sphere_radius = 1200.0 + n * 250.0;
    let x = theta.cos() * radius_at_y * sphere_radius;
    let y_pos = y * (sphere_radius * 0.55);
    let z = theta.sin() * radius_at_y * sphere_radius;

    [x, y_pos, z]
}

/// Compute Tier 0: Galaxy Overview across all loaded corpora or a specific corpus.
pub fn build_tier_0_overview(
    catalog: &CorpusCatalog,
    budget: usize,
    cluster_mode: ClusterMode,
) -> GraphLayout {
    let mut all_nodes = Vec::new();
    let mut all_edges = Vec::new();

    let corpus_names = catalog.corpus_names();
    let num_corpora = corpus_names.len().max(1);
    let centers = compute_corpus_centers(catalog);

    // Global path -> node_id mapping for cross-corpus links
    let mut path_to_global_id: HashMap<String, u32> = HashMap::new();
    let mut node_to_corpus: Vec<usize> = Vec::new();

    for (c_idx, name) in corpus_names.iter().enumerate() {
        let Some(snapshot) = catalog.get_corpus(name) else {
            continue;
        };

        // Budget per corpus in multi-corpus mode
        let per_corpus_budget = (budget / num_corpora).max(2000);
        let layout = build_tier_1_corpus(&snapshot, per_corpus_budget, cluster_mode);

        let center = centers.get(name).copied().unwrap_or([0.0, 0.0, 0.0]);
        let base_id = all_nodes.len() as u32;
        // Namespacing community IDs per corpus guarantees commCentroids in frontend never collapses
        let base_comm = ((c_idx + 1) as u32) * 1000;
        let mut id_map = HashMap::new();

        for (local_i, mut node) in layout.nodes.into_iter().enumerate() {
            let global_id = base_id + (local_i as u32);
            id_map.insert(node.id, global_id);

            path_to_global_id.insert(node.path.clone(), global_id);
            let clean_p = node.path.replace('\\', "/");
            path_to_global_id.insert(clean_p.clone(), global_id);
            if let Some(sub) = clean_p.split('#').nth(1) {
                path_to_global_id.insert(sub.to_string(), global_id);
            }

            node.id = global_id;
            node.community += base_comm;
            node.position[0] += center[0];
            node.position[1] += center[1];
            node.position[2] += center[2];
            all_nodes.push(node);
            node_to_corpus.push(c_idx);
        }

        for mut edge in layout.edges {
            if let (Some(&src), Some(&tgt)) = (id_map.get(&edge.source), id_map.get(&edge.target)) {
                edge.source = src;
                edge.target = tgt;
                all_edges.push(edge);
            }
        }
    }

    // Detect and connect cojoining cross-corpus edges
    for name in &corpus_names {
        if let Some(snapshot) = catalog.get_corpus(name) {
            let pet = snapshot.graph.inner();
            for edge_ref in pet.edge_references() {
                let src_node = &pet[edge_ref.source()];
                let tgt_node = &pet[edge_ref.target()];
                let src_p = src_node.path.replace('\\', "/");
                let tgt_p = tgt_node.path.replace('\\', "/");

                if let (Some(&s_id), Some(&t_id)) = (
                    path_to_global_id.get(&src_p).or_else(|| {
                        src_p.split('#').nth(1).and_then(|sub| path_to_global_id.get(sub))
                    }),
                    path_to_global_id.get(&tgt_p).or_else(|| {
                        tgt_p.split('#').nth(1).and_then(|sub| path_to_global_id.get(sub))
                    }),
                ) {
                    let s_idx = s_id as usize;
                    let t_idx = t_id as usize;
                    if s_id != t_id
                        && s_idx < node_to_corpus.len()
                        && t_idx < node_to_corpus.len()
                        && node_to_corpus[s_idx] != node_to_corpus[t_idx]
                    {
                        all_edges.push(EdgeLayout {
                            source: s_id,
                            target: t_id,
                            edge_type: "cross_corpus".to_string(),
                            weight: 2.0,
                        });
                    }
                }
            }
        }
    }

    GraphLayout {
        corpus: "all".to_string(),
        nodes: all_nodes,
        edges: all_edges,
        communities_count: num_corpora,
    }
}

/// Compute Tier 1: Corpus Shell layout bounded by a maximum node budget.
pub fn build_tier_1_corpus(
    snapshot: &CorpusSnapshot,
    budget: usize,
    cluster_mode: ClusterMode,
) -> GraphLayout {
    let pet_graph = snapshot.graph.inner();
    let total_nodes = pet_graph.node_count();

    if total_nodes == 0 {
        return GraphLayout {
            corpus: snapshot.name.clone(),
            nodes: Vec::new(),
            edges: Vec::new(),
            communities_count: 0,
        };
    }

    // Collect degree per node
    let mut degree_pairs: Vec<(petgraph::graph::NodeIndex, usize)> = pet_graph
        .node_indices()
        .map(|idx| {
            let in_d = pet_graph.edges_directed(idx, Direction::Incoming).count();
            let out_d = pet_graph.edges_directed(idx, Direction::Outgoing).count();
            (idx, in_d + out_d)
        })
        .collect();

    // Sort descending by degree for hub selection
    degree_pairs.sort_by(|a, b| b.1.cmp(&a.1));

    // Select top nodes within budget
    let selected_indices: HashSet<petgraph::graph::NodeIndex> =
        degree_pairs.iter().take(budget).map(|&(idx, _)| idx).collect();

    let mut paths = Vec::new();
    let mut titles = Vec::new();
    let mut degrees = Vec::new();
    let mut communities = Vec::new();
    let mut idx_to_local: HashMap<petgraph::graph::NodeIndex, usize> = HashMap::new();

    // Detect communities using Leiden or Louvain
    let community_res = match cluster_mode {
        ClusterMode::Community => snapshot.graph.detect_communities_leiden(),
        ClusterMode::Directory => snapshot.graph.detect_communities(),
    };

    let mut node_to_comm: HashMap<String, u32> = HashMap::new();
    for (comm_id, comm) in community_res.communities.iter().enumerate() {
        for member in &comm.members {
            node_to_comm.insert(member.clone(), comm_id as u32);
        }
    }

    for (local_idx, &pet_idx) in selected_indices.iter().enumerate() {
        let node_data = &pet_graph[pet_idx];
        idx_to_local.insert(pet_idx, local_idx);

        paths.push(node_data.path.clone());
        titles.push(node_data.title.clone());

        let deg = pet_graph.edges_directed(pet_idx, Direction::Incoming).count()
            + pet_graph.edges_directed(pet_idx, Direction::Outgoing).count();
        degrees.push(deg);

        let comm = node_to_comm.get(&node_data.path).copied().unwrap_or(0);
        communities.push(comm);
    }

    // Extract edges between selected nodes
    let mut raw_edges = Vec::new();
    for pet_edge in pet_graph.edge_references() {
        if let (Some(&src_loc), Some(&tgt_loc)) =
            (idx_to_local.get(&pet_edge.source()), idx_to_local.get(&pet_edge.target()))
        {
            let w = pet_edge.weight();
            raw_edges.push((src_loc, tgt_loc, w.edge_type.clone(), w.weight));
        }
    }

    let config = LayoutConfig {
        iterations: if paths.len() > 10000 { 25 } else { 40 },
        cluster_mode,
        anchor_strength: match cluster_mode {
            ClusterMode::Directory => 0.25,
            ClusterMode::Community => 0.22,
        },
        ..Default::default()
    };

    let (positions, edges) =
        compute_force_layout(&paths, &titles, &degrees, &communities, &raw_edges, &config);

    let nodes = paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            let (entity_type, color_rgb) =
                assign_color_and_type(&path, communities[i], Some(&snapshot.ast_types));
            let deg = degrees[i];
            let size = 2.0 + (deg as f32).sqrt().min(15.0);

            NodeLayout {
                id: i as u32,
                path,
                title: titles[i].clone(),
                position: positions[i],
                degree: deg,
                community: communities[i],
                entity_type,
                color_rgb,
                size,
            }
        })
        .collect();

    GraphLayout {
        corpus: snapshot.name.clone(),
        nodes,
        edges,
        communities_count: community_res.communities.len().max(1),
    }
}

/// Compute Tier 2: Local Ego Subgraph around a focal node path up to `max_hops`.
pub fn build_tier_2_local(
    snapshot: &CorpusSnapshot,
    center_path: &str,
    max_hops: usize,
    cluster_mode: ClusterMode,
) -> Option<GraphLayout> {
    let pet_graph = snapshot.graph.inner();
    let center_idx = snapshot.graph.get_node(center_path)?;

    let mut visited: HashSet<petgraph::graph::NodeIndex> = HashSet::new();
    let mut queue: VecDeque<(petgraph::graph::NodeIndex, usize)> = VecDeque::new();

    visited.insert(center_idx);
    queue.push_back((center_idx, 0));

    // Breadth-First Search up to max_hops
    while let Some((curr, hops)) = queue.pop_front() {
        if hops >= max_hops {
            continue;
        }

        for edge in pet_graph.edges_directed(curr, Direction::Outgoing) {
            let target = edge.target();
            if visited.insert(target) {
                queue.push_back((target, hops + 1));
            }
        }
        for edge in pet_graph.edges_directed(curr, Direction::Incoming) {
            let source = edge.source();
            if visited.insert(source) {
                queue.push_back((source, hops + 1));
            }
        }
    }

    let mut paths = Vec::new();
    let mut titles = Vec::new();
    let mut degrees = Vec::new();
    let mut communities = Vec::new();
    let mut idx_to_local = HashMap::new();

    for (local_idx, &pet_idx) in visited.iter().enumerate() {
        let node_data = &pet_graph[pet_idx];
        idx_to_local.insert(pet_idx, local_idx);

        paths.push(node_data.path.clone());
        titles.push(node_data.title.clone());

        let deg = pet_graph.edges_directed(pet_idx, Direction::Incoming).count()
            + pet_graph.edges_directed(pet_idx, Direction::Outgoing).count();
        degrees.push(deg);
        communities.push(if pet_idx == center_idx { 1 } else { 0 });
    }

    let mut raw_edges = Vec::new();
    for &pet_idx in &visited {
        for edge in pet_graph.edges_directed(pet_idx, Direction::Outgoing) {
            if let (Some(&src_loc), Some(&tgt_loc)) =
                (idx_to_local.get(&pet_idx), idx_to_local.get(&edge.target()))
            {
                let w = edge.weight();
                raw_edges.push((src_loc, tgt_loc, w.edge_type.clone(), w.weight));
            }
        }
    }

    let config = LayoutConfig {
        iterations: 45,
        spring_length: 35.0,
        cluster_mode,
        anchor_strength: 0.2,
        ..Default::default()
    };

    let (positions, edges) =
        compute_force_layout(&paths, &titles, &degrees, &communities, &raw_edges, &config);

    let nodes = paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            let is_center = path == center_path;
            let (entity_type, mut color) =
                assign_color_and_type(&path, communities[i], Some(&snapshot.ast_types));
            if is_center {
                color = 0xf59e0b; // Bright amber for focal node
            }
            let deg = degrees[i];
            let size = if is_center { 12.0 } else { 4.0 + (deg as f32).sqrt().min(8.0) };

            NodeLayout {
                id: i as u32,
                path,
                title: titles[i].clone(),
                position: positions[i],
                degree: deg,
                community: communities[i],
                entity_type,
                color_rgb: color,
                size,
            }
        })
        .collect();

    Some(GraphLayout { corpus: snapshot.name.clone(), nodes, edges, communities_count: 2 })
}
