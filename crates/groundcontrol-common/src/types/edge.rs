//! Edge domain types.

use serde::{Deserialize, Serialize};

/// A registered edge type configuration persisted to the database.
#[derive(Debug, Clone)]
pub struct EdgeTypeRecord {
    /// Name of the edge type.
    pub name: String,
    /// Source kind (e.g. "wikilink", "tag", "frontmatter", "reference").
    pub source: String,
    /// Weight for scoring.
    pub weight: f32,
    /// Whether edges of this type are created in both directions.
    pub bidirectional: bool,
    /// Frontmatter field name (if source is frontmatter).
    pub field: Option<String>,
    /// Additional config serialized as JSON.
    pub config: Option<String>,
}

/// Confidence band for a resolved cross-corpus symbol link.
///
/// Resolution across independent corpora is inherently ambiguous, so each
/// cross-corpus edge carries a provenance band describing how it was resolved:
///
/// - [`ResolutionConfidence::High`] — exactly one exact `scope_path` match across
///   all corpora (unambiguous).
/// - [`ResolutionConfidence::Medium`] — unique only after a tie-break heuristic
///   (e.g. matching language).
/// - [`ResolutionConfidence::Speculative`] — a weaker, best-effort match.
///
/// This phase only ever emits `High` edges; `Medium`/`Speculative` exist for the
/// confidence-band API and are populated by later resolution phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolutionConfidence {
    /// Exactly one exact `scope_path` match across all corpora.
    High,
    /// Unique only after a tie-break heuristic (e.g. same language).
    Medium,
    /// A weaker, best-effort match.
    Speculative,
}

/// A typed, weighted, directed edge in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// Source document or code entity path.
    pub source: String,
    /// Target document or code entity path.
    pub target: String,
    /// Edge type name (must match a registered `EdgeTypeConfig.name`).
    pub edge_type: String,
    /// Weight of this edge.
    pub weight: f32,
    /// How this edge was created.
    pub provenance: EdgeProvenance,
    /// Name of the corpus the target lives in, for cross-corpus links.
    ///
    /// `None` for intra-corpus edges; `Some(corpus)` when the target symbol was
    /// resolved in a different corpus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_corpus: Option<String>,
    /// Confidence band for a resolved cross-corpus link.
    ///
    /// `None` for intra-corpus edges; `Some(_)` for cross-corpus links.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ResolutionConfidence>,
    /// Path of the resolved target within its (possibly remote) corpus.
    ///
    /// `None` for intra-corpus edges. For cross-corpus edges this is the
    /// remote-endpoint payload that lets a federated query report a hop and
    /// continue traversal into the target corpus without re-resolving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    /// Qualified symbol name of the resolved cross-corpus target (`None` for
    /// intra-corpus edges).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_symbol: Option<String>,
    /// Kind of the resolved cross-corpus target (e.g. code symbol type or
    /// document kind); `None` for intra-corpus edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<String>,
}

impl Edge {
    /// Create a standard intra-corpus edge.
    pub fn new(
        source: impl Into<String>,
        target: impl Into<String>,
        edge_type: impl Into<String>,
        weight: f32,
        provenance: EdgeProvenance,
    ) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            edge_type: edge_type.into(),
            weight,
            provenance,
            target_corpus: None,
            confidence: None,
            target_path: None,
            target_symbol: None,
            target_kind: None,
        }
    }
}

/// How an edge came into existence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeProvenance {
    /// Parsed from explicit wikilink in content.
    Wikilink,
    /// Derived from shared tag.
    SharedTag,
    /// Declared in frontmatter field.
    Frontmatter,
    /// Parsed from standard markdown link.
    MarkdownLink,
    /// AST symbol definition (file defines symbol).
    CodeDefines,
    /// Import / use dependency across files.
    CodeImports,
    /// Function/method call site invocation.
    CodeCalls,
    /// Trait or interface implementation.
    CodeImplementsTrait,
    /// Language decorator or annotation (e.g. @decorator, #[derive]).
    CodeDecorates,
    /// Class inheritance / extension (e.g. class A extends B).
    CodeExtends,
    /// Macro expansion invocation (e.g. println!, vec![]).
    CodeMacroExpands,
    /// Embedded struct field composition (e.g. Go anonymous struct fields).
    CodeStructEmbeds,
    /// Relational foreign key constraint (e.g. SQL REFERENCES).
    CodeForeignKey,
    /// Markdown documentation specifies or documents code symbol.
    DocumentsCode,
    /// Code entity implements an architecture decision record (ADR).
    ImplementsAdr,
    /// Inferred by LLM extraction (future: tiered ontological model).
    Inferred,
}

/// Durable relational edge record for SQLite persistence (`meta.db`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeRecord {
    /// Database row ID (None for unsaved records).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// Source document or code entity path.
    pub source: String,
    /// Target document or code entity path.
    pub target: String,
    /// Edge relationship type (e.g. "calls", "defines", "imports", "implements", "wikilink", "documents").
    pub edge_type: String,
    /// Edge classification class ("structural", "semantic", "hybrid").
    pub edge_class: String,
    /// Edge weight (default 1.0).
    pub weight: f32,
    /// Resolution confidence (0.0 - 1.0, default 1.0).
    pub confidence: f32,
    /// Optional metadata payload (e.g. line numbers, call AST context).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
}
