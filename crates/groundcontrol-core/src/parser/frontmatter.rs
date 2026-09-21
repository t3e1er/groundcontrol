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
    let yaml_str = &after_opening[..end_pos].trim();

    // Parse YAML into a JSON Value for uniform handling
    serde_yaml::from_str(yaml_str).ok()
}

/// Strip the frontmatter block from content, returning just the body.
pub fn strip_frontmatter(content: &str) -> &str {
    let content = content.trim_start_matches('\u{feff}');
    if !content.starts_with("---") {
        return content;
    }

    let after_opening = &content[3..];
    if let Some(end_pos) = after_opening.find("\n---") {
        let remainder = &after_opening[end_pos + 4..];
        remainder.trim_start_matches(|c| c == '\r' || c == '\n')
    } else {
        content
    }
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
}
