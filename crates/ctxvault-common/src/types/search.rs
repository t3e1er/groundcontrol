//! Search domain types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::code::{EntityKind, Modality};
use super::graph::{GraphAffordances, SchemaEnvelope};

/// A 256-bit Matryoshka binary fingerprint packed into four 64-bit words (32 bytes).
///
/// Quantized from the leading 256 dimensions of a continuous embedding or SIF projection
/// using 1-bit sign thresholding (`bit_i = 1` if `v_i > 0.0` else `0`). Evaluated via
/// single-cycle bitwise XOR and native CPU POPCOUNT (`count_ones()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct BinaryFingerprint(pub [u64; 4]);

impl BinaryFingerprint {
    /// Quantize a 256-dimensional float vector into 256 bits using 1-bit sign thresholding.
    #[inline]
    pub fn from_f32_slice(v: &[f32; 256]) -> Self {
        let mut bits = [0u64; 4];
        for i in 0..256 {
            if v[i] > 0.0 {
                bits[i / 64] |= 1u64 << (i % 64);
            }
        }
        BinaryFingerprint(bits)
    }

    /// Exact Hamming distance using native CPU POPCOUNT.
    ///
    /// Evaluates bitwise XOR across four 64-bit words. LLVM automatically emits
    /// hardware POPCNT instructions with zero unsafe code.
    #[inline]
    pub fn hamming_distance(&self, other: &Self) -> u32 {
        let d0 = (self.0[0] ^ other.0[0]).count_ones();
        let d1 = (self.0[1] ^ other.0[1]).count_ones();
        let d2 = (self.0[2] ^ other.0[2]).count_ones();
        let d3 = (self.0[3] ^ other.0[3]).count_ones();
        d0 + d1 + d2 + d3
    }

    /// Normalized similarity score in `[0.0, 1.0]`.
    #[inline]
    pub fn similarity(&self, other: &Self) -> f32 {
        1.0 - (self.hamming_distance(other) as f32 / 256.0)
    }

    /// Serialize fingerprint to 32 raw bytes (little-endian).
    #[inline]
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        for (i, word) in self.0.iter().enumerate() {
            bytes[i * 8..(i + 1) * 8].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    /// Deserialize fingerprint from 32 raw bytes (little-endian).
    #[inline]
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let mut bits = [0u64; 4];
        for i in 0..4 {
            let mut word_bytes = [0u8; 8];
            word_bytes.copy_from_slice(&bytes[i * 8..(i + 1) * 8]);
            bits[i] = u64::from_le_bytes(word_bytes);
        }
        BinaryFingerprint(bits)
    }
}

/// An indexed 256-bit binary fingerprint record with entity identifier and modality.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FingerprintRecord {
    /// Identifier (relative doc/code path or scoped symbol handle).
    pub id: String,
    /// 256-bit binary fingerprint.
    pub fingerprint: BinaryFingerprint,
    /// Entity modality (docs or code).
    pub modality: Modality,
}

/// Structural lineage metadata annotations for a retrieved document.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LineageAnnotation {
    /// Notes that supersede this note.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded_by: Vec<String>,
    /// Notes that this note supersedes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
    /// Notes implemented by this note.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implements: Vec<String>,
    /// Notes that implement this note.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implemented_by: Vec<String>,
    /// Notes that this note depends on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Notes that depend on this note.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depended_on_by: Vec<String>,
    /// Decisions that this note serves as an ADR for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adr_for: Vec<String>,
    /// ADRs that document this note.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub has_adr: Vec<String>,
    /// Parent notes in hierarchy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parent_of: Vec<String>,
    /// Child notes in hierarchy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_of: Vec<String>,
    /// All other active incoming structural links grouped by edge type.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub incoming: HashMap<String, Vec<String>>,
    /// All other active outgoing structural links grouped by edge type.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub outgoing: HashMap<String, Vec<String>>,
}

