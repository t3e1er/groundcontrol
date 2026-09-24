//! PDF document extractor.
//!
//! Extracts text pages, `<!-- Page N -->` line anchors, metadata, and outbound URI
//! annotations from vector/text PDF documents using pure-Rust `lopdf`.
//! Strictly scoped to text/vector PDFs; scanned PDFs emit a warning.

pub mod info;
pub mod links;
pub mod pages;

use std::path::Path;

use lopdf::Document;

use groundcontrol_common::ports::{DocumentExtractor, ExtractedDocument};
use groundcontrol_common::{Error, Result};

use self::info::extract_info_metadata;
use self::pages::extract_pages_and_headings;

/// Extractor for Portable Document Format files (.pdf).
#[derive(Debug, Default, Clone)]
pub struct PdfExtractor;

impl PdfExtractor {
    /// Create a new PDF document extractor.
    pub fn new() -> Self {
        Self
    }
}

impl DocumentExtractor for PdfExtractor {
    fn can_extract(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
    }

    fn extract(&self, path: &Path, bytes: &[u8]) -> Result<ExtractedDocument> {
        let doc = Document::load_mem(bytes).map_err(|e| Error::Parse {
            path: path.to_string_lossy().to_string(),
            message: format!("failed to parse PDF document: {e}"),
        })?;

        // 1. Extract metadata from /Info dictionary if present
        let (mut title, metadata) = extract_info_metadata(&doc);

        // 2. Extract pages and annotations
        let mut lines = Vec::new();
        let mut outbound_links = Vec::new();
        let mut first_heading = None;

        extract_pages_and_headings(&doc, &mut lines, &mut outbound_links, &mut first_heading);

        if title.is_none() {
            title = first_heading;
        }

        // Deduplicate outbound links
        let mut seen = std::collections::HashSet::new();
        outbound_links.retain(|l| seen.insert(l.target.clone()));

        // Collapse empty lines
        let mut cleaned_lines = Vec::new();
        let mut prev_empty = false;
        for line in lines {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if !prev_empty {
                    cleaned_lines.push(String::new());
                    prev_empty = true;
                }
            } else {
                cleaned_lines.push(line);
                prev_empty = false;
            }
        }

        let normalized_text = cleaned_lines.join("\n").trim().to_string();

        Ok(ExtractedDocument { title, metadata, normalized_text, outbound_links })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::Content;
    use lopdf::{dictionary, Object, Stream};

    #[test]
    fn test_extract_minimal_pdf() {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();

        // Create font object
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });

        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        });

        // Page content stream
        let content = Content {
            operations: vec![
                lopdf::content::Operation::new("BT", vec![]),
                lopdf::content::Operation::new("Tf", vec!["F1".into(), 12.into()]),
                lopdf::content::Operation::new(
                    "Tj",
                    vec![Object::string_literal("1. Introduction to Systems")],
                ),
                lopdf::content::Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));

        // Page dictionary
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
        });

        // Pages dictionary
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        // Catalog
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();

        let extractor = PdfExtractor::new();
        let extracted = extractor.extract(Path::new("test.pdf"), &bytes).unwrap();

        assert!(extracted.normalized_text.contains("<!-- Page 1 -->"));
        assert!(extracted.normalized_text.contains("## 1. Introduction to Systems"));
    }
}
