//! HTML document extractor.
//!
//! Extracts structured Markdown prose, title, metadata, and outbound hyperlinks
//! from HTML documentation articles using pure-Rust DOM parsing (`scraper`).
//! Strips non-content boilerplate (`<script>`, `<style>`, `<nav>`, `<header>`, `<footer>`, etc.).

pub mod dom;
pub mod links;
pub mod metadata;

use std::path::Path;

use scraper::{Html, Selector};

use groundcontrol_common::ports::{DocumentExtractor, ExtractedDocument};
use groundcontrol_common::Result;

use self::dom::render_node_to_markdown;
use self::metadata::extract_metadata;

/// Extractor for rich HTML documentation files (.html, .htm).
#[derive(Debug, Default, Clone)]
pub struct HtmlDocExtractor;

impl HtmlDocExtractor {
    /// Create a new HTML document extractor.
    pub fn new() -> Self {
        Self
    }
}

impl DocumentExtractor for HtmlDocExtractor {
    fn can_extract(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"))
            .unwrap_or(false)
    }

    fn extract(&self, _path: &Path, bytes: &[u8]) -> Result<ExtractedDocument> {
        let text = String::from_utf8_lossy(bytes);
        let document = Html::parse_document(&text);

        let (mut title, metadata) = extract_metadata(&document);

        // Select main content container or fallback to <body> or root
        let content_node = if let Ok(article_sel) = Selector::parse("article") {
            document.select(&article_sel).next()
        } else {
            None
        }
        .or_else(|| {
            if let Ok(main_sel) = Selector::parse("main") {
                document.select(&main_sel).next()
            } else {
                None
            }
        })
        .or_else(|| {
            if let Ok(body_sel) = Selector::parse("body") {
                document.select(&body_sel).next()
            } else {
                None
            }
        })
        .unwrap_or_else(|| document.root_element());

        let mut outbound_links = Vec::new();
        let mut markdown_lines = Vec::new();

        // If title wasn't found in <title>, search for first <h1>
        if title.is_none() {
            if let Ok(h1_sel) = Selector::parse("h1") {
                if let Some(h1) = content_node.select(&h1_sel).next() {
                    let h1_text = h1.text().collect::<Vec<_>>().join(" ").trim().to_string();
                    if !h1_text.is_empty() {
                        title = Some(h1_text);
                    }
                }
            }
        }

        // Convert content node tree to markdown
        render_node_to_markdown(&content_node, &mut markdown_lines, &mut outbound_links);

        // Deduplicate outbound links while preserving order
        let mut seen = std::collections::HashSet::new();
        outbound_links.retain(|link| seen.insert(link.target.clone()));

        // Collapse empty lines
        let mut cleaned_lines = Vec::new();
        let mut prev_empty = false;
        for line in markdown_lines {
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

    #[test]
    fn test_extract_html_article() {
        let html = br#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>Architecture Guide</title>
            <meta name="author" content="Engineering Team">
        </head>
        <body>
            <nav><a href="/home">Home</a></nav>
            <article>
                <h1>Storage Engine</h1>
                <p>The storage engine uses <code>SQLite</code> and Tantivy for <strong>hybrid retrieval</strong>.</p>
                <p>See <a href="docs/search.md">Search Docs</a> for more details.</p>
                <h2>Performance</h2>
                <ul>
                    <li>Sub-millisecond retrieval</li>
                    <li>Zero C runtime</li>
                </ul>
            </article>
            <footer>Copyright 2026</footer>
        </body>
        </html>
        "#;

        let extractor = HtmlDocExtractor::new();
        let doc = extractor.extract(Path::new("guide.html"), html).unwrap();

        assert_eq!(doc.title.as_deref(), Some("Architecture Guide"));
        assert_eq!(doc.metadata.get("author").map(|s| s.as_str()), Some("Engineering Team"));

        // Verify boilerplate is stripped
        assert!(!doc.normalized_text.contains("Home"));
        assert!(!doc.normalized_text.contains("Copyright 2026"));

        // Verify markdown conversion
        assert!(doc.normalized_text.contains("# Storage Engine"));
        assert!(doc.normalized_text.contains("The storage engine uses `SQLite` and Tantivy"));
        assert!(doc.normalized_text.contains("[Search Docs](docs/search.md)"));
        assert!(doc.normalized_text.contains("## Performance"));
        assert!(doc.normalized_text.contains("* Sub-millisecond retrieval"));

        // Verify link extraction
        assert_eq!(doc.outbound_links.len(), 1);
        assert_eq!(doc.outbound_links[0].target, "docs/search.md");
        assert_eq!(doc.outbound_links[0].label.as_deref(), Some("Search Docs"));
    }
}