impl LineageAnnotation {
    /// Whether any lineage relations are present.
    pub fn is_empty(&self) -> bool {
        self.superseded_by.is_empty()
            && self.supersedes.is_empty()
            && self.implements.is_empty()
            && self.implemented_by.is_empty()
            && self.depends_on.is_empty()
            && self.depended_on_by.is_empty()
            && self.adr_for.is_empty()
            && self.has_adr.is_empty()
            && self.parent_of.is_empty()
            && self.child_of.is_empty()
            && self.incoming.is_empty()
            && self.outgoing.is_empty()
    }
}

/// A search result from any search strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Document or code entity path.
    pub path: String,
    /// Combined relevance score (0.0 - 1.0).
    pub score: f64,
    /// Text snippet showing the relevant passage.
    pub snippet: Option<String>,
    /// Which chunk matched (if applicable).
    pub chunk_index: Option<usize>,
    /// Score breakdown for explainability.
    pub score_components: Option<ScoreBreakdown>,
    /// Structural lineage annotations (e.g. superseded_by, implements).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<LineageAnnotation>,
    /// Entity kind for this search hit (e.g. Documentation, CodeSymbol, CodeChunk).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_kind: Option<EntityKind>,
    /// Programming language if this result is from code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Name of the corpus this result originated from.
    ///
    /// Populated by the multi-corpus routing layer; `None` for results that
    /// have not been tagged with a source corpus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corpus: Option<String>,
    /// Graph degree affordances for Turn 2 expansion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_affordances: Option<GraphAffordances>,
    /// Immediate 1-hop neighborhood in Cypher-Lite ASCII notation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
    /// Code symbol identifier (provided when snippet is omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

impl SearchResult {
    /// Create a basic search result.
    pub fn new(path: impl Into<String>, score: f64) -> Self {
        Self {
            path: path.into(),
            score,
            snippet: None,
            chunk_index: None,
            score_components: None,
            lineage: None,
            entity_kind: None,
            language: None,
            corpus: None,
            graph_affordances: None,
            graph: None,
            symbol: None,
        }
    }

    /// Set Cypher-Lite graph representation.
    pub fn with_graph(mut self, graph: Option<String>) -> Self {
        self.graph = graph;
        self
    }

    /// Set symbol identifier.
    pub fn with_symbol(mut self, symbol: Option<String>) -> Self {
        self.symbol = symbol;
        self
    }

    /// Set snippet text.
    pub fn with_snippet(mut self, snippet: Option<String>) -> Self {
        self.snippet = snippet;
        self
    }

    /// Set chunk index.
    pub fn with_chunk_index(mut self, chunk_index: Option<usize>) -> Self {
        self.chunk_index = chunk_index;
        self
    }

    /// Set score components breakdown.
    pub fn with_score_components(mut self, components: ScoreBreakdown) -> Self {
        self.score_components = Some(components);
        self
    }

    /// Set entity kind.
    pub fn with_entity_kind(mut self, entity_kind: EntityKind) -> Self {
        self.entity_kind = Some(entity_kind);
        self
    }

    /// Set language.
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Set the source corpus tag.
    pub fn with_corpus(mut self, corpus: Option<String>) -> Self {
        self.corpus = corpus;
        self
    }

    /// Set graph affordances.
    pub fn with_graph_affordances(mut self, affordances: GraphAffordances) -> Self {
        self.graph_affordances = Some(affordances);
        self
    }
}

/// Metadata about a stored vector, mapping HNSW internal IDs to documents/chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorMeta {
    /// Document path this vector belongs to.
    pub doc_path: String,
    /// Chunk index within the document (None for document-level embeddings).
    pub chunk_index: Option<usize>,
    /// Whether this is a document-level embedding (vs chunk-level).
    pub is_doc_level: bool,
    /// Coarse modality tag ("code" / "docs") for modality-filtered search.
    #[serde(default = "default_modality")]
    pub modality: String,
}

