//! Markdown structure extraction using pulldown-cmark.
//!
//! Extracts headings, sections, and structure for heading-aware chunking.

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

use groundcontrol_common::types::ChunkEmbedPolicy;

/// Classify whether a markdown chunk should receive a dense vector embedding (Anchor)
/// or be indexed solely via BM25 lexical search and graph edges (GraphOnly).
///
/// Under Option 2 V2 Anchor Policy:
/// - Semantic Anchors (~5%–10% of total chunks):
///   - Document title/H1 blocks or preamble (chunk_index == 0 or heading_level <= 1).
///   - Top-level section summaries (first chunk of an H2 section, unless it's a raw list or table).
///   - Architecture Decision Records (ADRs) and architecture decision/context blocks.
/// - Graph-Only (zero neural forward pass, searchable via BM25 and navigable in graph):
///   - Repetitive changelogs and release notes (e.g. `RELEASES.md`, `CHANGELOG.md` chunks > 0).
///   - Deep subsection paragraphs (H3, H4, H5, H6).
///   - Continuation sub-chunks under an H2 (subsequent paragraphs of a split section).
///   - Table-heavy blocks (predominantly markdown tables).
///   - Raw bullet lists (predominantly `- `, `* `, `+ `, or numbered list items).
pub fn classify_markdown_chunk(
    doc_path: &str,
    chunk_index: usize,
    heading_level: usize,
    heading_chain: Option<&str>,
    is_first_chunk_in_section: bool,
    text: &str,
    template: Option<&str>,
) -> ChunkEmbedPolicy {
    let norm_path = doc_path.replace('\\', "/").to_lowercase();
    let file_name = norm_path.rsplit('/').next().unwrap_or(&norm_path);

    // 1. Changelog / Release files: only the document title / overview (chunk 0) is an Anchor.
    // All individual release notes, PR lists, and change bullets are GraphOnly.
    let is_changelog = file_name.starts_with("releases")
        || file_name.starts_with("changelog")
        || file_name.starts_with("changes")
        || file_name.starts_with("history")
        || norm_path.contains("/releases/")
        || norm_path.contains("/changelog/");

    if is_changelog {
        return if chunk_index == 0 {
            ChunkEmbedPolicy::Anchor
        } else {
            ChunkEmbedPolicy::GraphOnly
        };
    }

    // 2. Architecture Decision Records (ADRs) & architectural decision blocks.
    let is_adr = template == Some("adr")
        || norm_path.contains("/adr/")
        || norm_path.contains("/adrs/")
        || file_name.starts_with("adr-")
        || norm_path.contains("/architecture/");

    if is_adr {
        let text_lower = text.to_lowercase();
        if chunk_index == 0
            || heading_level <= 2
            || text_lower.contains("decision")
            || text_lower.contains("context")
            || text_lower.contains("status")
            || text_lower.contains("consequences")
        {
            return ChunkEmbedPolicy::Anchor;
        }
    }

    // 3. Document Title / Preamble / First H1 block
    if chunk_index == 0 || heading_level <= 1 {
        return ChunkEmbedPolicy::Anchor;
    }

    // 4. Check for deep subsections (H3+): always GraphOnly
    if heading_level >= 3 {
        return ChunkEmbedPolicy::GraphOnly;
    }

    // If heading_chain indicates deep nesting (e.g. "A > B > C"), it's deep
    if let Some(chain) = heading_chain {
        if chain.matches('>').count() >= 2 {
            return ChunkEmbedPolicy::GraphOnly;
        }
    }

    // 5. Continuation chunks under H2 (subsequent paragraphs of split sections) are GraphOnly
    if !is_first_chunk_in_section {
        return ChunkEmbedPolicy::GraphOnly;
    }

    // 6. Inspect text for raw list bullets or table rows
    if is_predominantly_list_or_table(text) {
        return ChunkEmbedPolicy::GraphOnly;
    }

    // 7. Top-level section summary (first chunk of H2) with prose content -> Anchor
    if heading_level == 2 {
        return ChunkEmbedPolicy::Anchor;
    }

    ChunkEmbedPolicy::GraphOnly
}

