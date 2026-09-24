//! Code domain types.

use serde::{Deserialize, Serialize};

use super::edge::ResolutionConfidence;

/// Discriminates between documentation notes and polyglot source code entities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// Markdown documentation note, RFC, or ADR.
    Documentation,
    /// Whole source code file (e.g. `src/engine.rs`).
    CodeFile {
        /// Programming language of the source file.
        language: String,
    },
    /// Distinct code symbol (function, struct, class, trait, interface, etc.).
    CodeSymbol {
        /// Programming language of the symbol.
        language: String,
        /// Classification of the symbol.
        symbol_type: CodeSymbolType,
        /// Hierarchical scope path (e.g. `crate::search::Engine`).
        scope_path: String,
        /// Full signature or declaration line.
        signature: String,
    },
    /// Syntactically coherent AST chunk for vector and BM25 indexing.
    CodeChunk {
        /// Programming language of the chunk.
        language: String,
        /// Hierarchical scope path breadcrumb.
        scope_path: String,
        /// 1-based start line number in original file.
        start_line: usize,
        /// 1-based end line number in original file.
        end_line: usize,
    },
}

impl Default for EntityKind {
    fn default() -> Self {
        Self::Documentation
    }
}

impl EntityKind {
    /// Whether this entity is source code (anything other than [`EntityKind::Documentation`]).
    pub fn is_code(&self) -> bool {
        !matches!(self, EntityKind::Documentation)
    }

    /// Coarse modality tag for indexing/filtering: `"code"` for any code entity,
    /// `"docs"` for documentation.
    pub fn modality_tag(&self) -> &'static str {
        if self.is_code() {
            "code"
        } else {
            "docs"
        }
    }
}

/// Restricts search results to documentation, code, or both.
///
/// Applied consistently across BM25, vector, graph, and the fused hybrid path.
/// [`Modality::Both`] (the default) returns every entity kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    /// Only documentation entities ([`EntityKind::Documentation`]).
    Docs,
    /// Only code entities (`CodeFile`, `CodeSymbol`, `CodeChunk`).
    Code,
    /// Both documentation and code (no restriction).
    Both,
}

impl Default for Modality {
    fn default() -> Self {
        Modality::Both
    }
}

impl Modality {
    /// Parse from a string (case-insensitive): `"docs"`, `"code"`, or `"both"`.
    pub fn from_str_name(s: &str) -> Option<Modality> {
        match s.to_lowercase().as_str() {
            "docs" => Some(Modality::Docs),
            "code" => Some(Modality::Code),
            "both" => Some(Modality::Both),
            _ => None,
        }
    }

    /// Whether an [`EntityKind`] passes this modality filter.
    ///
    /// [`Modality::Both`] matches all kinds; [`Modality::Docs`] matches only
    /// [`EntityKind::Documentation`]; [`Modality::Code`] matches every code kind.
    pub fn matches_kind(self, kind: &EntityKind) -> bool {
        match self {
            Modality::Both => true,
            Modality::Docs => !kind.is_code(),
            Modality::Code => kind.is_code(),
        }
    }

    /// Whether a coarse modality tag (`"code"` / `"docs"`) passes this filter.
    pub fn matches_tag(self, tag: &str) -> bool {
        match self {
            Modality::Both => true,
            Modality::Docs => tag == "docs",
            Modality::Code => tag == "code",
        }
    }
}

/// The specific classification of a code symbol.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CodeSymbolType {
    /// Standalone function.
    Function,
    /// Method associated with a struct, class, or trait.
    Method,
    /// Struct data structure.
    Struct,
    /// Object-oriented class.
    Class,
    /// Rust trait definition.
    Trait,
    /// Interface definition.
    Interface,
    /// Enum type definition.
    Enum,
    /// Module or namespace declaration.
    Module,
    /// Constant or static value.
    Constant,
    /// Type alias definition.
    TypeAlias,
}

