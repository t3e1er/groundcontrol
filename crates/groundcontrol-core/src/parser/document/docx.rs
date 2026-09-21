//! Microsoft Word (.docx) OpenXML document extractor.
//!
//! Extracts structured Markdown text, headings, tables, metadata, and outbound
//! hyperlinks from Word (.docx) files using pure-Rust `zip` and `quick-xml`.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use zip::ZipArchive;

use groundcontrol_common::ports::{DocumentExtractor, DocumentLink, ExtractedDocument};
use groundcontrol_common::{Error, Result};

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

/// Parse `docProps/core.xml` for title, creator, etc.
fn parse_core_properties(
    xml: &str,
    metadata: &mut HashMap<String, String>,
    title: &mut Option<String>,
) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut current_tag = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                current_tag = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
            }
            Ok(Event::Text(ref e)) => {
                let text_raw = std::str::from_utf8(e.as_ref()).unwrap_or("");
                let val = quick_xml::escape::unescape(text_raw)
                    .unwrap_or(std::borrow::Cow::Borrowed(text_raw))
                    .trim()
                    .to_string();
                if !val.is_empty() {
                    match current_tag.as_str() {
                        "title" => {
                            *title = Some(val.clone());
                            metadata.insert("title".to_string(), val);
                        }
                        "creator" => {
                            metadata.insert("author".to_string(), val);
                        }
                        "description" | "subject" => {
                            metadata.insert("description".to_string(), val);
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(_)) => {
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
}

/// Parse `word/_rels/document.xml.rels` to map relationship Id to Target URI.
fn parse_relationships(xml: &str, rels: &mut HashMap<String, String>) {
    let mut reader = Reader::from_str(xml);

    loop {
        match reader.read_event() {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if e.local_name().as_ref() == b"Relationship" {
                    let mut id = None;
                    let mut target = None;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"Id" {
                            id = String::from_utf8(attr.value.to_vec()).ok();
                        } else if attr.key.as_ref() == b"Target" {
                            target = String::from_utf8(attr.value.to_vec()).ok();
                        }
                    }
                    if let (Some(i), Some(t)) = (id, target) {
                        rels.insert(i, t);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
}

/// Parse `word/document.xml` into structured Markdown.
fn parse_document_xml(
    xml: &str,
    rels: &HashMap<String, String>,
) -> (String, Vec<DocumentLink>, Option<String>) {
    let mut reader = Reader::from_str(xml);
    let mut lines = Vec::new();
    let mut links = Vec::new();
    let mut first_heading = None;

    let mut in_cell = false;

    let mut current_p_style = None;
    let mut current_hyperlink_target = None;
    let mut current_text = String::new();

    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"tbl" => {
                        table_rows.clear();
                    }
                    b"tr" => {
                        current_row.clear();
                    }
                    b"tc" => {
                        in_cell = true;
                        current_text.clear();
                    }
                    b"p" => {
                        current_p_style = None;
                        current_text.clear();
                    }
                    b"pStyle" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == b"val" {
                                current_p_style = String::from_utf8(attr.value.to_vec()).ok();
                            }
                        }
                    }
                    b"hyperlink" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == b"id" {
                                if let Ok(id) = String::from_utf8(attr.value.to_vec()) {
                                    if let Some(target) = rels.get(&id) {
                                        current_hyperlink_target = Some(target.clone());
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"pStyle" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == b"val" {
                                current_p_style = String::from_utf8(attr.value.to_vec()).ok();
                            }
                        }
                    }
                    b"tab" => {
                        current_text.push('\t');
                    }
                    b"br" => {
                        current_text.push('\n');
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                let text_raw = std::str::from_utf8(e.as_ref()).unwrap_or("");
                let txt = quick_xml::escape::unescape(text_raw)
                    .unwrap_or(std::borrow::Cow::Borrowed(text_raw));
                if let Some(ref target) = current_hyperlink_target {
                    let link_text = txt.trim().to_string();
                    if !link_text.is_empty() {
                        links.push(DocumentLink {
                            target: target.clone(),
                            label: Some(link_text.clone()),
                        });
                        current_text.push_str(&format!("[{}]({})", link_text, target));
                    }
                } else {
                    current_text.push_str(&txt);
                }
            }
            Ok(Event::End(ref e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"hyperlink" => {
                        current_hyperlink_target = None;
                    }
                    b"p" => {
                        let trimmed = current_text.trim();
                        if !trimmed.is_empty() {
                            if in_cell {
                                // Accumulate inside table cell
                            } else {
                                // Top-level paragraph
                                if let Some(ref style) = current_p_style {
                                    let level = heading_level_from_style(style);
                                    if level > 0 {
                                        let prefix = "#".repeat(level);
                                        if first_heading.is_none() {
                                            first_heading = Some(trimmed.to_string());
                                        }
                                        lines.push(String::new());
                                        lines.push(format!("{} {}", prefix, trimmed));
                                        lines.push(String::new());
                                    } else {
                                        lines.push(trimmed.to_string());
                                        lines.push(String::new());
                                    }
                                } else {
                                    lines.push(trimmed.to_string());
                                    lines.push(String::new());
                                }
                            }
                        }
                        current_p_style = None;
                    }
                    b"tc" => {
                        in_cell = false;
                        current_row.push(current_text.trim().replace('|', "\\|"));
                        current_text.clear();
                    }
                    b"tr" => {
                        if !current_row.is_empty() {
                            table_rows.push(current_row.clone());
                        }
                    }
                    b"tbl" => {
                        if !table_rows.is_empty() {
                            emit_gfm_table(&table_rows, &mut lines);
                            table_rows.clear();
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    // Clean up empty lines
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

    (cleaned_lines.join("\n").trim().to_string(), links, first_heading)
}

/// Map OpenXML style names to heading levels 1..6.
fn heading_level_from_style(style: &str) -> usize {
    let lower = style.to_ascii_lowercase();
    if lower == "heading1" || lower == "1" || lower == "title" {
        1
    } else if lower == "heading2" || lower == "2" {
        2
    } else if lower == "heading3" || lower == "3" {
        3
    } else if lower == "heading4" || lower == "4" {
        4
    } else if lower == "heading5" || lower == "5" {
        5
    } else if lower == "heading6" || lower == "6" {
        6
    } else {
        0
    }
}

/// Render collected table rows into GFM markdown table lines.
fn emit_gfm_table(rows: &[Vec<String>], lines: &mut Vec<String>) {
    if rows.is_empty() {
        return;
    }

    lines.push(String::new());

    // First row as header
    let header = &rows[0];
    lines.push(format!("| {} |", header.join(" | ")));

    let separators: Vec<String> = header.iter().map(|_| "---".to_string()).collect();
    lines.push(format!("| {} |", separators.join(" | ")));

    for row in rows.iter().skip(1) {
        lines.push(format!("| {} |", row.join(" | ")));
    }

    lines.push(String::new());
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
            zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
            <cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/">
                <dc:title>System Architecture Spec</dc:title>
                <dc:creator>Alice Engineer</dc:creator>
            </cp:coreProperties>
            "#).unwrap();

            // Write word/_rels/document.xml.rels
            zip.start_file("word/_rels/document.xml.rels", SimpleFileOptions::default()).unwrap();
            zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
            <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="docs/overview.md"/>
            </Relationships>
            "#).unwrap();

            // Write word/document.xml
            zip.start_file("word/document.xml", SimpleFileOptions::default()).unwrap();
            zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
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
            "#).unwrap();

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
