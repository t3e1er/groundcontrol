//! Markdown parsing: frontmatter extraction, wikilink detection, chunking.

pub mod artifact;
pub mod chunker;
pub mod code;
pub mod document;
pub mod frontmatter;
pub mod markdown;
pub mod wikilink;

pub use artifact::ArtifactParser;

use groundcontrol_common::types::Document;
use groundcontrol_common::Result;
use std::path::Path;

/// Parse a markdown file into a structured `Document`.
///
/// Extracts frontmatter, wikilinks, tags, title, and computes content hash.
pub fn parse_document(path: &Path, content: &str) -> Result<Document> {
    let frontmatter = frontmatter::extract(content);
    let body = frontmatter::strip_frontmatter(content);
    let wikilinks = wikilink::extract_all(&body);
    let tags = extract_tags(&body, &frontmatter);
    let title = extract_title(&body, &frontmatter);
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
        wikilinks,
        template,
        content: body.to_string(),
        content_hash,
    })
}

/// Extract tags from frontmatter `tags:` field and inline `#tag` references.
fn extract_tags(body: &str, frontmatter: &Option<serde_json::Value>) -> Vec<String> {
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
fn extract_title(body: &str, frontmatter: &Option<serde_json::Value>) -> Option<String> {
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
        assert_eq!(doc.wikilinks.len(), 2);
        assert_eq!(doc.wikilinks[0].target, "another-note");
        assert_eq!(doc.wikilinks[1].alias, Some("aliased link".to_string()));
        assert_eq!(doc.template, Some("decision-record".to_string()));
    }
}
