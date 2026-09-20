//! Search service and algorithmic search index ports.

use crate::types::{
    BinaryFingerprint, FingerprintRecord, Modality, SearchDepth, SearchExplanation, SearchResult,
};
use crate::Result;

/// Query options for the [`SearchService`] port.
///
/// Carries every input the search-mode dispatch needs, mirroring the MCP
/// `search` tool's parameter semantics exactly. The consumer (the MCP adapter)
/// parses raw request JSON into this struct; the service owns the per-mode
/// defaults, fallbacks, and dispatch. Fields that are only meaningful for a
/// subset of modes are still carried unconditionally — the service applies the
/// mode-specific default (e.g. `graph_depth` defaults to 2 for `hybrid`/`explain`
/// and 3 for `graph`; `edge_class` defaults to `Semantic` for `hybrid`,
/// `Structural` for `graph`, and `None` for `explain`).
///
/// This opts type lives in `ctxvault-common` — alongside the port trait it feeds
/// — because it references only [`crate::types`] / [`crate::config`] domain types
/// (`Modality`, `SearchDepth`) and standard-library types, forcing no heavy
/// dependency onto the dependency-light crate. Verbosity/detail (`detail=ids`
/// snippet stripping) is deliberately absent: it shapes the outbound JSON and
/// stays an adapter concern, not a search-dispatch concern.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// The raw query text.
    pub query: String,
    /// Retrieval mode: `bm25`, `semantic`, `hybrid`, `graph`, `explain`, or `fast`.
    ///
    /// `None` selects the default (`hybrid`). `fast` executes sub-minute CPU SIF +
    /// 256-bit MRL binary Hamming scan + Query-Time PPR with zero ONNX neural inference.
    /// Any unrecognized value is an error, reproduced by the service.
    pub mode: Option<String>,
    /// Maximum number of results to return. `None` defaults to 10.
    pub limit: Option<usize>,
    /// Modality filter (docs | code | both), threaded through every mode.
    pub modality: Modality,
    /// Semantic-search depth (precise | broad | adaptive). Only used by `semantic`.
    pub depth: SearchDepth,
    /// Graph traversal depth. `None` takes the per-mode default (2 for
    /// `hybrid`/`explain`, 3 for `graph`).
    pub graph_depth: Option<usize>,
    /// Optional edge-type filter for graph-aware modes.
    pub edge_types: Option<Vec<String>>,
    /// Optional edge-class filter (`code` | `semantic` | `structural` | `crossmodal` | `hybrid`) as a raw
    /// string. The service applies the per-mode default when this is `None`.
    pub edge_class: Option<String>,
    /// Whether to run multi-hop query decomposition (`hybrid` mode only).
    pub decompose: Option<bool>,
    /// Number of top-ranked search results to inline source snippets for in Turn 1.
    pub snippets: Option<usize>,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            mode: None,
            limit: None,
            modality: Modality::Both,
            depth: SearchDepth::default(),
            graph_depth: None,
            edge_types: None,
            edge_class: None,
            decompose: None,
            snippets: None,
        }
    }
}

/// Port for high-throughput algorithmic semantic search and fingerprinting.
pub trait AlgorithmicSearchIndex: Send + Sync {
    /// Add or update binary fingerprints for extracted code symbols or doc chunks.
    fn index_fingerprints(&mut self, records: &[FingerprintRecord]) -> Result<()>;

    /// Perform a high-speed linear SIMD Hamming scan across all registered fingerprints.
    fn search_hamming(
        &self,
        query_bits: &BinaryFingerprint,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<(String, u32)>>;

    /// Project a text query into a 256-bit binary fingerprint via SIF.
    fn project_query(&self, query: &str) -> Result<BinaryFingerprint>;

    /// Clear all registered fingerprints.
    fn clear(&mut self);

    /// Total number of indexed fingerprints.
    fn len(&self) -> usize;

    /// Whether the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Search service port: the search-mode dispatch + RRF fusion contract.
///
/// This is the domain-facing contract for the retrieval dispatch that selects a
/// mode (`bm25` | `semantic` | `hybrid` | `graph` | `explain` | `fast`) and fuses signals
/// via RRF. Every signature speaks only [`SearchQuery`] and [`crate::types`]
/// domain results ([`SearchResult`], [`SearchExplanation`]) — no backend type
/// (`tantivy::*`, `hnsw_rs::*`, `petgraph::*`, `ort::*`) crosses this boundary,
/// so consumers dispatch a search without naming any concrete adapter.
///
/// # Two methods, two result shapes
///
/// The `explain` mode returns a different, richer shape
/// ([`SearchExplanation`], with per-signal score breakdown) than the other four
/// modes ([`SearchResult`]). Rather than fold the two into one type, the port
/// exposes them as separate methods:
///
/// - [`SearchService::search`] handles `bm25`, `semantic`, `hybrid`, `graph`, and `fast`,
///   returning `Vec<SearchResult>`.
/// - [`SearchService::explain`] handles `explain`, returning
///   `Vec<SearchExplanation>`.
///
/// A caller routes on [`SearchQuery::mode`] and calls the matching method; the
/// implementation still validates the mode and reproduces the exact
/// invalid-mode error for a mode that does not belong to the method invoked.
///
/// # Generic-vs-`dyn` wiring
///
/// Consistent with the per-port decision recorded above: `SearchService` sits on
/// the retrieval hot path and there is **no runtime-swap seam** for the dispatch
/// today, so its implementation is a concrete struct wired as a
/// **generic type parameter with this trait bound** — monomorphized, zero-cost,
/// statically dispatched — never `Arc<dyn SearchService>`. The trait exists to
/// keep consumers (the MCP layer) depending on the contract rather than on the
/// core implementation's internals.
pub trait SearchService {
    /// Dispatch a `bm25` / `semantic` / `hybrid` / `graph` / `fast` search, returning
    /// ranked [`SearchResult`]s (before any detail/verbosity shaping).
    ///
    /// Returns an error if `query.mode` is `explain` (use
    /// [`SearchService::explain`]) or an unrecognized mode.
    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>>;

    /// Dispatch an `explain` search, returning per-result score breakdowns as
    /// [`SearchExplanation`]s (before any detail/verbosity shaping).
    fn explain(&self, query: &SearchQuery) -> Result<Vec<SearchExplanation>>;

    /// Related search: given seed document paths, find the documents most
    /// related to them via a multi-source BFS approximation of Personalized
    /// PageRank over the knowledge graph.
    ///
    /// Traverses only the graph (never the lexical or vector signals),
    /// restricting results to the requested [`Modality`], and returns ranked
    /// [`SearchResult`]s (before any detail/verbosity shaping).
    fn search_related(
        &self,
        seeds: &[String],
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<SearchResult>>;
}
