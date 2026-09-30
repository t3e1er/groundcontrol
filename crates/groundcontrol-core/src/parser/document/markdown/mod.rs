//! Markdown knowledge base parsing: frontmatter extraction, wikilink detection, and metadata.

pub mod cmark;
pub mod frontmatter;
pub mod wikilink;

pub use cmark::{extract_headings, Heading};

use std::path::Path;

use groundcontrol_common::types::Document;
use groundcontrol_common::Result;

/// Parse a markdown file into a structured `Document`.
///
/// Extracts frontmatter, cross-reference links, tags, title, and computes content hash.
pub fn parse_document(path: &Path, content: &str) -> Result<Document> {
    let frontmatter = frontmatter::extract(content);
    let (body, body_offset) = frontmatter::strip_frontmatter_with_offset(content);
    let links = wikilink::extract_all(body);
    let tags = extract_tags(body, &frontmatter);
    let title = extract_title(body, &frontmatter);
    let template = frontmatter
        .as_ref()
        .and_then(|fm| fm.get("template"))
        .and_then(serde_json::Value::as_str)
        .map(String::from);
    let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();

    Ok(Document {
        path: path.to_string_lossy().to_string(),
        frontmatter,
        title,
        tags,
        links,
        template,
        content: body.to_string(),
        content_hash,
        body_offset,
    })
}

/// Chunk a markdown document into embedding chunks, adjusting chunk byte offsets and line numbers
/// so they are absolute with respect to the source file on disk.
pub fn chunk_markdown_document(
    doc_path: &str,
    full_content: &str,
    doc: &Document,
    config: &groundcontrol_common::config::ChunkingConfig,
) -> Vec<groundcontrol_common::types::Chunk> {
    let mut chunks =
        crate::parser::document::chunker::chunk_document(doc_path, &doc.content, config);
    if doc.body_offset > 0 && doc.body_offset <= full_content.len() {
        let line_offset =
            full_content.as_bytes()[..doc.body_offset].iter().filter(|&&b| b == b'\n').count();
        for chunk in &mut chunks {
            chunk.start_byte += doc.body_offset;
            chunk.end_byte += doc.body_offset;
            chunk.start_line += line_offset;
            chunk.end_line += line_offset;
        }
    }
    chunks
}

/// Extract tags from frontmatter `tags:` field and inline `#tag` references.
pub fn extract_tags(body: &str, frontmatter: &Option<serde_json::Value>) -> Vec<String> {
    let mut tags = Vec::new();

    // From frontmatter
    if let Some(fm) = frontmatter {
        if let Some(arr) = fm.get("tags").and_then(|v| v.as_array()) {
            for tag in arr {
                if let Some(s) = tag.as_str() {
                    tags.push(s.to_string());
                }
            }
        }
    }

    // From inline #tags (simple regex-free parser)
    for word in body.split_whitespace() {
        if let Some(tag) = word.strip_prefix('#') {
            let tag = tag.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
            if !tag.is_empty() && tag.chars().next().is_some_and(|c| c.is_alphabetic()) {
                tags.push(tag.to_string());
            }
        }
    }

    tags.sort();
    tags.dedup();
    tags
}

/// Extract title from frontmatter `title:` field or first `# Heading`.
pub fn extract_title(body: &str, frontmatter: &Option<serde_json::Value>) -> Option<String> {
    // Prefer frontmatter title
    if let Some(fm) = frontmatter {
        if let Some(title) = fm.get("title").and_then(|v| v.as_str()) {
            return Some(title.to_string());
        }
    }

    // Fall back to first H1
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return Some(heading.trim().to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_simple_document() {
        let content = r#"---
title: Test Note
tags: [rust, learning]
template: decision-record
---

# Test Note

This links to [[another-note]] and [[third|aliased link]].

Some #inline-tag here.
"#;
        let doc = parse_document(&PathBuf::from("test.md"), content).unwrap();
        assert_eq!(doc.title, Some("Test Note".to_string()));
        assert_eq!(doc.tags, vec!["inline-tag", "learning", "rust"]);
        assert_eq!(doc.links.len(), 2);
        assert_eq!(doc.links[0].target, "another-note");
        assert_eq!(doc.links[1].label, Some("aliased link".to_string()));
        assert_eq!(doc.template, Some("decision-record".to_string()));
    }

    #[test]
    fn chunk_markdown_document_aligns_with_file_bytes() {
        let content =
            "---\ntitle: Frontmatter Note\ntags: [test]\n---\n\n# Heading\n\nFirst body paragraph.";
        let doc = parse_document(&PathBuf::from("note.md"), content).unwrap();
        assert!(doc.body_offset > 0);

        let config = groundcontrol_common::config::ChunkingConfig::default();
        let chunks = chunk_markdown_document("note.md", content, &doc, &config);
        assert!(!chunks.is_empty());

        for chunk in &chunks {
            // Verify that slicing original full file content by start_byte..end_byte matches chunk text
            let file_slice = &content.as_bytes()[chunk.start_byte..chunk.end_byte];
            let file_text = std::str::from_utf8(file_slice).unwrap();
            assert_eq!(file_text, chunk.text);
            assert!(chunk.start_byte >= doc.body_offset);
            assert!(chunk.start_line > 4); // Line numbers correctly offset past frontmatter
        }
    }
}
