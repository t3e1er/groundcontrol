//! HTML document extractor.
//!
//! Extracts structured Markdown prose, title, metadata, and outbound hyperlinks
//! from HTML documentation articles using pure-Rust DOM parsing (`scraper`).
//! Strips non-content boilerplate (`<script>`, `<style>`, `<nav>`, `<header>`, `<footer>`, etc.).

use std::collections::HashMap;
use std::path::Path;

use scraper::{ElementRef, Html, Node, Selector};

use groundcontrol_common::ports::{DocumentExtractor, DocumentLink, ExtractedDocument};
use groundcontrol_common::Result;

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

        let mut metadata = HashMap::new();
        let mut title = None;

        // 1. Extract <title>
        if let Ok(title_sel) = Selector::parse("title") {
            if let Some(elem) = document.select(&title_sel).next() {
                let t = elem.text().collect::<Vec<_>>().join(" ").trim().to_string();
                if !t.is_empty() {
                    title = Some(t.clone());
                    metadata.insert("title".to_string(), t);
                }
            }
        }

        // 2. Extract <meta> tags
        if let Ok(meta_sel) = Selector::parse("meta") {
            for elem in document.select(&meta_sel) {
                let name = elem.value().attr("name").or_else(|| elem.value().attr("property"));
                let content = elem.value().attr("content");
                if let (Some(n), Some(c)) = (name, content) {
                    if !n.is_empty() && !c.is_empty() {
                        metadata.insert(n.to_string(), c.to_string());
                    }
                }
            }
        }

        // 3. Select main content container or fallback to <body> or root
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

/// Recursively traverses a DOM element and synthesizes markdown lines.
fn render_node_to_markdown(
    element: &ElementRef,
    lines: &mut Vec<String>,
    links: &mut Vec<DocumentLink>,
) {
    let tag = element.value().name();

    // Skip boilerplate tags
    if matches!(
        tag,
        "script"
            | "style"
            | "noscript"
            | "svg"
            | "canvas"
            | "iframe"
            | "nav"
            | "header"
            | "footer"
            | "aside"
    ) {
        return;
    }

    // Check class names for boilerplate banners/sidebars
    if let Some(class) = element.value().attr("class") {
        let class_lower = class.to_ascii_lowercase();
        if class_lower.contains("navbar")
            || class_lower.contains("sidebar")
            || class_lower.contains("cookie-banner")
            || class_lower.contains("advertisement")
        {
            return;
        }
    }

    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = tag[1..].parse::<usize>().unwrap_or(1);
            let prefix = "#".repeat(level);
            let text = collect_text_and_links(element, links);
            if !text.is_empty() {
                lines.push(String::new());
                lines.push(format!("{} {}", prefix, text));
                lines.push(String::new());
            }
        }
        "p" => {
            let text = collect_text_and_links(element, links);
            if !text.is_empty() {
                lines.push(text);
                lines.push(String::new());
            }
        }
        "li" => {
            let text = collect_text_and_links(element, links);
            if !text.is_empty() {
                lines.push(format!("* {}", text));
            }
        }
        "pre" => {
            let code_text = element.text().collect::<Vec<_>>().join("");
            lines.push(String::new());
            lines.push("```".to_string());
            lines.push(code_text.trim_end().to_string());
            lines.push("```".to_string());
            lines.push(String::new());
        }
        "blockquote" => {
            let text = collect_text_and_links(element, links);
            if !text.is_empty() {
                lines.push(format!("> {}", text));
                lines.push(String::new());
            }
        }
        "table" => {
            render_table(element, lines, links);
        }
        _ => {
            // Recurse into children
            for child in element.children() {
                if let Some(child_elem) = ElementRef::wrap(child) {
                    render_node_to_markdown(&child_elem, lines, links);
                }
            }
        }
    }
}

/// Collects inline text, formatting, and hyperlinks from an element.
fn collect_text_and_links(element: &ElementRef, links: &mut Vec<DocumentLink>) -> String {
    let mut out = String::new();

    for child in element.children() {
        match child.value() {
            Node::Text(t) => {
                out.push_str(t);
            }
            Node::Element(elem) => {
                if let Some(child_ref) = ElementRef::wrap(child) {
                    let name = elem.name();
                    if name == "a" {
                        let text =
                            child_ref.text().collect::<Vec<_>>().join(" ").trim().to_string();
                        if let Some(href) = elem.attr("href") {
                            let clean_href = href.trim();
                            if !clean_href.is_empty()
                                && !clean_href.starts_with('#')
                                && !clean_href.starts_with("javascript:")
                            {
                                links.push(DocumentLink {
                                    target: clean_href.to_string(),
                                    label: if text.is_empty() { None } else { Some(text.clone()) },
                                });
                            }
                            out.push_str(&format!("[{}]({})", text, clean_href));
                        } else {
                            out.push_str(&text);
                        }
                    } else if name == "code" {
                        let code_txt = child_ref.text().collect::<Vec<_>>().join("");
                        out.push_str(&format!("`{}`", code_txt));
                    } else if name == "strong" || name == "b" {
                        out.push_str(&format!("**{}**", collect_text_and_links(&child_ref, links)));
                    } else if name == "em" || name == "i" {
                        out.push_str(&format!("*{}*", collect_text_and_links(&child_ref, links)));
                    } else {
                        out.push_str(&collect_text_and_links(&child_ref, links));
                    }
                }
            }
            _ => {}
        }
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Renders an HTML `<table>` into a GitHub Flavored Markdown table.
fn render_table(table: &ElementRef, lines: &mut Vec<String>, links: &mut Vec<DocumentLink>) {
    let mut header_rows = Vec::new();
    let mut body_rows = Vec::new();

    if let Ok(tr_sel) = Selector::parse("tr") {
        for tr in table.select(&tr_sel) {
            let mut row = Vec::new();
            let is_header = tr.select(&Selector::parse("th").unwrap()).next().is_some();

            for child in tr.children() {
                if let Some(cell) = ElementRef::wrap(child) {
                    if cell.value().name() == "th" || cell.value().name() == "td" {
                        let text = collect_text_and_links(&cell, links);
                        row.push(text.replace('|', "\\|"));
                    }
                }
            }

            if !row.is_empty() {
                if is_header {
                    header_rows.push(row);
                } else {
                    body_rows.push(row);
                }
            }
        }
    }

    if header_rows.is_empty() && body_rows.is_empty() {
        return;
    }

    lines.push(String::new());

    // Header row
    let headers = if let Some(first_h) = header_rows.first() {
        first_h.clone()
    } else if let Some(first_b) = body_rows.first() {
        first_b.clone()
    } else {
        return;
    };

    lines.push(format!("| {} |", headers.join(" | ")));
    let separators: Vec<String> = headers.iter().map(|_| "---".to_string()).collect();
    lines.push(format!("| {} |", separators.join(" | ")));

    let start_idx = if header_rows.is_empty() { 1 } else { 0 };
    for row in body_rows.into_iter().skip(start_idx) {
        lines.push(format!("| {} |", row.join(" | ")));
    }

    lines.push(String::new());
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
