//! Unified artifact parsing, polyglot code AST chunking, and document extraction.

pub mod artifact;
pub mod code;
pub mod document;

pub use artifact::ArtifactParser;
pub use document::markdown::{chunk_markdown_document, parse_document};
pub use document::{chunker, policy};