/// Default coarse modality tag ("docs") for round-tripping persisted vectors.
fn default_modality() -> String {
    "docs".to_string()
}

/// A single vector search result.
#[derive(Debug, Clone)]
pub struct VectorSearchResult {
    /// Document path.
    pub doc_path: String,
    /// Chunk index (None for document-level).
    pub chunk_index: Option<usize>,
    /// Cosine similarity score (0.0 to 1.0, higher = more similar).
    pub score: f64,
    /// Whether this came from a document-level embedding.
    pub is_doc_level: bool,
    /// Coarse modality tag ("code" / "docs") of the matched vector.
    pub modality: String,
}

fn is_zero_f64(v: &f64) -> bool {
    v.abs() < 1e-9
}

/// Breakdown of how a search score was computed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    /// BM25 component (0.0 if not applicable).
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub bm25: f64,
    /// Vector cosine similarity component.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub vector: f64,
    /// Graph proximity boost.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub graph_boost: f64,
    /// Number of hops from seed in graph traversal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_hops: Option<usize>,
}

impl ScoreBreakdown {
    /// Returns true if all numerical components are zero and graph_hops is None.
    pub fn is_empty(&self) -> bool {
        is_zero_f64(&self.bm25)
            && is_zero_f64(&self.vector)
            && is_zero_f64(&self.graph_boost)
            && self.graph_hops.is_none()
    }
}

/// Depth level for dual-level retrieval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchDepth {
    /// Chunk-level only — best for specific factual queries.
    #[default]
    Precise,
    /// Document-level only — best for "what do we know about X?" sensemaking.
    Broad,
    /// Both chunk and doc-level, merged with RRF — default.
    Adaptive,
}

impl SearchDepth {
    /// Parse from a string (case-insensitive).
    pub fn from_str_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "precise" => Some(Self::Precise),
            "broad" => Some(Self::Broad),
            "adaptive" => Some(Self::Adaptive),
            _ => None,
        }
    }
}

/// Detailed explanation of how a search result was scored.
/// Richer than `ScoreBreakdown` — includes per-signal rank and RRF contributions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchExplanation {
    /// Document path.
    pub path: String,
    /// Final fused score (RRF total).
    pub final_score: f64,
    /// BM25 component details.
    pub bm25: SignalExplanation,
    /// Vector similarity component details.
    pub vector: SignalExplanation,
    /// Graph proximity component details.
    pub graph: GraphExplanation,
    /// Text snippet from the matched chunk.
    pub snippet: Option<String>,
    /// Which chunk matched (if applicable).
    pub chunk_index: Option<usize>,
}

/// Explanation of a single signal (BM25 or vector).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalExplanation {
    /// Raw score from this signal (BM25 score or cosine similarity).
    pub raw_score: f64,
    /// Rank position in this signal's result list (1-based, 0 if not present).
    pub rank: usize,
    /// RRF contribution from this signal: 1/(k + rank).
    pub rrf_contribution: f64,
}

/// Explanation of graph proximity signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphExplanation {
    /// Graph boost score (1/hop_distance accumulated).
    pub boost: f64,
    /// Minimum hops from any seed node.
    pub min_hops: Option<usize>,
    /// Rank position in graph signal's result list (1-based, 0 if not present).
    pub rank: usize,
    /// RRF contribution from graph signal.
    pub rrf_contribution: f64,
}

/// A partitioned search result set (docs or code).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchPartition {
    /// Hits in this partition.
    pub results: Vec<SearchResult>,
    /// Total matches across the corpus in this modality.
    pub total_matches: usize,
    /// Number of hits returned in `results`.
    pub top_k_returned: usize,
    /// Schema envelope providing information scent for Turn 2 expansion.
    pub schema_envelope: SchemaEnvelope,
}

/// Partitioned bimodal search response.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResponse {
    /// Documentation partition (if requested/available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<SearchPartition>,
    /// Source code partition (if requested/available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<SearchPartition>,
}
