//! Microsoft Word OpenXML main body (`word/document.xml`) parser.

use groundcontrol_common::types::DocLink;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashMap;

/// Parse `word/document.xml` into structured Markdown, outbound `DocLink`s, and the first heading.
pub fn parse_document_xml(
    xml: &str,
    rels: &HashMap<String, String>,
) -> (String, Vec<DocLink>, Option<String>) {
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
                        links.push(DocLink {
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
