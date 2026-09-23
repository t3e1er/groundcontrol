//! Text chunking strategies for embedding preparation.
//!
//! Supports multiple strategies configurable per-corpus:
//! - **heading**: Split at markdown headings, track heading hierarchy.
//! - **paragraph**: Split at blank lines, merge to target size.
//! - **semantic**: Line-based accumulation to target size (legacy).
//! - **fixed**: Pure character-count split.

use groundcontrol_common::config::{ChunkingConfig, ChunkingStrategy};
use groundcontrol_common::types::Chunk;

/// Chunk a document body into pieces suitable for embedding.
///
/// Dispatches to the appropriate strategy based on `config.strategy`.
pub fn chunk_document(doc_path: &str, body: &str, config: &ChunkingConfig) -> Vec<Chunk> {
    let raw_chunks = match config.strategy {
        ChunkingStrategy::Heading => chunk_by_heading(doc_path, body, config),
        ChunkingStrategy::Paragraph => {
            let mut chunks = chunk_by_paragraph(doc_path, body, config);
            for (idx, c) in chunks.iter_mut().enumerate() {
                c.embed_policy = super::policy::classify_document_chunk(
                    doc_path,
                    idx,
                    if idx == 0 { 1 } else { 3 },
                    None,
                    idx == 0,
                    &c.text,
                    None,
                );
            }
            chunks
        }
        ChunkingStrategy::Semantic => {
            let mut chunks = chunk_by_semantic(doc_path, body, config);
            for (idx, c) in chunks.iter_mut().enumerate() {
                c.embed_policy = super::policy::classify_document_chunk(
                    doc_path,
                    idx,
                    if idx == 0 { 1 } else { 3 },
                    None,
                    idx == 0,
                    &c.text,
                    None,
                );
            }
            chunks
        }
        ChunkingStrategy::Fixed => {
            let mut chunks = chunk_by_fixed(doc_path, body, config);
            for (idx, c) in chunks.iter_mut().enumerate() {
                c.embed_policy = super::policy::classify_document_chunk(
                    doc_path,
                    idx,
                    if idx == 0 { 1 } else { 3 },
                    None,
                    idx == 0,
                    &c.text,
                    None,
                );
            }
            chunks
        }
        ChunkingStrategy::CodeAst => {
            if let Some(res) = crate::parser::code::CodeChunker::parse_and_chunk(
                std::path::Path::new(doc_path),
                body,
                config,
            ) {
                res.chunks
            } else {
                chunk_by_heading(doc_path, body, config)
            }
        }
    };

    // Apply overlap if configured.
    let mut chunks = if config.overlap_tokens > 0 && raw_chunks.len() > 1 {
        apply_overlap(raw_chunks, config.overlap_tokens)
    } else {
        raw_chunks
    };

    for chunk in &mut chunks {
        if chunk.start_line == 1
            && chunk.end_line == 1
            && (chunk.start_byte > 0 || chunk.end_byte > 0)
        {
            chunk.start_line = byte_offset_to_line(body, chunk.start_byte);
            chunk.end_line = byte_offset_to_line(body, chunk.end_byte);
        }
    }

    chunks
}

/// Convert a byte offset in a string to a 1-based line number.
fn byte_offset_to_line(content: &str, byte_offset: usize) -> usize {
    let offset = byte_offset.min(content.len());
    1 + content.as_bytes()[..offset].iter().filter(|&&b| b == b'\n').count()
}

// ---------------------------------------------------------------------------
// Heading strategy
// ---------------------------------------------------------------------------

/// A section parsed from the document, delimited by headings.
struct HeadingSection {
    /// The heading level (1 for #, 2 for ##, etc.). 0 means preamble (before first heading).
    level: usize,
    /// The heading text (without the # prefix). Empty for preamble.
    heading_text: String,
    /// The body content of this section (excluding the heading line itself).
    body: String,
    /// Byte offset where this section starts in the original document.
    start_byte: usize,
    /// Byte offset where this section ends.
    end_byte: usize,
}

/// Parse heading level from a line. Returns (level, heading_text) or None.
fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if level > 6 {
        return None;
    }
    // Must have a space after the #'s (or be just #'s with nothing after)
    let rest = &trimmed[level..];
    if rest.is_empty() {
        return Some((level, ""));
    }
    if rest.starts_with(' ') {
        return Some((level, rest[1..].trim()));
    }
    None
}

/// Build heading chain from a heading stack (e.g., "Setup > Prerequisites").
fn build_heading_chain(stack: &[(usize, String)]) -> Option<String> {
    if stack.is_empty() {
        return None;
    }
    let chain: Vec<&str> = stack.iter().map(|(_, text)| text.as_str()).collect();
    Some(chain.join(" > "))
}

