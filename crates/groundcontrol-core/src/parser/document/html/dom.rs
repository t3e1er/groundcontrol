//! HTML DOM tree visitor, boilerplate stripper, and markdown synthesizer.

use groundcontrol_common::types::DocLink;
use scraper::{ElementRef, Node, Selector};

use super::links::parse_anchor_link;

/// Recursively traverses a DOM element and synthesizes markdown lines.
pub fn render_node_to_markdown(
    element: &ElementRef,
    lines: &mut Vec<String>,
    links: &mut Vec<DocLink>,
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
pub fn collect_text_and_links(element: &ElementRef, links: &mut Vec<DocLink>) -> String {
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
                            if let Some(doc_link) = parse_anchor_link(href, &text) {
                                links.push(doc_link);
                            }
                            out.push_str(&format!("[{}]({})", text, href.trim()));
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
pub fn render_table(table: &ElementRef, lines: &mut Vec<String>, links: &mut Vec<DocLink>) {
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
