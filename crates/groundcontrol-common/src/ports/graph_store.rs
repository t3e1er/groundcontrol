//! Knowledge graph store port.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::config::{EdgeClass, EdgeTypeConfig};
use crate::types::{
    BrokenLink, CircularDependency, CommunityDensity, CommunityDetectionResult, Document, Edge,
    EdgeProvenance, GraphAffordances, GraphStats, LineageAnnotation, LineageNode, OrphanAdr,
    ResolutionConfidence,
};
use crate::Result;

/// Graph store port: the typed knowledge-graph contract for a corpus.
///
/// This is the domain-facing contract for the Petgraph-backed `KnowledgeGraph`.
/// It covers node/edge mutation, edge construction from parsed documents,
/// traversal (BFS, shortest path, lineage), backlink/forwardlink queries,
/// taxonomy validation (broken links, cycles, orphan ADRs), community
/// detection, statistics, and persistence. Every signature speaks only
/// [`crate::types`] / [`crate::config`] domain types and standard-library
/// types — no backend type (`petgraph::NodeIndex`, `DiGraph`, …) ever crosses
/// this boundary, so consumers depend on the contract rather than on Petgraph.
///
/// # Exclusions (surface deliberately not on the port)
///
/// - **Construction and loading (`new`, `load`).** Both return `Self` — a
///   `&dyn`-object-safe trait cannot express that, and building a graph (empty
///   or deserialized from `graph.bin`) is an adapter/composition-root concern,
///   not runtime behaviour of an existing store. `save` (`&self`) *is* on the
///   port because persisting current state is runtime behaviour; loading stays
///   an inherent method on the concrete adapter.
/// - **`get_node`.** It returns a backend `NodeIndex` and has no non-test
///   callers; every existence check it served is covered by
///   [`GraphStore::contains_node`], so it is excluded entirely (a backend type
///   must never leak across the port).
/// - **`add_node` returns `()` here, not the backend `NodeIndex`.** The sole
///   external caller discards the index, and the internal consumers of that
///   index live inside the adapter's own edge-construction methods — off the
///   port.
///
/// Some inherent methods on the adapter (e.g. `traverse_dfs`, `orphan_paths`,
/// `node_degree_list`, `ensure_node`) have no non-test callers today and are
/// intentionally omitted from this contract; the port mirrors exactly the
/// surface consumers use.
pub trait GraphStore {
    // ------------------------------------------------------------------
    // Node / edge mutation
    // ------------------------------------------------------------------

    /// Add or update a node for the given path (with an optional title).
    ///
    /// Idempotent: re-adding an existing path updates its title in place.
    fn add_node(&mut self, path: &str, title: Option<&str>);