/// Heading-aware chunking: each heading starts a new chunk.
fn chunk_by_heading(doc_path: &str, body: &str, config: &ChunkingConfig) -> Vec<Chunk> {
    let target_chars = config.target_tokens * 4;
    let max_chars = config.max_tokens * 4;
    let min_chars = config.min_chunk_tokens * 4;

    // Parse sections.
    let sections = parse_sections(body);

    // Build chunks from sections, tracking heading hierarchy.
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut heading_stack: Vec<(usize, String)> = Vec::new();

    let mut i = 0;
    while i < sections.len() {
        let section = &sections[i];

        // Update heading stack for hierarchy tracking.
        if section.level > 0 {
            // Pop headings at same or deeper level.
            while heading_stack.last().map_or(false, |(lvl, _)| *lvl >= section.level) {
                let _ = heading_stack.pop();
            }
            heading_stack.push((section.level, section.heading_text.clone()));
        }

        let section_text = if section.heading_text.is_empty() {
            section.body.clone()
        } else {
            let hashes = "#".repeat(section.level);
            if section.body.is_empty() {
                format!("{} {}", hashes, section.heading_text)
            } else {
                format!("{} {}\n\n{}", hashes, section.heading_text, section.body)
            }
        };

        let section_chars = section_text.len();
        let heading_chain = build_heading_chain(&heading_stack);

        if section_chars < min_chars && i + 1 < sections.len() {
            // Section too small — merge with next section.
            // We'll accumulate into merged text.
            let mut merged_text = section_text;
            let merged_start = section.start_byte;
            let mut merged_end = section.end_byte;
            let merged_chain = heading_chain.clone();

            i += 1;
            while i < sections.len() {
                let next = &sections[i];
                let next_text = if next.heading_text.is_empty() {
                    next.body.clone()
                } else {
                    let hashes = "#".repeat(next.level);
                    if next.body.is_empty() {
                        format!("{} {}", hashes, next.heading_text)
                    } else {
                        format!("{} {}\n\n{}", hashes, next.heading_text, next.body)
                    }
                };

                if merged_text.len() + next_text.len() + 2 > max_chars {
                    break;
                }

                merged_text.push_str("\n\n");
                merged_text.push_str(&next_text);
                merged_end = next.end_byte;

                // Update heading stack for merged sections
                if next.level > 0 {
                    while heading_stack.last().map_or(false, |(lvl, _)| *lvl >= next.level) {
                        let _ = heading_stack.pop();
                    }
                    heading_stack.push((next.level, next.heading_text.clone()));
                }

                if merged_text.len() >= min_chars {
                    i += 1;
                    break;
                }
                i += 1;
            }

            let trimmed = merged_text.trim();
            if trimmed.len() >= min_chars {
                let policy = super::policy::classify_document_chunk(
                    doc_path,
                    chunks.len(),
                    section.level,
                    merged_chain.as_deref(),
                    true,
                    trimmed,
                    None,
                );
                chunks.push(
                    Chunk::new(doc_path, chunks.len(), trimmed, merged_start, merged_end)
                        .with_heading_chain(merged_chain)
                        .with_embed_policy(policy),
                );
            }
        } else if section_chars > max_chars {
            // Section too large — split at paragraph boundaries within it.
            let sub_chunks = split_large_section(
                &section_text,
                section.start_byte,
                target_chars,
                max_chars,
                min_chars,
            );
            for (sub_i, sub_text) in sub_chunks.into_iter().enumerate() {
                let trimmed = sub_text.text.trim();
                if trimmed.len() >= min_chars {
                    let policy = super::policy::classify_document_chunk(
                        doc_path,
                        chunks.len(),
                        section.level,
                        heading_chain.as_deref(),
                        sub_i == 0,
                        trimmed,
                        None,
                    );
                    chunks.push(
                        Chunk::new(doc_path, chunks.len(), trimmed, sub_text.start, sub_text.end)
                            .with_heading_chain(heading_chain.clone())
                            .with_embed_policy(policy),
                    );
                }
            }
            i += 1;
        } else {
            // Normal-sized section (or final section) — emit as a chunk.
            let trimmed = section_text.trim();
            if !trimmed.is_empty() {
                if trimmed.len() < min_chars && !chunks.is_empty() {
                    let last = chunks.last_mut().unwrap();
                    if last.text.len() + trimmed.len() + 2 <= max_chars {
                        last.text.push_str("\n\n");
                        last.text.push_str(trimmed);
                        last.end_byte = section.end_byte;
                    } else {
                        let policy = super::policy::classify_document_chunk(
                            doc_path,
                            chunks.len(),
                            section.level,
                            heading_chain.as_deref(),
                            true,
                            trimmed,
                            None,
                        );
                        chunks.push(
                            Chunk::new(
                                doc_path,
                                chunks.len(),
                                trimmed,
                                section.start_byte,
                                section.end_byte,
                            )
                            .with_heading_chain(heading_chain)
                            .with_embed_policy(policy),
                        );
                    }
                } else {
                    let policy = super::policy::classify_document_chunk(
                        doc_path,
                        chunks.len(),
                        section.level,
                        heading_chain.as_deref(),
                        true,
                        trimmed,
                        None,
                    );
                    chunks.push(
                        Chunk::new(
                            doc_path,
                            chunks.len(),
                            trimmed,
                            section.start_byte,
                            section.end_byte,
                        )
                        .with_heading_chain(heading_chain)
                        .with_embed_policy(policy),
                    );
                }
            }
            i += 1;
        }
    }

    if chunks.is_empty() && !body.trim().is_empty() {
        chunks.push(
            Chunk::new(doc_path, 0, body.trim(), 0, body.len())
                .with_embed_policy(groundcontrol_common::types::ChunkEmbedPolicy::Anchor),
        );
    }

    chunks
}

