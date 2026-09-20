//! Chunk domain types.

use serde::{Deserialize, Serialize};

use super::code::EntityKind;

/// A text chunk coordinate record stored for a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRecord {
    /// Zero-based index within the parent document.
    pub chunk_index: usize,
    /// Byte offset of chunk start in original content.
    pub start_byte: usize,
    /// Byte offset of chunk end in original content.
    pub end_byte: usize,
    /// 1-based line number of chunk start.
    pub start_line: usize,
    /// 1-based line number of chunk end.
    pub end_line: usize,
}

/// Policy for whether a chunk should receive a dense vector embedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChunkEmbedPolicy {
    /// Always embed: documentation, ADRs, exported interfaces, public API surfaces.
    Anchor,
    /// Never embed: internal helpers, private methods, leaf implementations.
    /// Searchable via BM25 lexical index and navigable via AST graph.
    GraphOnly,
}

impl Default for ChunkEmbedPolicy {
    fn default() -> Self {
        Self::Anchor
    }
}

fn default_line() -> usize {
    1
}

/// A text chunk ready for embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// The document or code file this chunk belongs to.
    pub doc_path: String,
    /// Zero-based index of this chunk within the document.
    pub chunk_index: usize,
    /// The text content of this chunk.
    pub text: String,
    /// Byte offset of chunk start in original content.
    pub start_byte: usize,
    /// Byte offset of chunk end in original content.
    pub end_byte: usize,
    /// 1-based line number of chunk start.
    #[serde(default = "default_line")]
    pub start_line: usize,
    /// 1-based line number of chunk end.
    #[serde(default = "default_line")]
    pub end_line: usize,
    /// Heading hierarchy for this chunk (e.g., "Setup > Prerequisites").
    /// Populated by the heading-aware chunker; None for other strategies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_chain: Option<String>,
    /// Source code language if this is an AST code chunk (e.g. "rust", "typescript").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Hierarchical AST scope breadcrumb for code chunks (e.g. "crate::search::Engine > search_hybrid").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_path: Option<String>,
    /// Entity kind for this chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_kind: Option<EntityKind>,
    /// Embedding policy for this chunk (anchor vs graph-only).
    #[serde(default)]
    pub embed_policy: ChunkEmbedPolicy,
    /// Compact skeleton text (e.g. signature + docstring + scope) for embedding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_text: Option<String>,
}

impl Chunk {
    /// Create a standard document chunk with default metadata.
    pub fn new(
        doc_path: impl Into<String>,
        chunk_index: usize,
        text: impl Into<String>,
        start_byte: usize,
        end_byte: usize,
    ) -> Self {
        Self {
            doc_path: doc_path.into(),
            chunk_index,
            text: text.into(),
            start_byte,
            end_byte,
            start_line: 1,
            end_line: 1,
            heading_chain: None,
            language: None,
            scope_path: None,
            entity_kind: Some(EntityKind::Documentation),
            embed_policy: ChunkEmbedPolicy::Anchor,
            skeleton_text: None,
        }
    }

    /// Set line span.
    pub fn with_lines(mut self, start_line: usize, end_line: usize) -> Self {
        self.start_line = start_line;
        self.end_line = end_line;
        self
    }

    /// Set heading chain.
    pub fn with_heading_chain(mut self, heading_chain: Option<String>) -> Self {
        self.heading_chain = heading_chain;
        self
    }

    /// Set code AST metadata.
    pub fn with_code_metadata(
        mut self,
        language: impl Into<String>,
        scope_path: impl Into<String>,
        start_line: usize,
        end_line: usize,
    ) -> Self {
        let lang = language.into();
        let scope = scope_path.into();
        self.language = Some(lang.clone());
        self.scope_path = Some(scope.clone());
        self.start_line = start_line;
        self.end_line = end_line;
        self.entity_kind =
            Some(EntityKind::CodeChunk { language: lang, scope_path: scope, start_line, end_line });
        self
    }

    /// Set embedding policy.
    pub fn with_embed_policy(mut self, embed_policy: ChunkEmbedPolicy) -> Self {
        self.embed_policy = embed_policy;
        self
    }

    /// Set skeleton text for compact embedding.
    pub fn with_skeleton_text(mut self, skeleton_text: impl Into<String>) -> Self {
        self.skeleton_text = Some(skeleton_text.into());
        self
    }
}
