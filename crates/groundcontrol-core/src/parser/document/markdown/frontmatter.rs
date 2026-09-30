//! YAML frontmatter extraction from markdown files.

use serde_json::Value;

/// Extract YAML frontmatter from markdown content.
///
/// Returns `None` if no valid frontmatter block is found.
/// Frontmatter must be delimited by `---` at the very start of the file.
pub fn extract(content: &str) -> Option<Value> {
    let content = content.trim_start_matches('\u{feff}'); // strip BOM
    if !content.starts_with("---") {
        return None;
    }

    let after_opening = &content[3..];
    let end_pos = after_opening.find("\n---")?;
    let yaml_str = after_opening[..end_pos].trim();

    // Parse YAML into a JSON Value for uniform handling
    serde_yaml::from_str(yaml_str).ok()
}

/// Strip the frontmatter block from content, returning just the body along with its byte offset in `content`.
pub fn strip_frontmatter_with_offset(content: &str) -> (&str, usize) {
    let bom_offset = if content.starts_with('\u{feff}') { '\u{feff}'.len_utf8() } else { 0 };
    let unbom = &content[bom_offset..];
    if !unbom.starts_with("---") {
        return (unbom, bom_offset);
    }

    let after_opening = &unbom[3..];
    if let Some(end_pos) = after_opening.find("\n---") {
        let after_closing = &after_opening[end_pos + 4..];
        let trimmed = after_closing.trim_start_matches(|c| c == '\r' || c == '\n');
        let offset = bom_offset + 3 + end_pos + 4 + (after_closing.len() - trimmed.len());
        (trimmed, offset)
    } else {
        (unbom, bom_offset)
    }
}

/// Strip the frontmatter block from content, returning just the body.
pub fn strip_frontmatter(content: &str) -> &str {
    strip_frontmatter_with_offset(content).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_valid_frontmatter() {
        let content = "---\ntitle: Hello\ntags:\n  - rust\n  - test\n---\n\n# Body";
        let fm = extract(content).unwrap();
        assert_eq!(fm["title"], "Hello");
        assert_eq!(fm["tags"][0], "rust");
    }

    #[test]
    fn returns_none_for_no_frontmatter() {
        let content = "# Just a heading\n\nSome content.";
        assert!(extract(content).is_none());
    }

    #[test]
    fn strips_frontmatter_correctly() {
        let content = "---\ntitle: Hello\n---\n\n# Body here";
        let body = strip_frontmatter(content);
        assert_eq!(body, "# Body here");
    }

    #[test]
    fn handles_bom() {
        let content = "\u{feff}---\ntitle: BOM Test\n---\n\nContent";
        let fm = extract(content).unwrap();
        assert_eq!(fm["title"], "BOM Test");
    }

    #[test]
    fn strips_frontmatter_with_offset_lf_and_crlf() {
        let content_lf = "---\ntitle: LF\n---\n\n# Body";
        let (body_lf, offset_lf) = strip_frontmatter_with_offset(content_lf);
        assert_eq!(body_lf, "# Body");
        assert_eq!(&content_lf[offset_lf..], body_lf);

        let content_crlf = "---\r\ntitle: CRLF\r\n---\r\n\r\n# Body";
        let (body_crlf, offset_crlf) = strip_frontmatter_with_offset(content_crlf);
        assert_eq!(body_crlf, "# Body");
        assert_eq!(&content_crlf[offset_crlf..], body_crlf);
    }
}
