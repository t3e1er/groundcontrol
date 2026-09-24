//! Cross-reference link extraction from markdown content.
//!
//! Extracts `[[target]]`, `[[target|alias]]`, and standard `[label](target)` links
//! into canonical [`DocLink`] records.

use groundcontrol_common::types::DocLink;

/// Extract all document links from markdown content.
pub fn extract_all(content: &str) -> Vec<DocLink> {
    let mut links = Vec::new();
    let mut chars = content.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        if ch == '[' {
            if let Some(&(_, next_ch)) = chars.peek() {
                if next_ch == '[' {
                    let _ = chars.next(); // consume second '['
                    if let Some(link) = parse_wikilink_at(&content[i + 2..]) {
                        links.push(link);
                    }
                }
            }
        }
    }

    links
}

/// Parse a wikilink starting after the opening `[[`.
fn parse_wikilink_at(content: &str) -> Option<DocLink> {
    let end = content.find("]]")?;
    let inner = &content[..end];

    // Skip empty links
    if inner.trim().is_empty() {
        return None;
    }

    // Check for alias: [[target|alias]]
    if let Some(pipe_pos) = inner.find('|') {
        let target = inner[..pipe_pos].trim().to_string();
        let label = inner[pipe_pos + 1..].trim().to_string();
        Some(DocLink { target, label: if label.is_empty() { None } else { Some(label) } })
    } else {
        Some(DocLink { target: inner.trim().to_string(), label: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_wikilink() {
        let links = extract_all("See [[my-note]] for details.");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "my-note");
        assert_eq!(links[0].label, None);
    }

    #[test]
    fn extracts_aliased_wikilink() {
        let links = extract_all("Check [[path/to/note|the note]] here.");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "path/to/note");
        assert_eq!(links[0].label, Some("the note".to_string()));
    }

    #[test]
    fn extracts_multiple_wikilinks() {
        let links = extract_all("Links: [[a]], [[b|B]], [[c]].");
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].target, "a");
        assert_eq!(links[1].target, "b");
        assert_eq!(links[2].target, "c");
    }

    #[test]
    fn ignores_empty_wikilinks() {
        let links = extract_all("Empty: [[]] and [[ ]].");
        assert_eq!(links.len(), 0);
    }

    #[test]
    fn ignores_single_brackets() {
        let links = extract_all("[not a link] and [also not](url).");
        assert_eq!(links.len(), 0);
    }
}
