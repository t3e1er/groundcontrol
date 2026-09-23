//! Unified intermediate parsed artifact representation.

use serde::{Deserialize, Serialize};

use super::chunk::Chunk;
use super::code::{CodeSymbol, ExternalRef, ExtractedGrammarSemantics};
use super::document::{Document, FileFormat};
use super::edge::Edge;

/// A parsed file artifact produced once per file by Tree-sitter / Markdown parsing.
///
/// Broadcast across all registered [`crate::ports::RetrievalAlgorithm`] implementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedArtifact {
    /// Relative path within the corpus.
    pub path: String,
    /// Blake3 content hash.
    pub hash: String,
    /// Whether the file is a source code file.
    pub is_code: bool,
    /// Format of the file (native source or projected document).
    pub format: FileFormat,
    /// File title or document title if available.
    pub title: Option<String>,
    /// Parsed markdown document metadata (if markdown).
    pub doc_metadata: Option<Document>,
    /// Extracted code symbols (empty for markdown notes).
    pub symbols: Vec<CodeSymbol>,
    /// Extracted grammar semantics matching symbols (empty for markdown notes).
    pub grammar_semantics: Vec<ExtractedGrammarSemantics>,
    /// Syntactic and semantic chunks.
    pub chunks: Vec<Chunk>,
    /// Structural code or markdown edges.
    pub graph_edges: Vec<Edge>,
    /// Unresolved call/import targets captured for cross-corpus resolution.
    pub external_refs: Vec<ExternalRef>,
    /// Raw text content if available.
    pub raw_content: Option<String>,
    /// Optional synthesized projection text for non-text / binary files.
    pub projection_text: Option<String>,
}