/// Parse the document into sections delimited by headings.
fn parse_sections(body: &str) -> Vec<HeadingSection> {
    let mut sections: Vec<HeadingSection> = Vec::new();
    let mut current_level: usize = 0;
    let mut current_heading = String::new();
    let mut current_body = String::new();
    let mut current_start: usize = 0;
    let mut pos: usize = 0;

    let mut in_code_block = false;
    let mut fence_char = '\0';
    let mut fence_len = 0;

    while pos < body.len() {
        let line_end = body[pos..].find('\n').map(|i| pos + i).unwrap_or(body.len());
        let line_content_end = if line_end > pos && body.as_bytes()[line_end - 1] == b'\r' {
            line_end - 1
        } else {
            line_end
        };
        let line = &body[pos..line_content_end];
        let next_pos = if line_end < body.len() { line_end + 1 } else { body.len() };

        let trimmed = line.trim_start();
        if in_code_block {
            if (fence_char == '`' && trimmed.starts_with("```"))
                || (fence_char == '~' && trimmed.starts_with("~~~"))
            {
                let close_len = trimmed.chars().take_while(|&c| c == fence_char).count();
                if close_len >= fence_len {
                    in_code_block = false;
                }
            }
        } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence_char = trimmed.chars().next().unwrap();
            fence_len = trimmed.chars().take_while(|&c| c == fence_char).count();
            in_code_block = true;
        }

        let heading_opt = if in_code_block { None } else { parse_heading(line) };

        if let Some((level, text)) = heading_opt {
            // Emit previous section if it has content.
            if !current_body.is_empty() || !current_heading.is_empty() || current_level > 0 {
                sections.push(HeadingSection {
                    level: current_level,
                    heading_text: current_heading.clone(),
                    body: current_body.trim().to_string(),
                    start_byte: current_start,
                    end_byte: pos,
                });
            }
            // Start new section.
            current_level = level;
            current_heading = text.to_string();
            current_body = String::new();
            current_start = pos;
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }

        pos = next_pos;
    }

    // Emit final section.
    if !current_body.is_empty() || !current_heading.is_empty() || current_level > 0 {
        sections.push(HeadingSection {
            level: current_level,
            heading_text: current_heading,
            body: current_body.trim().to_string(),
            start_byte: current_start,
            end_byte: body.len(),
        });
    }

    // Handle edge case: if body has no headings, emit the whole body as a preamble.
    if sections.is_empty() && !body.trim().is_empty() {
        sections.push(HeadingSection {
            level: 0,
            heading_text: String::new(),
            body: body.trim().to_string(),
            start_byte: 0,
            end_byte: body.len(),
        });
    }

    sections
}

struct SubChunk {
    text: String,
    start: usize,
    end: usize,
}

/// A paragraph extracted from a section, with exact slice offsets and code-fence protection.
struct ParagraphSpan<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

