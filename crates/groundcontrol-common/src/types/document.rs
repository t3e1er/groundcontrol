//! Document domain types.

use serde::{Deserialize, Serialize};

/// A unique identifier for a document (note) within a corpus.
pub type DocId = String;

/// A parsed markdown document with extracted metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Relative path within the corpus.
    pub path: String,
    /// Parsed YAML frontmatter (if present).
    pub frontmatter: Option<serde_json::Value>,
    /// Document title (from frontmatter or first heading).
    pub title: Option<String>,
    /// Extracted tags (from frontmatter and inline #tags).
    pub tags: Vec<String>,
    /// Wikilinks found in the content.
    pub wikilinks: Vec<WikiLink>,
    /// The template this note declares (from frontmatter `template:` field).
    pub template: Option<String>,
    /// Raw markdown content (without frontmatter block).
    pub content: String,
    /// Content hash for change detection.
    pub content_hash: String,
}

/// A wikilink reference found in a document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WikiLink {
    /// The target path or name (what's inside the `[[...]]`).
    pub target: String,
    /// Optional display alias (from `[[target|alias]]`).
    pub alias: Option<String>,
}

/// Format/nature of a tracked file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FileFormat {
    /// Native plain-text source file (Markdown notes or polyglot code files).
    #[default]
    Source,
    /// Microsoft Word OpenXML document (.docx).
    Docx,
    /// Portable Document Format (.pdf).
    Pdf,
    /// HTML rich documentation article (.html / .htm).
    HtmlDoc,
}

impl FileFormat {
    /// Returns the database string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Docx => "docx",
            Self::Pdf => "pdf",
            Self::HtmlDoc => "html_doc",
        }
    }

    /// Parses from database string representation.
    pub fn from_str_name(s: &str) -> Self {
        match s {
            "docx" => Self::Docx,
            "pdf" => Self::Pdf,
            "html_doc" => Self::HtmlDoc,
            _ => Self::Source,
        }
    }

    /// Whether this format produces a Derived Text Projection.
    pub fn is_projected(&self) -> bool {
        !matches!(self, Self::Source)
    }
}

impl std::fmt::Display for FileFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A tracked file in the index.
#[derive(Debug, Clone)]
pub struct FileRecord {
    /// Relative path within the corpus.
    pub path: String,
    /// BLAKE3 content hash.
    pub content_hash: String,
    /// File modification time as Unix timestamp (seconds).
    pub modified_at: i64,
    /// Template declared in frontmatter.
    pub template: Option<String>,
    /// Document title.
    pub title: Option<String>,
    /// When this file was last indexed (Unix timestamp seconds).
    pub indexed_at: i64,
    /// Format of the file (native source or projected document).
    pub format: FileFormat,
}
