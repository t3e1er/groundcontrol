//! Wikilink extraction from markdown content.
//!
//! Supports both `[[target]]` and `[[target|alias]]` syntax.

use groundcontrol_common::types::WikiLink;

/// Extract all wikilinks from markdown content.
pub fn extract_all(content: &str) -> Vec<WikiLink> {
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
fn parse_wikilink_at(content: &str) -> Option<WikiLink> {
    let end = content.find("]]")?;
    let inner = &content[..end];

    // Skip empty links
    if inner.trim().is_empty() {
        return None;
    }

    // Check for alias: [[target|alias]]
    if let Some(pipe_pos) = inner.find('|') {
        let target = inner[..pipe_pos].trim().to_string();
        let alias = inner[pipe_pos + 1..].trim().to_string();
        Some(WikiLink { target, alias: if alias.is_empty() { None } else { Some(alias) } })
    } else {
        Some(WikiLink { target: inner.trim().to_string(), alias: None })
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
        assert_eq!(links[0].alias, None);
    }

    #[test]
    fn extracts_aliased_wikilink() {
        let links = extract_all("Check [[path/to/note|the note]] here.");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "path/to/note");
        assert_eq!(links[0].alias, Some("the note".to_string()));
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
