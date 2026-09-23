//! PDF page text stream extraction and synthetic line anchor synthesis.

use groundcontrol_common::types::DocLink;
use lopdf::Document;

use super::links::extract_annots_links;

/// Extract page streams, headings, and link annotations from a loaded PDF document.
pub fn extract_pages_and_headings(
    doc: &Document,
    lines: &mut Vec<String>,
    outbound_links: &mut Vec<DocLink>,
    first_heading: &mut Option<String>,
) {
    let pages = doc.get_pages();

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

                // Heading heuristic: numbered section or short title-case line
                if is_pdf_heading(trimmed) {
                    if first_heading.is_none() {
                        *first_heading = Some(trimmed.to_string());
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
                    extract_annots_links(doc, annots, outbound_links);
                }
            }
        }
    }
}

/// Heuristic to detect section headings in extracted PDF text lines.
pub fn is_pdf_heading(line: &str) -> bool {
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
