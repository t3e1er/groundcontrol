//! Graph density analysis: hubs, orphans, and community connection statistics.

use serde::{Deserialize, Serialize};

use crate::graph::KnowledgeGraph;

/// Result of graph density analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DensityReport {
    /// Total number of nodes in the graph.
    pub total_nodes: usize,
    /// Total number of edges.
    pub total_edges: usize,
    /// Overall graph density (edges / max_possible_edges).
    pub density: f64,
    /// Nodes with no edges (orphans).
    pub orphans: Vec<String>,
    /// Top N most-connected nodes (hubs).
    pub hubs: Vec<HubInfo>,
    /// Density breakdown per tag (if tags available).
    pub tag_density: Vec<TagDensity>,
    /// Per-community density statistics (from Louvain detection).
    pub community_stats: Vec<CommunityDensityInfo>,
}

/// Information about a hub node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubInfo {
    /// Node path.
    pub path: String,
    /// Total degree (in + out edges).
    pub degree: usize,
    /// Inbound edge count.
    pub in_degree: usize,
    /// Outbound edge count.
    pub out_degree: usize,
}

/// Density information for a specific tag group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagDensity {
    /// Tag name.
    pub tag: String,
    /// Number of nodes with this tag.
    pub node_count: usize,
    /// Number of edges between nodes sharing this tag.
    pub internal_edges: usize,
}

/// Per-community density statistics for the density report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityDensityInfo {
    /// Community id.
    pub community_id: usize,
    /// Number of members in this community.
    pub member_count: usize,
    /// Number of internal edges within the community.
    pub internal_edges: usize,
    /// Density of internal connections.
    pub density: f64,
}

/// Analyze graph density, identifying hubs and orphans.
pub fn analyze_density(graph: &KnowledgeGraph, top_hubs: usize) -> DensityReport {
    let stats = graph.stats();

    let total_nodes = stats.node_count;
    let total_edges = stats.edge_count;

    // Density = edges / (nodes * (nodes - 1)) for directed graph.
    let max_edges = if total_nodes > 1 { total_nodes * (total_nodes - 1) } else { 1 };
    let density = total_edges as f64 / max_edges as f64;

    // Find orphans.
    let orphans = graph.orphan_paths();

    // Find hubs (most connected nodes) using node_degree_list.
    let degrees = graph.node_degree_list();
    let mut hubs: Vec<HubInfo> = degrees
        .into_iter()
        .map(|(path, in_deg, out_deg)| HubInfo {
            path,
            degree: in_deg + out_deg,
            in_degree: in_deg,
            out_degree: out_deg,
        })
        .collect();
    hubs.sort_by(|a, b| b.degree.cmp(&a.degree));
    hubs.truncate(top_hubs);

    // Compute per-community density statistics.
    let community_densities = graph.community_densities();
    let community_stats: Vec<CommunityDensityInfo> = community_densities
        .into_iter()
        .map(|cd| CommunityDensityInfo {
            community_id: cd.community_id,
            member_count: cd.node_count,
            internal_edges: cd.internal_edges,
            density: cd.density,
        })
        .collect();

    DensityReport {
        total_nodes,
        total_edges,
        density,
        orphans,
        hubs,
        tag_density: Vec::new(), // Tag density requires store access, left empty here.
        community_stats,
    }
}
