//! Microsoft Word (.docx) OpenXML document extractor.
//!
//! Extracts structured Markdown text, headings, tables, metadata, and outbound
//! hyperlinks from Word (.docx) files using pure-Rust `zip` and `quick-xml`.

pub mod body;
pub mod properties;
pub mod relationships;

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use zip::ZipArchive;

use groundcontrol_common::ports::{DocumentExtractor, ExtractedDocument};
use groundcontrol_common::{Error, Result};

use self::body::parse_document_xml;
use self::properties::parse_core_properties;
use self::relationships::parse_relationships;

/// Extractor for Microsoft Word OpenXML documents (.docx).
#[derive(Debug, Default, Clone)]
pub struct DocxExtractor;

impl DocxExtractor {
    /// Create a new Word document extractor.
    pub fn new() -> Self {
        Self
    }
}

impl DocumentExtractor for DocxExtractor {
    fn can_extract(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("docx"))
            .unwrap_or(false)
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> Result<ExtractedDocument> {
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor).map_err(|e| Error::Parse {
            path: path.to_string_lossy().to_string(),
            message: format!("failed to open docx zip archive: {e}"),
        })?;

        let mut metadata = HashMap::new();
        let mut title = None;

        // 1. Extract metadata from docProps/core.xml if present
        if let Ok(mut core_file) = archive.by_name("docProps/core.xml") {
            let mut xml = String::new();
            if core_file.read_to_string(&mut xml).is_ok() {
                parse_core_properties(&xml, &mut metadata, &mut title);
            }
        }

        // 2. Extract relationship targets from word/_rels/document.xml.rels
        let mut rels = HashMap::new();
        if let Ok(mut rels_file) = archive.by_name("word/_rels/document.xml.rels") {
            let mut xml = String::new();
            if rels_file.read_to_string(&mut xml).is_ok() {
                parse_relationships(&xml, &mut rels);
            }
        }

        // 3. Extract main content from word/document.xml
        let mut doc_file = archive.by_name("word/document.xml").map_err(|e| Error::Parse {
            path: path.to_string_lossy().to_string(),
            message: format!("docx missing word/document.xml: {e}"),
        })?;
        let mut document_xml = String::new();
        doc_file.read_to_string(&mut document_xml).map_err(|e| {
            Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        })?;

        let (normalized_text, mut outbound_links, first_heading) =
            parse_document_xml(&document_xml, &rels);

        if title.is_none() {
            title = first_heading;
        }

        // Deduplicate outbound links
        let mut seen = std::collections::HashSet::new();
        outbound_links.retain(|l| seen.insert(l.target.clone()));

        Ok(ExtractedDocument { title, metadata, normalized_text, outbound_links })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn test_extract_minimal_docx() {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));

            // Write docProps/core.xml
            zip.start_file("docProps/core.xml", SimpleFileOptions::default()).unwrap();
            zip.write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
            <cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">
                <dc:title>System Architecture Spec</dc:title>
                <dc:creator>Alice Engineer</dc:creator>
            </cp:coreProperties>
            "#,
            )
            .unwrap();

            // Write word/_rels/document.xml.rels
            zip.start_file("word/_rels/document.xml.rels", SimpleFileOptions::default()).unwrap();
            zip.write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
            <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="docs/overview.md"/>
            </Relationships>
            "#,
            )
            .unwrap();

            // Write word/document.xml
            zip.start_file("word/document.xml", SimpleFileOptions::default()).unwrap();
            zip.write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
                <w:body>
                    <w:p>
                        <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
                        <w:r><w:t>Introduction</w:t></w:r>
                    </w:p>
                    <w:p>
                        <w:r><w:t>This document specifies the storage layer.</w:t></w:r>
                    </w:p>
                    <w:p>
                        <w:hyperlink r:id="rId1">
                            <w:r><w:t>Overview Link</w:t></w:r>
                        </w:hyperlink>
                    </w:p>
                    <w:tbl>
                        <w:tr>
                            <w:tc><w:p><w:r><w:t>Module</w:t></w:r></w:p></w:tc>
                            <w:tc><w:p><w:r><w:t>Status</w:t></w:r></w:p></w:tc>
                        </w:tr>
                        <w:tr>
                            <w:tc><w:p><w:r><w:t>Storage</w:t></w:r></w:p></w:tc>
                            <w:tc><w:p><w:r><w:t>Active</w:t></w:r></w:p></w:tc>
                        </w:tr>
                    </w:tbl>
                </w:body>
            </w:document>
            "#,
            )
            .unwrap();

            zip.finish().unwrap();
        }

        let extractor = DocxExtractor::new();
        let doc = extractor.extract(Path::new("spec.docx"), &buf).unwrap();

        assert_eq!(doc.title.as_deref(), Some("System Architecture Spec"));
        assert_eq!(doc.metadata.get("author").map(|s| s.as_str()), Some("Alice Engineer"));

        // Content
        assert!(doc.normalized_text.contains("# Introduction"));
        assert!(doc.normalized_text.contains("This document specifies the storage layer."));
        assert!(doc.normalized_text.contains("[Overview Link](docs/overview.md)"));
        assert!(doc.normalized_text.contains("| Module | Status |"));
        assert!(doc.normalized_text.contains("| Storage | Active |"));

        // Outbound links
        assert_eq!(doc.outbound_links.len(), 1);
        assert_eq!(doc.outbound_links[0].target, "docs/overview.md");
    }
}
