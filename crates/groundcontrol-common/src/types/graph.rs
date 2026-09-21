//! Graph domain types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::edge::EdgeProvenance;

/// Graph statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    /// Total number of nodes.
    pub node_count: usize,
    /// Total number of edges.
    pub edge_count: usize,
    /// Nodes with zero edges (neither incoming nor outgoing).
    pub orphan_count: usize,
    /// Top 10 nodes by total degree (incoming + outgoing).
    pub most_connected: Vec<(String, usize)>,
    /// Count of edges per edge type.
    pub edge_type_distribution: HashMap<String, usize>,
}

/// A step or node in a lineage traversal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineageNode {
    /// Document path.
    pub path: String,
    /// Document title if known.
    pub title: Option<String>,
    /// Hop distance from the start node (0 for start note).
    pub depth: usize,
    /// Edge type traversed to reach this note.
    pub edge_type: String,
    /// Direction traversed ("start", "outgoing", "incoming").
    pub direction: String,
}

/// Broken link detected in taxonomy validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokenLink {
    /// Source document path containing the link.
    pub source: String,
    /// Target path or wikilink that could not be resolved.
    pub target: String,
    /// Edge type (e.g. "Wikilink", "supersedes", etc.).
    pub edge_type: String,
    /// Edge provenance.
    pub provenance: EdgeProvenance,
}

/// Circular dependency detected in a directed acyclic relation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CircularDependency {
    /// The edge type where the cycle exists (e.g. "supersedes").
    pub edge_type: String,
    /// The cycle path (e.g. ["A.md", "B.md", "A.md"]).
    pub cycle: Vec<String>,
}

/// Orphan ADR detected in taxonomy validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrphanAdr {
    /// Path to the orphan ADR note.
    pub path: String,
    /// Note title if known.
    pub title: Option<String>,
    /// Human-readable explanation.
    pub reason: String,
}

/// A detected community of nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Community {
    /// Community identifier.
    pub id: usize,
    /// Paths of nodes in this community.
    pub members: Vec<String>,
    /// Modularity contribution of this community to the overall partition.
    pub modularity_contribution: f64,
}

/// Result of community detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityDetectionResult {
    /// Detected communities.
    pub communities: Vec<Community>,
    /// Overall modularity of the partition (Q ∈ [-0.5, 1.0]).
    pub modularity: f64,
    /// Number of iterations the algorithm ran.
    pub iterations: usize,
}

/// Per-community density statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityDensity {
    /// Community id.
    pub community_id: usize,
    /// Number of nodes in the community.
    pub node_count: usize,
    /// Number of internal edges (edges within the community, treating as undirected).
    pub internal_edges: usize,
    /// Density: internal_edges / max_possible_internal_edges.
    pub density: f64,
}

/// Graph affordances for a search result node.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GraphAffordances {
    /// Inbound calls count (for code symbols).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calls_in: Option<usize>,
    /// Outbound calls count (for code symbols).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calls_out: Option<usize>,
    /// Implemented interfaces / traits count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implements: Option<usize>,
    /// Imported dependencies count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imports: Option<usize>,
    /// Inbound wikilinks count (for doc nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wikilinks_in: Option<usize>,
    /// Outbound wikilinks count (for doc nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wikilinks_out: Option<usize>,
    /// Connected code documentation links count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documents_code: Option<usize>,
    /// Dynamic counts for language-specific or extended edge types (e.g. decorates, extends, foreign_key).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub edge_counts: HashMap<String, usize>,
    /// Count of edges suppressed due to hub degree thresholds, keyed by edge type.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub suppressed_edges: HashMap<String, usize>,
}

impl GraphAffordances {
    /// Returns true if all affordance counts are None or zero, and edge_counts and suppressed_edges are empty.
    pub fn is_empty(&self) -> bool {
        self.calls_in.unwrap_or(0) == 0
            && self.calls_out.unwrap_or(0) == 0
            && self.implements.unwrap_or(0) == 0
            && self.imports.unwrap_or(0) == 0
            && self.wikilinks_in.unwrap_or(0) == 0
            && self.wikilinks_out.unwrap_or(0) == 0
            && self.documents_code.unwrap_or(0) == 0
            && self.edge_counts.is_empty()
            && self.suppressed_edges.is_empty()
    }
}

/// Contextual schema envelope returned with search partitions.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SchemaEnvelope {
    /// Distinct node labels/kinds relevant to this partition.
    pub node_labels: Vec<String>,
    /// Active edge types present in this partition.
    pub active_edges: Vec<String>,
}

/// Result of a `graph_match` traversal query formatted as a hierarchical branching tree.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphMatchResult {
    /// Root node from which the pattern query expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// Source file and line of the root entity if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// High-signal cardinality summary (direct & transitive impact, files affected, max depth).
    pub summary: GraphImpactSummary,
    /// Hierarchical branching tree of traversed paths.
    pub tree: Vec<GraphTreeNode>,
    /// Total matches / paths reached.
    pub total_matches: usize,
}

/// Summary metrics of graph impact / blast radius.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphImpactSummary {
    /// Number of immediate 1-hop connections.
    pub direct: usize,
    /// Total number of unique transitive nodes reached.
    pub transitive: usize,
    /// Number of unique files affected across the traversed subgraph.
    pub files: usize,
    /// Maximum hop depth reached.
    pub max_depth: usize,
}

/// A node in the hierarchical graph traversal tree.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphTreeNode {
    /// Node identifier or scope path.
    pub node: String,
    /// Relationship type leading to this node (e.g. "calls", "implements", "extends").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel: Option<String>,
    /// File path where this entity is defined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Starting line number in the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// Hop distance from the root.
    pub hop: usize,
    /// Child branches expanding from this node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<GraphTreeNode>,
    /// Number of suppressed branches if this node is a high-degree hub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppressed: Option<usize>,
    /// Whether this node was identified and capped as a high-degree hub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub: Option<bool>,
}
