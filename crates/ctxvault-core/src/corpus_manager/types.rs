use serde::{Deserialize, Serialize};

use ctxvault_common::types::ResolutionConfidence;

/// Which resolver tier produced a cross-corpus match, ordered by trust
/// (highest first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolverKind {
    /// Resolved via a SCIP moniker (compiler-grade, highest trust).
    Scip,
    /// Reserved for in-engine hybrid-LSP cross-corpus resolution (not yet live).
    HybridLsp,
    /// Resolved via qualified-name matching against the SQLite symbol catalog
    /// (always-available fallback).
    QualName,
}

/// Status information for a single corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusInfo {
    /// Corpus name.
    pub name: String,
    /// Path to the corpus directory.
    pub path: String,
    /// Access mode (read-write or read-only).
    pub mode: String,
    /// Indexing mode (Full, DocsEmbed, or Fast).
    pub index_mode: String,
    /// Number of indexed files.
    pub file_count: usize,
    /// Whether the embedder is active for this corpus.
    pub embedder_active: bool,
    /// Number of vectors in the index.
    pub vector_count: usize,
    /// Number of nodes in the knowledge graph.
    pub graph_node_count: usize,
}

/// One intra-corpus node visited during a [`FederatedTraversal`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedNode {
    /// Corpus this node lives in.
    pub corpus: String,
    /// Node key (path / scope_path / route key) within `corpus`.
    pub node: String,
    /// BFS depth within `corpus` from the node the traversal entered it at.
    pub depth: usize,
}

/// One cross-corpus hop encountered during a federated traversal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusHop {
    /// Corpus the crossed edge originates in.
    pub from_corpus: String,
    /// Origin node (in `from_corpus`) carrying the cross-corpus edge.
    pub from_node: String,
    /// `target_corpus` of the crossed edge.
    pub to_corpus: String,
    /// Resolved real node in `to_corpus`, if any.
    pub to_node: Option<String>,
    /// Edge type crossed (`"calls"`, `"imports"`, …).
    pub edge_type: String,
    /// Free-form kind of the remote endpoint (e.g. `"Symbol"`), if the edge carried one.
    pub target_kind: Option<String>,
    /// Resolution-confidence band recorded on the crossed edge, if any.
    pub confidence: Option<ResolutionConfidence>,
    /// Hop index in the corpus-hop sequence (`1` = the first cross-corpus hop).
    pub corpus_depth: usize,
}

/// Result of a [`CorpusManager::federated_traverse`] call.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FederatedTraversal {
    /// Intra-corpus nodes visited, tagged with their corpus and hop depth.
    pub nodes: Vec<FederatedNode>,
    /// Cross-corpus hops encountered.
    pub hops: Vec<CorpusHop>,
}
