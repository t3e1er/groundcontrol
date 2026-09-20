//! Deterministic corpus modality classifier.
//!
//! Classifies files into documentation notes, polyglot code, rich documents
//! (.html, .pdf, .docx), or ignored files:
//! 1. Native markdown notes (.md, .markdown) are always documentation.
//! 2. Rich non-markdown documents (.html, .pdf, .docx) matching `docs.patterns`
//!    are promoted to Derived Text Projections.
//! 3. Polyglot source code files are parsed via tree-sitter AST chunking.

use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use ctxvault_common::config::CorpusConfig;
use ctxvault_common::types::FileFormat;

use crate::parser::code::{detect_language, is_code_file, SupportedLanguage};

/// Categorization of a file discovered in a corpus root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileClassification {
    /// Native Markdown documentation note (.md).
    MarkdownDoc,
    /// Polyglot source code file.
    Code(SupportedLanguage),
    /// Rich document file that will produce a Derived Text Projection.
    Document(FileFormat),
    /// Ignored file (binary asset, test fixture, or unsupported format).
    Ignored,
}

impl FileClassification {
    /// Returns true if this file is indexable (doc, code, or rich document).
    pub fn is_indexable(&self) -> bool {
        !matches!(self, Self::Ignored)
    }

    /// Returns the corresponding `FileFormat`.
    pub fn file_format(&self) -> FileFormat {
        match self {
            Self::MarkdownDoc | Self::Code(_) => FileFormat::Source,
            Self::Document(fmt) => *fmt,
            Self::Ignored => FileFormat::Source,
        }
    }

    /// Returns whether this file is treated as documentation for retrieval modality.
    pub fn is_documentation(&self) -> bool {
        matches!(self, Self::MarkdownDoc | Self::Document(_))
    }
}

/// A compiled classifier for determining file modalities.
#[derive(Clone, Debug)]
pub struct FileClassifier {
    doc_matcher: Option<Gitignore>,
}

impl FileClassifier {
    /// Build a new classifier from the corpus configuration.
    pub fn new(root: &Path, config: &CorpusConfig) -> Self {
        let doc_matcher = if !config.docs.patterns.is_empty() {
            let mut builder = GitignoreBuilder::new(root);
            for p in &config.docs.patterns {
                let _ = builder.add_line(None, p);
            }
            builder.build().ok()
        } else {
            None
        };

        Self { doc_matcher }
    }

    /// Classify a file by path, optionally reading sample bytes.
    pub fn classify(&self, path: &Path, _sample_bytes: Option<&[u8]>) -> FileClassification {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();

        // 1. Native Markdown notes are always documentation.
        if ext == "md" || ext == "markdown" {
            return FileClassification::MarkdownDoc;
        }

        let is_html = ext == "html" || ext == "htm";
        let is_pdf = ext == "pdf";
        let is_docx = ext == "docx";

        // 2. Explicit doc patterns promotion for rich documents
        if is_html || is_pdf || is_docx {
            if let Some(ref matcher) = self.doc_matcher {
                if matcher.matched_path_or_any_parents(path, false).is_ignore() {
                    if is_html {
                        return FileClassification::Document(FileFormat::HtmlDoc);
                    } else if is_pdf {
                        return FileClassification::Document(FileFormat::Pdf);
                    } else if is_docx {
                        return FileClassification::Document(FileFormat::Docx);
                    }
                }
            }
        }

        // 3. Polyglot source code files (including HTML web components not promoted to docs)
        if is_code_file(path) {
            if let Some(lang) = detect_language(path) {
                return FileClassification::Code(lang);
            }
        }

        FileClassification::Ignored
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxvault_common::config::DocsConfig;
    use tempfile::TempDir;

    #[test]
    fn test_classifier_defaults() {
        let tmp = TempDir::new().unwrap();
        let config = CorpusConfig {
            name: "test".to_string(),
            path: tmp.path().to_string_lossy().to_string(),
            docs: DocsConfig { patterns: vec!["docs/**".to_string(), "wiki/**".to_string()] },
            ..Default::default()
        };

        let classifier = FileClassifier::new(tmp.path(), &config);

        // Markdown
        assert_eq!(
            classifier.classify(Path::new("README.md"), None),
            FileClassification::MarkdownDoc
        );
        assert_eq!(
            classifier.classify(Path::new("docs/intro.md"), None),
            FileClassification::MarkdownDoc
        );

        // Polyglot code
        assert_eq!(
            classifier.classify(Path::new("src/main.rs"), None),
            FileClassification::Code(SupportedLanguage::Rust)
        );
        assert_eq!(
            classifier.classify(Path::new("src/index.html"), None),
            FileClassification::Code(SupportedLanguage::Html)
        );

        // Rich documents promoted by docs.patterns
        assert_eq!(
            classifier.classify(Path::new("docs/api.html"), None),
            FileClassification::Document(FileFormat::HtmlDoc)
        );
        assert_eq!(
            classifier.classify(Path::new("docs/manual.pdf"), None),
            FileClassification::Document(FileFormat::Pdf)
        );
        assert_eq!(
            classifier.classify(Path::new("wiki/spec.docx"), None),
            FileClassification::Document(FileFormat::Docx)
        );

        // Rich documents outside docs.patterns
        assert_eq!(
            classifier.classify(Path::new("fixtures/sample.pdf"), None),
            FileClassification::Ignored
        );
    }
}
