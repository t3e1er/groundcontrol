//! Document extractor port for rich container documents (.docx, .pdf, .html).

use std::collections::HashMap;
use std::path::Path;

use crate::types::DocLink;
use crate::Result;

/// An outbound cross-reference link extracted from a rich document (canonical `DocLink`).
pub type DocumentLink = DocLink;

/// Structured document extracted from a rich document container (.docx, .pdf, .html).
#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    /// Document title extracted from metadata or primary heading.
    pub title: Option<String>,
    /// Extracted document metadata properties (author, date, subject, etc.).
    pub metadata: HashMap<String, String>,
    /// Normalized UTF-8 text with Markdown formatting and synthetic lines.
    pub normalized_text: String,
    /// Outbound cross-reference links (hyperlinks, anchors, references).
    pub outbound_links: Vec<DocumentLink>,
}

/// Port for deterministic document extraction.
///
/// Implementations of this port extract structured, normalized UTF-8 text and
/// outbound links from binary or container document formats (.docx, .pdf, .html)
/// in 100% pure Rust without external C runtimes.
pub trait DocumentExtractor: Send + Sync {
    /// Returns true if this extractor handles the given file format.
    fn can_extract(&self, path: &Path) -> bool;

    /// Extract structured text and links from raw file bytes.
    fn extract(&self, path: &Path, bytes: &[u8]) -> Result<ExtractedDocument>;
}