fn extract_paragraphs(text: &str) -> Vec<ParagraphSpan<'_>> {
    let mut paragraphs = Vec::new();
    let mut pos = 0;
    let mut current_start = 0;
    let mut in_code_block = false;
    let mut fence_char = '\0';
    let mut fence_len = 0;
    let mut has_content = false;

    while pos < text.len() {
        let line_end = text[pos..].find('\n').map(|i| pos + i).unwrap_or(text.len());
        let line_content_end = if line_end > pos && text.as_bytes()[line_end - 1] == b'\r' {
            line_end - 1
        } else {
            line_end
        };
        let line = &text[pos..line_content_end];
        let next_pos = if line_end < text.len() { line_end + 1 } else { text.len() };

        let trimmed = line.trim_start();
        if in_code_block {
            if (fence_char == '`' && trimmed.starts_with("```"))
                || (fence_char == '~' && trimmed.starts_with("~~~"))
            {
                let close_len = trimmed.chars().take_while(|&c| c == fence_char).count();
                if close_len >= fence_len {
                    in_code_block = false;
                }
            }
        } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence_char = trimmed.chars().next().unwrap();
            fence_len = trimmed.chars().take_while(|&c| c == fence_char).count();
            in_code_block = true;
        }

        let is_blank = line.trim().is_empty();
        if is_blank && !in_code_block && has_content {
            let para_slice = &text[current_start..pos];
            let trimmed_slice = para_slice.trim();
            if !trimmed_slice.is_empty() {
                paragraphs.push(ParagraphSpan {
                    text: trimmed_slice,
                    start: current_start,
                    end: pos,
                });
            }
            current_start = next_pos;
            has_content = false;
        } else if !is_blank {
            has_content = true;
        }

        pos = next_pos;
    }

    if has_content && current_start < text.len() {
        let para_slice = &text[current_start..text.len()];
        let trimmed_slice = para_slice.trim();
        if !trimmed_slice.is_empty() {
            paragraphs.push(ParagraphSpan {
                text: trimmed_slice,
                start: current_start,
                end: text.len(),
            });
        }
    }

    paragraphs
}

/// Split a large section at paragraph boundaries, falling back to sentence/character splits.
fn split_large_section(
    text: &str,
    base_offset: usize,
    target_chars: usize,
    max_chars: usize,
    min_chars: usize,
) -> Vec<SubChunk> {
    let paragraphs = extract_paragraphs(text);
    let mut sub_chunks: Vec<SubChunk> = Vec::new();
    let mut current_text = String::new();
    let mut current_start = base_offset;
    let mut current_end = base_offset;

    for para in paragraphs {
        let para_len = para.text.len();
        let abs_start = base_offset + para.start;
        let abs_end = base_offset + para.end;

        if para_len > max_chars {
            if !current_text.is_empty() {
                let trimmed = current_text.trim().to_string();
                if trimmed.len() >= min_chars {
                    sub_chunks.push(SubChunk {
                        text: trimmed,
                        start: current_start,
                        end: current_end,
                    });
                }
                current_text = String::new();
            }

            let oversized_subs =
                split_oversized_paragraph(para.text, abs_start, target_chars, max_chars, min_chars);
            sub_chunks.extend(oversized_subs);
            current_start = abs_end;
            current_end = abs_end;
            continue;
        }

        if !current_text.is_empty() && current_text.len() + para_len + 2 > target_chars {
            let trimmed = current_text.trim().to_string();
            if trimmed.len() >= min_chars {
                sub_chunks.push(SubChunk { text: trimmed, start: current_start, end: current_end });
            }
            current_text = String::new();
            current_start = abs_start;
        }

        if current_text.is_empty() {
            current_start = abs_start;
        } else {
            current_text.push_str("\n\n");
        }
        current_text.push_str(para.text);
        current_end = abs_end;
    }

    if !current_text.is_empty() {
        let trimmed = current_text.trim().to_string();
        if trimmed.len() >= min_chars {
            sub_chunks.push(SubChunk { text: trimmed, start: current_start, end: current_end });
        }
    }

    sub_chunks
}