/// Helper to detect if a chunk consists predominantly (> 60% of non-empty lines)
/// of markdown table rows (`|`) or list items (`- `, `* `, `+ `, `1. `).
fn is_predominantly_list_or_table(text: &str) -> bool {
    let mut non_empty_lines = 0;
    let mut list_or_table_lines = 0;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        non_empty_lines += 1;

        if trimmed.starts_with('|')
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("+ ")
            || (trimmed.len() > 2
                && trimmed.chars().next().map_or(false, |c| c.is_ascii_digit())
                && trimmed.contains(". "))
        {
            list_or_table_lines += 1;
        }
    }

    if non_empty_lines == 0 {
        return false;
    }

    (list_or_table_lines as f64 / non_empty_lines as f64) > 0.60
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

    #[test]
    fn test_classify_markdown_chunk_changelogs() {
        // RELEASES.md chunk 0 is Anchor
        assert_eq!(
            classify_markdown_chunk(
                "RELEASES.md",
                0,
                1,
                Some("Rust Releases"),
                true,
                "# Rust Releases\n\nOverview of all stable releases.",
                None
            ),
            ChunkEmbedPolicy::Anchor
        );

        // RELEASES.md chunks > 0 are GraphOnly
        for i in 1..10 {
            assert_eq!(
                classify_markdown_chunk(
                    "RELEASES.md",
                    i,
                    2,
                    Some("Rust Releases > Version 1.70.0"),
                    true,
                    "## Version 1.70.0\n\n- Fix regression in borrow checker (#123)\n- Stabilize OnceLock",
                    None
                ),
                ChunkEmbedPolicy::GraphOnly
            );
        }
    }

    #[test]
    fn test_classify_markdown_chunk_adrs() {
        assert_eq!(
            classify_markdown_chunk(
                "docs/adr/0001-record-format.md",
                1,
                2,
                Some("ADR 1 > Decision"),
                true,
                "## Decision\n\nWe will use SQLite for persistence.",
                Some("adr")
            ),
            ChunkEmbedPolicy::Anchor
        );
    }

    #[test]
    fn test_classify_markdown_chunk_h2_and_deep_subsections() {
        // H1 Title is Anchor
        assert_eq!(
            classify_markdown_chunk(
                "guide.md",
                0,
                1,
                Some("User Guide"),
                true,
                "# User Guide\n\nWelcome to the guide.",
                None
            ),
            ChunkEmbedPolicy::Anchor
        );

        // Top-level H2 prose section is Anchor
        assert_eq!(
            classify_markdown_chunk(
                "guide.md",
                1,
                2,
                Some("User Guide > Getting Started"),
                true,
                "## Getting Started\n\nThis section explains how to initialize ctxvault.",
                None
            ),
            ChunkEmbedPolicy::Anchor
        );

        // Continuation chunk in H2 is GraphOnly
        assert_eq!(
            classify_markdown_chunk(
                "guide.md",
                2,
                2,
                Some("User Guide > Getting Started"),
                false,
                "More details on configuration flags and environment variables.",
                None
            ),
            ChunkEmbedPolicy::GraphOnly
        );

        // Deep subsection H3 is GraphOnly
        assert_eq!(
            classify_markdown_chunk(
                "guide.md",
                3,
                3,
                Some("User Guide > Getting Started > Linux Setup"),
                true,
                "### Linux Setup\n\nInstall build-essential packages.",
                None
            ),
            ChunkEmbedPolicy::GraphOnly
        );

        // Table-heavy block in H2 is GraphOnly
        assert_eq!(
            classify_markdown_chunk(
                "guide.md",
                4,
                2,
                Some("User Guide > Supported Platforms"),
                true,
                "## Supported Platforms\n\n| OS | Architecture | Status |\n|---|---|---|\n| Windows | x64 | Tier 1 |\n| Linux | x64 | Tier 1 |",
                None
            ),
            ChunkEmbedPolicy::GraphOnly
        );

        // List-heavy block in H2 is GraphOnly
        assert_eq!(
            classify_markdown_chunk(
                "guide.md",
                5,
                2,
                Some("User Guide > Checklist"),
                true,
                "## Checklist\n\n- [ ] Item 1\n- [ ] Item 2\n- [ ] Item 3\n- [ ] Item 4",
                None
            ),
            ChunkEmbedPolicy::GraphOnly
        );
    }
}