/// A structured code symbol record extracted via AST analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeSymbol {
    /// File path where the symbol is defined.
    pub file_path: String,
    /// Identifier name (e.g. "search_hybrid").
    pub name: String,
    /// Fully qualified hierarchical scope (e.g. "groundcontrol_core::search::Engine").
    pub scope_path: String,
    /// Symbol classification.
    pub symbol_type: CodeSymbolType,
    /// Source code language (e.g. "rust", "typescript", "python", "go").
    pub language: String,
    /// Signature or declaration snippet (e.g. `pub fn search_hybrid(&self, ...) -> Result<Vec<SearchResult>>`).
    pub signature: String,
    /// Docstring or preceding documentation comments if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// 1-based start line.
    pub start_line: usize,
    /// 1-based end line.
    pub end_line: usize,
}

/// Alias for code symbol and document node identifiers.
pub type SymbolId = String;

/// The kind of unresolved reference captured as an [`ExternalRef`].
///
/// Both variants capture ordinary intra-language references that failed to
/// resolve locally and are handed to the cross-corpus reconciliation pass for
/// qualified-name symbol matching.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExternalRefKind {
    /// A call site whose callee did not resolve to any in-corpus symbol.
    Call,
    /// An import/use whose target did not resolve to any in-corpus symbol.
    Import,
}

/// A call or import target that failed to resolve against the full corpus
/// symbol index.
///
/// Single-repo indexing intentionally still emits a low-confidence intra-repo
/// edge for these (see the graph code extractor); an `ExternalRef` is captured
/// *in addition*, as a durable record of the unresolved target so a later
/// cross-corpus reconciliation pass can attempt to resolve it against other
/// corpora. Capturing these does not alter intra-repo edge output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalRef {
    /// Scope path of the caller/importer (the edge source) that referenced the
    /// unresolved target.
    pub caller_scope_path: String,
    /// The raw, unresolved target string as it appeared in the source (e.g. a
    /// bare callee name, `receiver.method`, or an import path).
    pub raw_target: String,
    /// Whether the unresolved reference is a call or an import.
    pub kind: ExternalRefKind,
    /// Resolution confidence band for the reference (always
    /// [`ResolutionConfidence::Speculative`] at capture time).
    pub confidence: ResolutionConfidence,
}

/// Extracted structural semantic signals from AST grammar.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtractedGrammarSemantics {
    /// Channel 0: Interface and declaration tokens (symbol name, parameter names, types).
    pub interface_tokens: Vec<WeightedToken>,
    /// Channel 1: Outbound API calls and invocations.
    pub api_tokens: Vec<WeightedToken>,
    /// Channel 2: Def-Use data flow paths (parameter flow to arguments, returns, and conditions).
    pub dataflow_paths: Vec<DataFlowPath>,
    /// Channel 3: Structural AST grammar transition bigrams and complexity profile.
    pub grammar_transitions: Vec<GrammarTransition>,
}

/// A weighted semantic token with structural tree-depth attenuation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightedToken {
    /// The token text.
    pub text: String,
    /// Structural depth weight: `1.0 / sqrt(1.0 + depth)`.
    pub weight: f32,
}

/// A data-flow def-use path linking a declared parameter to an internal sink.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DataFlowPath {
    /// Parameter identifier name.
    pub source_param: String,
    /// Destination sink kind.
    pub sink: DataFlowSink,
}

/// Destination sink in intra-symbol def-use chains.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataFlowSink {
    /// Parameter passed as an argument into an outbound callee.
    Call(String),
    /// Parameter returned from the function.
    Return,
    /// Parameter evaluated within a conditional branch or loop condition.
    Condition,
}

/// A structural parent-child grammar rule transition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GrammarTransition {
    /// Tree-sitter node kind of the parent.
    pub parent_kind: String,
    /// Tree-sitter node kind of the child.
    pub child_kind: String,
    /// Relative tree depth.
    pub depth: u16,
}