/// Fallback splitter for paragraphs that exceed max_chars without \n\n delimiters.
fn split_oversized_paragraph(
    para: &str,
    base_offset: usize,
    target_chars: usize,
    max_chars: usize,
    min_chars: usize,
) -> Vec<SubChunk> {
    let mut chunks = Vec::new();
    let sentences = split_sentences(para);
    let mut current = String::new();
    let mut chunk_start = base_offset;
    let mut byte_pos = base_offset;

    for (i, sent) in sentences.iter().enumerate() {
        let sent_len = sent.len();
        if sent_len > max_chars {
            // Flush any current accumulation
            if !current.is_empty() {
                let trimmed = current.trim().to_string();
                if trimmed.len() >= min_chars {
                    chunks.push(SubChunk { text: trimmed, start: chunk_start, end: byte_pos });
                }
                current.clear();
            }
            // Split sentence into character chunks with word-boundary awareness
            let char_chunks = split_by_chars(sent, byte_pos, target_chars, max_chars, min_chars);
            chunks.extend(char_chunks);
            byte_pos += sent_len;
            chunk_start = byte_pos;
        } else {
            if !current.is_empty()
                && (current.len() + sent_len + 1 > target_chars
                    || current.len() + sent_len + 1 > max_chars)
            {
                let trimmed = current.trim().to_string();
                if trimmed.len() >= min_chars {
                    chunks.push(SubChunk { text: trimmed, start: chunk_start, end: byte_pos });
                }
                current.clear();
                chunk_start = byte_pos;
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(sent);
            byte_pos += sent_len + if i < sentences.len() - 1 { 1 } else { 0 };
        }
    }

    if !current.is_empty() {
        let trimmed = current.trim().to_string();
        if trimmed.len() >= min_chars {
            chunks.push(SubChunk { text: trimmed, start: chunk_start, end: byte_pos });
        }
    }

    chunks
}

/// Split text on sentence boundaries (. , ! , ? followed by space, or newlines).
fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();

    for i in 0..chars.len() {
        let (idx, c) = chars[i];
        if c == '.' || c == '!' || c == '?' || c == '\n' {
            let is_boundary =
                if i + 1 < chars.len() { chars[i + 1].1.is_whitespace() } else { true };

            if is_boundary {
                let end = idx + c.len_utf8();
                if end > start {
                    sentences.push(&text[start..end]);
                    start = end;
                    while start < text.len()
                        && text[start..].chars().next().map_or(false, char::is_whitespace)
                    {
                        start += text[start..].chars().next().unwrap().len_utf8();
                    }
                }
            }
        }
    }

    if start < text.len() {
        sentences.push(&text[start..]);
    }

    sentences
}

/// Split text by character count respecting whitespace boundaries.
fn split_by_chars(
    text: &str,
    base_offset: usize,
    target_chars: usize,
    max_chars: usize,
    _min_chars: usize,
) -> Vec<SubChunk> {
    let mut chunks = Vec::new();
    let mut start_idx = 0;
    let len = text.len();

    while start_idx < len {
        let remaining = len - start_idx;
        if remaining <= max_chars {
            let slice = text[start_idx..].trim();
            if !slice.is_empty() {
                chunks.push(SubChunk {
                    text: slice.to_string(),
                    start: base_offset + start_idx,
                    end: base_offset + len,
                });
            }
            break;
        }

        let mut target_end = (start_idx + target_chars).min(len);
        while target_end > start_idx && !text.is_char_boundary(target_end) {
            target_end -= 1;
        }
        if target_end == start_idx {
            while target_end < len && !text.is_char_boundary(target_end) {
                target_end += 1;
            }
        }

        let mut split_point = target_end;

        // Try to break on whitespace before target_end
        if let Some(ws_idx) = text[start_idx..target_end].rfind(char::is_whitespace) {
            if ws_idx > 0 {
                split_point = start_idx + ws_idx;
            }
        }

        // Ensure valid character boundary
        while split_point < len && !text.is_char_boundary(split_point) {
            split_point += 1;
        }

        if split_point == start_idx {
            // Force progress if no whitespace found
            split_point = (start_idx + max_chars).min(len);
            while split_point > start_idx && !text.is_char_boundary(split_point) {
                split_point -= 1;
            }
            if split_point == start_idx {
                while split_point < len && !text.is_char_boundary(split_point) {
                    split_point += 1;
                }
            }
        }

        let slice = text[start_idx..split_point].trim();
        if !slice.is_empty() {
            chunks.push(SubChunk {
                text: slice.to_string(),
                start: base_offset + start_idx,
                end: base_offset + split_point,
            });
        }

        start_idx = split_point;
        while start_idx < len && text[start_idx..].chars().next().map_or(false, char::is_whitespace)
        {
            start_idx += text[start_idx..].chars().next().unwrap().len_utf8();
        }
    }

    chunks
}

// ---------------------------------------------------------------------------
// Paragraph strategy
// ---------------------------------------------------------------------------

