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

use groundcontrol_common::config::CorpusConfig;
use groundcontrol_common::types::FileFormat;

use crate::parser::code::{detect_language_with_content, is_code_file, SupportedLanguage};

/// Categorization of a file discovered in a corpus root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileClassification {
    /// Native Markdown documentation note (.md).
    MarkdownDoc,
    /// Polyglot source code file.
    Code(SupportedLanguage),
    /// Rich document file that will produce a Derived Text Projection.
    Document(FileFormat),
    /// Plain-text code or configuration in an unsupported language (BM25 fallback).
    GenericText,
    /// Ignored file (binary asset, test fixture, or unsupported format).
    Ignored,
}

impl FileClassification {
    /// Returns true if this file is indexable (doc, code, rich document, or generic text).
    pub fn is_indexable(&self) -> bool {
        !matches!(self, Self::Ignored)
    }

    /// Returns the corresponding `FileFormat`.
    pub fn file_format(&self) -> FileFormat {
        match self {
            Self::MarkdownDoc | Self::Code(_) | Self::GenericText => FileFormat::Source,
            Self::Document(fmt) => *fmt,
            Self::Ignored => FileFormat::Source,
        }
    }

    /// Returns whether this file is treated as documentation for retrieval modality.
    pub fn is_documentation(&self) -> bool {
        matches!(self, Self::MarkdownDoc | Self::Document(_))
    }
}

/// Known binary file extensions that should never be indexed as text.
pub const KNOWN_BINARY_EXTENSIONS: &[&str] = &[
    "exe", "dll", "so", "dylib", "bin", "o", "a", "lib", "class", "jar", "pyc", "pyo", "pyd",
    "png", "jpg", "jpeg", "gif", "ico", "webp", "bmp", "tiff", "psd", "mp3", "wav", "flac", "ogg",
    "mp4", "avi", "mkv", "mov", "webm", "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "iso",
    "wasm", "woff", "woff2", "ttf", "eot", "otf", "db", "sqlite", "sqlite3", "pdf", "docx",
];

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
    pub fn classify(&self, path: &Path, sample_bytes: Option<&[u8]>) -> FileClassification {
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
            if let Some(lang) = detect_language_with_content(path, sample_bytes) {
                return FileClassification::Code(lang);
            }
        }

        // 4. Universal Fallback for unsupported text files
        // Ignore dotfiles/hidden files (e.g. .gitignore, .env, .gitattributes)
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if file_name.starts_with('.') {
                return FileClassification::Ignored;
            }
        }

        // Must have an extension to represent an unsupported code/markup language
        if ext.is_empty() || KNOWN_BINARY_EXTENSIONS.contains(&ext.as_str()) {
            return FileClassification::Ignored;
        }

        if let Some(bytes) = sample_bytes {
            let check_len = bytes.len().min(1024);
            if bytes[..check_len].contains(&0) {
                return FileClassification::Ignored;
            }
        }

        FileClassification::GenericText
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use groundcontrol_common::config::DocsConfig;
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

        // New polyglot languages
        assert_eq!(
            classifier.classify(Path::new("finance/account.cbl"), None),
            FileClassification::Code(SupportedLanguage::Cobol)
        );
        assert_eq!(
            classifier.classify(Path::new("legacy/form1.frm"), None),
            FileClassification::Code(SupportedLanguage::Vb6)
        );
        assert_eq!(
            classifier.classify(Path::new("db/pkg_body.pkb"), None),
            FileClassification::Code(SupportedLanguage::PlSql)
        );
        assert_eq!(
            classifier.classify(Path::new("maven/pom.xml"), None),
            FileClassification::Code(SupportedLanguage::Xml)
        );
        assert_eq!(
            classifier.classify(Path::new("transform/sheet.xslt"), None),
            FileClassification::Code(SupportedLanguage::Xml)
        );

        // Universal Plain-Text BM25 Fallback
        assert_eq!(
            classifier.classify(Path::new("pascal/main.pas"), None),
            FileClassification::GenericText
        );
        assert_eq!(
            classifier.classify(Path::new("config/settings.ini"), Some(b"[settings]\nport=8080\n")),
            FileClassification::GenericText
        );

        // Dialect disambiguation
        assert_eq!(
            classifier.classify(Path::new("db/query.sql"), Some(b"SELECT 1 FROM dual;")),
            FileClassification::Code(SupportedLanguage::Sql)
        );
        assert_eq!(
            classifier.classify(
                Path::new("db/package.sql"),
                Some(b"CREATE OR REPLACE PACKAGE BODY my_pkg IS ...")
            ),
            FileClassification::Code(SupportedLanguage::PlSql)
        );

        // Dotfiles and extensionless files ignored by fallback
        assert_eq!(classifier.classify(Path::new(".gitignore"), None), FileClassification::Ignored);
        assert_eq!(classifier.classify(Path::new(".env"), None), FileClassification::Ignored);
        assert_eq!(classifier.classify(Path::new("LICENSE"), None), FileClassification::Ignored);

        // Binary files correctly ignored
        assert_eq!(
            classifier.classify(Path::new("bin/app.exe"), None),
            FileClassification::Ignored
        );
        assert_eq!(
            classifier.classify(Path::new("data/corrupt.dat"), Some(&[0x48, 0x65, 0x00, 0x6c])),
            FileClassification::Ignored
        );
    }
}
