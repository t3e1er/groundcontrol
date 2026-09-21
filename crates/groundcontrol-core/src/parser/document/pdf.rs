//! PDF document extractor.
//!
//! Extracts text pages, `<!-- Page N -->` line anchors, metadata, and outbound URI
//! annotations from vector/text PDF documents using pure-Rust `lopdf`.
//! Strictly scoped to text/vector PDFs; scanned PDFs emit a warning.

use std::collections::HashMap;
use std::path::Path;

use lopdf::{Document, Object};

use groundcontrol_common::ports::{DocumentExtractor, DocumentLink, ExtractedDocument};
use groundcontrol_common::{Error, Result};

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

        let mut metadata = HashMap::new();
        let mut title = None;

        // 1. Extract metadata from /Info dictionary if present
        if let Ok(info_dict) =
            doc.trailer.get(b"Info").and_then(|obj| doc.get_object(obj.as_reference()?))
        {
            if let Ok(dict) = info_dict.as_dict() {
                if let Some(t) = dict.get(b"Title").ok().and_then(extract_string_value) {
                    if !t.is_empty() {
                        title = Some(t.clone());
                        metadata.insert("title".to_string(), t);
                    }
                }
                if let Some(a) = dict.get(b"Author").ok().and_then(extract_string_value) {
                    if !a.is_empty() {
                        metadata.insert("author".to_string(), a);
                    }
                }
                if let Some(s) = dict.get(b"Subject").ok().and_then(extract_string_value) {
                    if !s.is_empty() {
                        metadata.insert("subject".to_string(), s);
                    }
                }
            }
        }

        // 2. Extract pages and annotations
        let pages = doc.get_pages();
        let mut lines = Vec::new();
        let mut outbound_links = Vec::new();
        let mut first_heading = None;

        for (&page_num, &page_id) in pages.iter() {
            lines.push(String::new());
            lines.push(format!("<!-- Page {} -->", page_num));
            lines.push(String::new());

            // Extract page text via lopdf
            if let Ok(page_text) = doc.extract_text(&[page_num]) {
                for raw_line in page_text.lines() {
                    let trimmed = raw_line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    // Simple heading heuristic: numbered section or short title-case line
                    if is_pdf_heading(trimmed) {
                        if first_heading.is_none() {
                            first_heading = Some(trimmed.to_string());
                        }
                        lines.push(format!("## {}", trimmed));
                    } else {
                        lines.push(trimmed.to_string());
                    }
                }
            }

            // Extract page link annotations
            if let Ok(page_obj) = doc.get_object(page_id) {
                if let Ok(page_dict) = page_obj.as_dict() {
                    if let Ok(annots) = page_dict.get(b"Annots") {
                        extract_annots_links(&doc, annots, &mut outbound_links);
                    }
                }
            }
        }

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

/// Extracts a UTF-8 string from a lopdf String or Name object.
fn extract_string_value(obj: &Object) -> Option<String> {
    match obj {
        Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Object::Name(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

/// Heuristic to detect section headings in extracted PDF text lines.
fn is_pdf_heading(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.is_empty() || words.len() > 10 {
        return false;
    }

    // e.g. "1. Introduction" or "1.1 Architecture" or "Section 3"
    let first = words[0];
    if (first.ends_with('.')
        && first.trim_end_matches('.').chars().all(|c| c.is_ascii_digit() || c == '.'))
        || first.eq_ignore_ascii_case("section")
        || first.eq_ignore_ascii_case("chapter")
        || first.eq_ignore_ascii_case("abstract")
        || first.eq_ignore_ascii_case("references")
    {
        return true;
    }

    false
}

/// Extract URI links from `/Annots` object.
fn extract_annots_links(doc: &Document, annots_obj: &Object, links: &mut Vec<DocumentLink>) {
    let annot_list = match annots_obj {
        Object::Array(arr) => arr.clone(),
        Object::Reference(r) => {
            if let Ok(Object::Array(arr)) = doc.get_object(*r) {
                arr.clone()
            } else {
                return;
            }
        }
        _ => return,
    };

    for item in annot_list {
        let dict = match item {
            Object::Dictionary(d) => Some(d),
            Object::Reference(r) => doc.get_object(r).ok().and_then(|o| o.as_dict().ok().cloned()),
            _ => None,
        };

        if let Some(d) = dict {
            // Check if Subtype is Link
            if let Some(subtype) = d.get(b"Subtype").ok().and_then(extract_string_value) {
                if subtype.eq_ignore_ascii_case("Link") {
                    // Check action dictionary /A
                    if let Ok(action_obj) = d.get(b"A") {
                        let action_dict = match action_obj {
                            Object::Dictionary(ad) => Some(ad.clone()),
                            Object::Reference(r) => {
                                doc.get_object(*r).ok().and_then(|o| o.as_dict().ok().cloned())
                            }
                            _ => None,
                        };

                        if let Some(ad) = action_dict {
                            if let Some(uri) = ad.get(b"URI").ok().and_then(extract_string_value) {
                                let clean_uri = uri.trim();
                                if !clean_uri.is_empty() {
                                    links.push(DocumentLink {
                                        target: clean_uri.to_string(),
                                        label: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
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