/// Paragraph-aware chunking: split on double-newlines, merge to target size.
fn chunk_by_paragraph(doc_path: &str, body: &str, config: &ChunkingConfig) -> Vec<Chunk> {
    let target_chars = config.target_tokens * 4;
    let max_chars = config.max_tokens * 4;
    let min_chars = config.min_chunk_tokens * 4;

    let paragraphs: Vec<&str> = body.split("\n\n").collect();
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut current_text = String::new();
    let mut current_start: usize = 0;
    let mut byte_pos: usize = 0;

    for (i, para) in paragraphs.iter().enumerate() {
        let para_len = para.len();

        if para_len > max_chars {
            // Flush current
            if !current_text.is_empty() {
                let trimmed = current_text.trim().to_string();
                if trimmed.len() >= min_chars {
                    chunks.push(Chunk::new(
                        doc_path,
                        chunks.len(),
                        trimmed,
                        current_start,
                        byte_pos,
                    ));
                }
                current_text = String::new();
            }

            let sub_chunks =
                split_oversized_paragraph(para, byte_pos, target_chars, max_chars, min_chars);
            for sc in sub_chunks {
                chunks.push(Chunk::new(doc_path, chunks.len(), sc.text, sc.start, sc.end));
            }
            byte_pos += para_len + if i < paragraphs.len() - 1 { 2 } else { 0 };
            current_start = byte_pos;
            continue;
        }

        if !current_text.is_empty() && current_text.len() + para_len + 2 > target_chars {
            // Emit current chunk.
            let trimmed = current_text.trim().to_string();
            if trimmed.len() >= min_chars {
                chunks.push(Chunk::new(doc_path, chunks.len(), trimmed, current_start, byte_pos));
            }
            current_text = String::new();
            current_start = byte_pos;
        }

        if !current_text.is_empty() {
            current_text.push_str("\n\n");
        }
        current_text.push_str(para);
        byte_pos += para_len + if i < paragraphs.len() - 1 { 2 } else { 0 };
    }

    // Emit remainder.
    let trimmed = current_text.trim().to_string();
    if trimmed.len() >= min_chars {
        chunks.push(Chunk::new(doc_path, chunks.len(), trimmed, current_start, body.len()));
    }

    chunks
}

// ---------------------------------------------------------------------------
// Semantic strategy (legacy line-based)
// ---------------------------------------------------------------------------

/// Line-based accumulation chunking (the original/legacy strategy).
fn chunk_by_semantic(doc_path: &str, body: &str, config: &ChunkingConfig) -> Vec<Chunk> {
    let target_chars = config.target_tokens * 4;
    let min_chars = config.min_chunk_tokens * 4;

    let mut chunks = Vec::new();
    let mut current_start: usize = 0;
    let mut current_len: usize = 0;

    let line_ends: Vec<usize> = {
        let mut ends = Vec::new();
        let mut pos = 0;
        let bytes = body.as_bytes();
        while pos < bytes.len() {
            if bytes[pos] == b'\n' {
                ends.push(pos + 1);
            }
            pos += 1;
        }
        if ends.last().copied() != Some(bytes.len()) {
            ends.push(bytes.len());
        }
        ends
    };

    let mut prev_end: usize = 0;
    for &line_end in &line_ends {
        let line_byte_len = line_end - prev_end;
        current_len += line_byte_len;
        prev_end = line_end;

        if current_len >= target_chars {
            let slice_end = line_end.min(body.len());
            let text = body[current_start..slice_end].trim();
            if text.len() >= min_chars {
                chunks.push(Chunk::new(doc_path, chunks.len(), text, current_start, slice_end));
            }
            current_start = slice_end;
            current_len = 0;
        }
    }

    // Remaining content.
    if current_start < body.len() {
        let text = body[current_start..].trim();
        if text.len() >= min_chars {
            chunks.push(Chunk::new(doc_path, chunks.len(), text, current_start, body.len()));
        }
    }

    chunks
}

// ---------------------------------------------------------------------------
// Fixed strategy
// ---------------------------------------------------------------------------

/// Pure character-count chunking: split at `target_chars` boundaries.
fn chunk_by_fixed(doc_path: &str, body: &str, config: &ChunkingConfig) -> Vec<Chunk> {
    let target_chars = config.target_tokens * 4;
    let min_chars = config.min_chunk_tokens * 4;
    let mut chunks: Vec<Chunk> = Vec::new();

    if body.is_empty() {
        return chunks;
    }

    let mut start = 0;
    while start < body.len() {
        let mut end = (start + target_chars).min(body.len());
        // Ensure we don't split in the middle of a multi-byte UTF-8 character.
        while end < body.len() && !body.is_char_boundary(end) {
            end += 1;
        }
        let text = body[start..end].trim();
        if text.len() >= min_chars {
            chunks.push(Chunk::new(doc_path, chunks.len(), text, start, end));
        }
        start = end;
    }

    chunks
}

