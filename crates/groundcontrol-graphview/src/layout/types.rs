//! 3D Graph layout data structures and configuration.

use serde::{Deserialize, Serialize};

/// Method used to group nodes into spatial cluster anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterMode {
    /// Group by filesystem directory module hierarchy (first 2–3 path components).
    Directory,
    /// Group by topological graph communities (Leiden/Louvain modularity).
    Community,
}

impl Default for ClusterMode {
    fn default() -> Self {
        Self::Directory
    }
}

/// 3D position and visual properties for a single graph node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLayout {
    /// Internal integer identifier (0..N).
    pub id: u32,
    /// Authoritative repository-relative path or symbol signature.
    pub path: String,
    /// Document or symbol display title.
    pub title: Option<String>,
    /// Precomputed 3D Cartesian coordinates [x, y, z].
    pub position: [f32; 3],
    /// Total node degree (in-degree + out-degree).
    pub degree: usize,
    /// Community cluster index (from Leiden/Louvain).
    pub community: u32,
    /// Entity class/kind (DocNode, Function, Struct, Module, etc.).
    pub entity_type: String,
    /// Packed RGB color (0xRRGGBB).
    pub color_rgb: u32,
    /// Display radius size for WebGL shaders.
    pub size: f32,
}

/// Visual connection between two nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeLayout {
    /// Source node identifier.
    pub source: u32,
    /// Target node identifier.
    pub target: u32,
    /// Relationship type name (e.g. calls, defines, imports, wikilink).
    pub edge_type: String,
    /// Normalized edge weight.
    pub weight: f32,
}

/// Fully computed 3D scene payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphLayout {
    /// Corpus identifier or "all" for multi-corpus galaxy.
    pub corpus: String,
    /// List of placed nodes.
    pub nodes: Vec<NodeLayout>,
    /// List of placed edges.
    pub edges: Vec<EdgeLayout>,
    /// Total distinct communities identified.
    pub communities_count: usize,
}

/// Configuration parameters for the 3D force layout simulation.
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// Number of simulation relaxation steps.
    pub iterations: usize,
    /// Repulsion constant ($k_{\text{rep}}$).
    pub repulsion: f32,
    /// Spring stiffness constant.
    pub spring_stiffness: f32,
    /// Ideal spring rest length.
    pub spring_length: f32,
    /// Velocity damping factor (0.0..1.0).
    pub damping: f32,
    /// Barnes-Hut opening angle threshold $\theta$ (typically 0.8).
    pub theta: f32,
    /// Center gravity pulling towards coordinate origin `[0, 0, 0]`.
    pub center_gravity: f32,
    /// Anchor spring stiffness pulling nodes to their module/community center.
    pub anchor_strength: f32,
    /// Active clustering mode (Directory or Community).
    pub cluster_mode: ClusterMode,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            iterations: 40,
            repulsion: 6000.0,
            spring_stiffness: 0.06,
            spring_length: 45.0,
            damping: 0.85,
            theta: 0.8,
            center_gravity: 0.001,
            anchor_strength: 0.22,
            cluster_mode: ClusterMode::Directory,
        }
    }
}
