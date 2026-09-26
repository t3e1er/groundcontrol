use petgraph::graph::DiGraph;
use serde::{Deserialize, Serialize};

use groundcontrol_common::config::EdgeClass;
use groundcontrol_common::types::{EdgeProvenance, ResolutionConfidence};

/// Node data stored in the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    /// Relative path within the corpus.
    pub path: String,
    /// Document title (if known).
    pub title: Option<String>,
}

/// Edge data stored in the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Edge type name (must match a registered `EdgeTypeConfig.name`).
    pub edge_type: String,
    /// Weight of this edge.
    pub weight: f32,
    /// How this edge was created.
    pub provenance: EdgeProvenance,
    /// Edge class for filtering purposes.
    pub class: EdgeClass,
    /// Name of the corpus the target lives in, for cross-corpus links.
    pub target_corpus: Option<String>,
    /// Confidence band for a resolved cross-corpus link (`None` for intra-corpus).
    pub confidence: Option<ResolutionConfidence>,
    /// Repo-relative path of the target endpoint in `target_corpus`.
    pub target_path: Option<String>,
    /// Fully qualified symbol / endpoint name at the remote target.
    pub target_symbol: Option<String>,
    /// Free-form kind of the remote endpoint (e.g. `"Symbol"`, `"Route"`, `"Channel"`, `"RpcEndpoint"`, `"Resource"`).
    pub target_kind: Option<String>,
}

/// On-disk schema version stamped into `GraphData`.
pub const GRAPH_SCHEMA_VERSION: u32 = 2;

/// Serializable wrapper for persistence.
#[derive(Serialize, Deserialize)]
pub(crate) struct GraphData {
    /// Schema version stamp; see [`GRAPH_SCHEMA_VERSION`].
    pub(crate) version: u32,
    /// The serialized directed graph.
    pub(crate) graph: DiGraph<GraphNode, GraphEdge>,
}

/// Borrowed serializable wrapper for zero-clone streaming persistence.
#[derive(Serialize)]
pub(crate) struct GraphDataRef<'a> {
    pub(crate) version: u32,
    pub(crate) graph: &'a DiGraph<GraphNode, GraphEdge>,
}