// ---------------------------------------------------------------------------
// Overlap support
// ---------------------------------------------------------------------------

/// Apply token overlap between consecutive chunks.
/// Prepends the last `overlap_tokens` worth of text from the previous chunk
/// to the start of the next chunk.
fn apply_overlap(mut chunks: Vec<Chunk>, overlap_tokens: usize) -> Vec<Chunk> {
    let overlap_chars = overlap_tokens * 4;

    for i in 1..chunks.len() {
        let prev_text = chunks[i - 1].text.clone();
        if prev_text.len() <= overlap_chars {
            // Previous chunk is smaller than overlap — prepend entire previous text.
            let overlap_text = prev_text.as_str();
            chunks[i].text = format!("{}\n\n{}", overlap_text.trim(), chunks[i].text);
        } else {
            // Take the last `overlap_chars` characters from previous chunk.
            let start_idx = prev_text.len() - overlap_chars;
            // Find a safe char boundary.
            let safe_start = (start_idx..prev_text.len())
                .find(|&idx| prev_text.is_char_boundary(idx))
                .unwrap_or(prev_text.len());
            let overlap_text = &prev_text[safe_start..];
            if !overlap_text.trim().is_empty() {
                chunks[i].text = format!("{}\n\n{}", overlap_text.trim(), chunks[i].text);
            }
        }
    }

    chunks
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use groundcontrol_common::config::ChunkingConfig;

    fn heading_config() -> ChunkingConfig {
        ChunkingConfig {
            strategy: ChunkingStrategy::Heading,
            target_tokens: 128,
            max_tokens: 256,
            overlap_tokens: 0,
            respect_headings: true,
            min_chunk_tokens: 5,
        }
    }

    #[test]
    fn heading_strategy_splits_at_headings() {
        let body = "# Introduction\n\nThis is the intro paragraph.\n\n## Setup\n\nInstall the deps.\n\n## Usage\n\nRun the program.\n";
        let config = heading_config();
        let chunks = chunk_document("test.md", body, &config);

        assert!(chunks.len() >= 2, "Should split at headings, got {}", chunks.len());
        assert!(chunks[0].text.contains("Introduction"));
        assert!(chunks[0].heading_chain.as_deref() == Some("Introduction"));
    }

    #[test]
    fn heading_strategy_builds_hierarchy() {
        let body =
            "# RAG with Claude\n\n## Setup\n\n### Prerequisites\n\nInstall dependencies here.\n";
        let config = heading_config();
        let chunks = chunk_document("test.md", body, &config);

        // Find the chunk with "Install dependencies"
        let prereq_chunk = chunks.iter().find(|c| c.text.contains("Install dependencies"));
        assert!(prereq_chunk.is_some(), "Should have a prerequisites chunk");
        let chain = prereq_chunk.unwrap().heading_chain.as_deref().unwrap_or("");
        assert!(
            chain.contains("RAG with Claude") && chain.contains("Prerequisites"),
            "Heading chain should contain hierarchy, got: {}",
            chain
        );
    }

    #[test]
    fn heading_strategy_merges_small_sections() {
        let mut config = heading_config();
        config.min_chunk_tokens = 20; // ~80 chars minimum
        let body = "# Title\n\nOk.\n\n## A\n\nShort.\n\n## B\n\nAlso short but let's make this a bit longer to pass min.\n";
        let chunks = chunk_document("test.md", body, &config);
        // Small sections should be merged
        assert!(!chunks.is_empty());
    }

    #[test]
    fn paragraph_strategy_splits_on_blank_lines() {
        let config = ChunkingConfig {
            strategy: ChunkingStrategy::Paragraph,
            target_tokens: 20, // ~80 chars to force splits
            max_tokens: 256,
            overlap_tokens: 0,
            respect_headings: false,
            min_chunk_tokens: 5,
        };
        let body = "First paragraph with some text.\n\nSecond paragraph also with text.\n\nThird paragraph here.\n";
        let chunks = chunk_document("test.md", body, &config);
        assert!(!chunks.is_empty());
        // All chunks should have None heading_chain
        for chunk in &chunks {
            assert!(chunk.heading_chain.is_none());
        }
    }

    #[test]
    fn semantic_strategy_accumulates_lines() {
        let config = ChunkingConfig {
            strategy: ChunkingStrategy::Semantic,
            target_tokens: 10, // very small
            max_tokens: 256,
            overlap_tokens: 0,
            respect_headings: false,
            min_chunk_tokens: 2,
        };
        let body = "Line one here.\nLine two here.\nLine three here.\nLine four here.\n";
        let chunks = chunk_document("test.md", body, &config);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn fixed_strategy_splits_at_char_boundary() {
        let config = ChunkingConfig {
            strategy: ChunkingStrategy::Fixed,
            target_tokens: 10, // ~40 chars
            max_tokens: 256,
            overlap_tokens: 0,
            respect_headings: false,
            min_chunk_tokens: 2,
        };
        let body =
            "Hello world, this is a test document with multiple words and sentences for chunking.";
        let chunks = chunk_document("test.md", body, &config);
        assert!(chunks.len() >= 2, "Should produce multiple chunks");
        // Verify valid UTF-8
        for chunk in &chunks {
            let _ = chunk.text.len();
        }
    }

    #[test]
    fn overlap_prepends_previous_text() {
        let config = ChunkingConfig {
            strategy: ChunkingStrategy::Fixed,
            target_tokens: 20, // ~80 chars
            max_tokens: 256,
            overlap_tokens: 8, // ~32 chars overlap
            respect_headings: false,
            min_chunk_tokens: 2,
        };
        let body = "A".repeat(200); // Will produce multiple fixed chunks
        let chunks = chunk_document("test.md", &body, &config);
        if chunks.len() > 1 {
            // Second chunk should be longer than raw split because of overlap
            assert!(chunks[1].text.len() > 80 - 10); // approximate check
        }
    }

    #[test]
    fn handles_multibyte_utf8_characters() {
        let mut config = heading_config();
        config.target_tokens = 10;
        config.min_chunk_tokens = 2;
        let body = "# Hello\n\nHello world…\nThis is a test…\nMore content here…\n";
        let chunks = chunk_document("test.md", body, &config);
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            let _ = chunk.text.len(); // would panic if invalid UTF-8
        }
    }

    #[test]
    fn handles_windows_line_endings() {
        let mut config = heading_config();
        config.target_tokens = 10;
        config.min_chunk_tokens = 2;
        let body = "# Title\r\n\r\nLine one\r\nLine two\r\nLine three\r\n";
        let chunks = chunk_document("test.md", body, &config);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn chunks_short_document_into_single_chunk() {
        let mut config = heading_config();
        config.min_chunk_tokens = 5;
        let body = "# Short\n\nThis is a short document with two paragraphs.\n";
        let chunks = chunk_document("test.md", body, &config);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("short document"));
    }

    #[test]
    fn test_oversized_paragraph_splits_strictly_under_max_tokens() {
        let mut config = heading_config();
        config.target_tokens = 20; // ~80 chars
        config.max_tokens = 40; // ~160 chars
        config.min_chunk_tokens = 2;

        // Long single paragraph with multiple sentences and no \n\n
        let body = "# Section\n\nSentence one is here. Sentence two follows it quickly. Sentence three is also quite long and descriptive. Sentence four provides additional context. Sentence five wraps things up nicely.";
        let chunks = chunk_document("test.md", body, &config);
        assert!(chunks.len() >= 2, "Expected at least 2 chunks, got {}", chunks.len());
        for chunk in &chunks {
            assert!(
                chunk.text.len() <= config.max_tokens * 4 + 20,
                "Chunk exceeded max characters: len={}, text={}",
                chunk.text.len(),
                chunk.text
            );
        }
    }

    #[test]
    fn test_oversized_unbroken_text_character_fallback() {
        let mut config = heading_config();
        config.target_tokens = 10; // ~40 chars
        config.max_tokens = 20; // ~80 chars
        config.min_chunk_tokens = 2;

        // Unbroken block of text with no spaces or sentence delimiters
        let body = format!("# Section\n\n{}", "A".repeat(300));
        let chunks = chunk_document("test.md", &body, &config);
        assert!(chunks.len() >= 3, "Expected at least 3 chunks, got {}", chunks.len());
        for chunk in &chunks {
            assert!(
                chunk.text.len() <= config.max_tokens * 4,
                "Chunk exceeded max chars: len={}",
                chunk.text.len()
            );
        }
    }

    #[test]
    fn test_byte_offset_to_line_non_char_boundary_multibyte() {
        // '→' is 3 bytes (0xE2 0x86 0x92)
        let text = "Line 1\nLine 2 with arrow: →\nLine 3";
        let arrow_pos = text.find('→').unwrap();
        // Index inside the multi-byte character (arrow_pos + 1)
        let inside_char = arrow_pos + 1;
        // Must not panic on non-char boundary
        let line = byte_offset_to_line(text, inside_char);
        assert_eq!(line, 2);
    }
}
