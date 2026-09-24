//! Cypher-Lite AST types.

use std::collections::HashMap;

/// Direction of an edge in the query pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryDirection {
    /// Outbound: `-[...]->`
    Outgoing,
    /// Inbound: `<-[...]-`
    Incoming,
    /// Bidirectional / Undirected: `-[...]-`
    Undirected,
}

/// A parsed node pattern, e.g. `(c:CodeSymbol {name: 'SelectVictimsOnNode'})`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NodePattern {
    /// Optional variable name (e.g. `c`, `doc`).
    pub variable: Option<String>,
    /// Optional node label (e.g. `CodeSymbol`, `DocNode`, `Interface`).
    pub label: Option<String>,
    /// Exact property match filters (e.g. `name: '...'`).
    pub properties: HashMap<String, String>,
}

/// A parsed edge pattern, e.g. `<-[:calls|implements*1..2]-`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgePattern {
    /// Traversal direction relative to preceding node.
    pub direction: QueryDirection,
    /// Allowed edge types (empty means any edge type).
    pub edge_types: Vec<String>,
    /// Minimum hops (inclusive, default 1).
    pub min_hops: usize,
    /// Maximum hops (inclusive, default 1).
    pub max_hops: usize,
}

/// A full linear path pattern, e.g. `(A)-[:implements]->(B)<-[:calls*1..2]-(C)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPattern {
    /// Starting node pattern (anchor).
    pub start_node: NodePattern,
    /// Chained edge and subsequent node steps.
    pub steps: Vec<(EdgePattern, NodePattern)>,
}