    /// Add a directed intra-corpus edge between two nodes.
    ///
    /// Creates the target node if it is missing. Thin wrapper over
    /// [`GraphStore::add_edge_full`] with no cross-corpus metadata.
    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        class: EdgeClass,
    );

    /// Add a directed edge carrying optional cross-corpus resolution metadata.
    ///
    /// `target_corpus` / `confidence` are `None` for ordinary intra-corpus edges
    /// and `Some(_)` when the target was resolved in another corpus. Parallel
    /// edges of the same `edge_type` between the same nodes are de-duplicated
    /// (updated in place), keeping this operation idempotent.
    fn add_edge_full(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        class: EdgeClass,
        target_corpus: Option<String>,
        confidence: Option<ResolutionConfidence>,
    );

    /// Add a cross-corpus edge carrying the full remote-endpoint payload.
    ///
    /// Extends [`GraphStore::add_edge_full`] with where the target lives
    /// (`target_path`), what it is called (`target_symbol`), and its free-form
    /// kind (`target_kind`, e.g. `"Symbol"`, `"Route"`, `"Channel"`) so a
    /// federated traversal can report and continue the hop without re-resolving.
    /// Shares the same same-type de-duplication, keeping the operation idempotent.
    fn add_cross_corpus_edge(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        class: EdgeClass,
        target_corpus: Option<String>,
        confidence: Option<ResolutionConfidence>,
        target_path: Option<String>,
        target_symbol: Option<String>,
        target_kind: Option<String>,
    );

    /// Add a code edge into the graph with a structural edge class.
    fn add_code_edge(&mut self, edge: &Edge);

    /// Remove a node and all of its edges.
    fn remove_node(&mut self, path: &str) -> Result<()>;

    /// Remove all edges where the given path is source or target.
    fn remove_edges_for_node(&mut self, path: &str);

    // ------------------------------------------------------------------
    // Node / edge queries
    // ------------------------------------------------------------------

    /// Whether a node with the given path exists in the graph.
    fn contains_node(&self, path: &str) -> bool;

    /// Enumerate a node's outgoing frontmatter-provenance edges as
    /// `(edge_type, raw_target)` pairs (used by the cross-corpus resolver).
    fn outgoing_frontmatter_targets(&self, path: &str) -> Vec<(String, String)>;

    /// Enumerate all node paths currently in the graph.
    fn node_paths(&self) -> Vec<String>;

    /// Number of nodes.
    fn node_count(&self) -> usize;

    /// Number of edges.
    fn edge_count(&self) -> usize;

    /// Retrieve all edges currently in the knowledge graph.
    fn get_all_edges(&self) -> Vec<Edge>;

    /// Retrieve a single node's outgoing edges, carrying the full cross-corpus
    /// payload (`target_corpus`, `confidence`, `target_path`, `target_symbol`,
    /// `target_kind`).
    ///
    /// An O(deg) per-node lookup (versus scanning every edge via
    /// [`GraphStore::get_all_edges`]) used to expand one node per hop during a
    /// federated cross-corpus traversal. Returns an empty vector when the node
    /// is absent.
    fn outgoing_edges(&self, path: &str) -> Vec<Edge>;

    // ------------------------------------------------------------------
    // Edge construction from documents
    // ------------------------------------------------------------------

    /// Build edges for a parsed document from the given edge-type configs.
    ///
    /// Processes wikilinks, shared tags, and frontmatter fields.
    fn build_edges_for_document(
        &mut self,
        doc: &Document,
        edge_configs: &[EdgeTypeConfig],
        all_docs: &[Document],
    );

    /// Build tag edges across all documents using an inverted tag index.
    fn build_all_tag_edges(&mut self, configs: &[EdgeTypeConfig], all_docs: &[Document]);

    // ------------------------------------------------------------------
    // Traversal & queries
    // ------------------------------------------------------------------

    /// BFS from a starting node up to `max_depth` hops.
    ///
    /// Optionally filtered by edge type and/or edge class. Returns
    /// `(path, hops_from_start)` pairs.
    fn traverse_bfs(
        &self,
        start: &str,
        max_depth: usize,
        edge_type_filter: Option<&[String]>,
        edge_class_filter: Option<EdgeClass>,
    ) -> Vec<(String, usize)>;

    /// All notes that link TO this note, grouped by edge type.
    fn backlinks(
        &self,
        path: &str,
        edge_class_filter: Option<EdgeClass>,
    ) -> HashMap<String, Vec<String>>;

    /// All notes this note links TO, grouped by edge type.
    fn forwardlinks(
        &self,
        path: &str,
        edge_class_filter: Option<EdgeClass>,
    ) -> HashMap<String, Vec<String>>;

    /// Shortest path between two nodes, optionally filtered by edge type/class.
    ///
    /// Returns the path as a list of document paths, or `None` if unreachable.
    fn shortest_path(
        &self,
        from: &str,
        to: &str,
        edge_type_filter: Option<&[String]>,
        edge_class_filter: Option<EdgeClass>,
    ) -> Option<Vec<String>>;

    /// Graph statistics (node/edge counts, orphans, most-connected, distribution).
    fn stats(&self) -> GraphStats;

    /// Compute direct degree affordances for a node (O(deg) lookup).
    fn compute_affordances(&self, path: &str) -> GraphAffordances;

    /// Return the total in-degree of a node directly without allocating affordance maps.
    fn in_degree(&self, _path: &str) -> usize {
        0
    }

    // ------------------------------------------------------------------
    // Structural lineage & taxonomy
    // ------------------------------------------------------------------

    /// Deterministically traverse the graph along a structural edge type.
    ///
    /// `direction` is `"outgoing"`, `"incoming"`, or `"both"`. Returns the
    /// ordered lineage chain, starting with the start node at depth 0.
    fn traverse_lineage(
        &self,
        start: &str,
        edge_type: &str,
        direction: &str,
        max_depth: usize,
    ) -> Vec<LineageNode>;

    /// Extract active structural lineage metadata for a node, if any.
    fn extract_lineage_for_node(&self, path: &str) -> Option<LineageAnnotation>;

    /// Detect broken structural links (targets absent from `existing_paths`).
    fn detect_broken_links(&self, existing_paths: &HashSet<String>) -> Vec<BrokenLink>;

    /// Detect circular dependencies within the given directed-acyclic relations.
    fn detect_circular_dependencies(&self, edge_types: &[&str]) -> Vec<CircularDependency>;

    /// Detect ADR notes with no inbound or outbound structural links.
    fn detect_orphan_adrs(&self, adr_paths: &[String]) -> Vec<OrphanAdr>;

    // ------------------------------------------------------------------
    // Community detection
    // ------------------------------------------------------------------

    /// Detect communities using the Louvain modularity-based algorithm.
    fn detect_communities(&self) -> CommunityDetectionResult;

    /// Detect communities with a Leiden-style connectivity-refinement pass.
    fn detect_communities_leiden(&self) -> CommunityDetectionResult;

    /// Per-community density statistics for the current partition.
    fn community_densities(&self) -> Vec<CommunityDensity>;

    // ------------------------------------------------------------------
    // Persistence
    // ------------------------------------------------------------------

    /// Serialize the graph to a file at the given path.
    fn save(&self, path: &Path) -> Result<()>;
}
