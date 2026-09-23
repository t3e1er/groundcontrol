//! Markdown structure extraction using pulldown-cmark.
//!
//! Extracts headings and sections for heading-aware prose chunking.

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

/// A heading found in the markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// Heading level (1-6).
    pub level: u8,
    /// Text content of the heading.
    pub text: String,
    /// Byte offset where this heading starts in the source.
    pub byte_offset: usize,
}

/// Extract all headings from markdown content.
pub fn extract_headings(content: &str) -> Vec<Heading> {
    let parser = Parser::new(content);
    let mut headings = Vec::new();
    let mut in_heading = false;
    let mut current_level: u8 = 0;
    let mut current_text = String::new();
    let mut heading_offset: usize = 0;

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = true;
                current_level = heading_level_to_u8(level);
                current_text.clear();
                heading_offset = range.start;
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
                headings.push(Heading {
                    level: current_level,
                    text: current_text.clone(),
                    byte_offset: heading_offset,
                });
            }
            Event::Text(text) if in_heading => {
                current_text.push_str(&text);
            }
            Event::Code(code) if in_heading => {
                current_text.push('`');
                current_text.push_str(&code);
                current_text.push('`');
            }
            _ => {}
        }
    }

    headings
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_headings() {
        let content = "# Title\n\nParagraph.\n\n## Section A\n\nContent.\n\n### Sub B\n";
        let headings = extract_headings(content);
        assert_eq!(headings.len(), 3);
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[0].text, "Title");
        assert_eq!(headings[1].level, 2);
        assert_eq!(headings[1].text, "Section A");
        assert_eq!(headings[2].level, 3);
        assert_eq!(headings[2].text, "Sub B");
    }
}
