//! Chunk split suggestions: heuristic analysis of long or multi-topic chunks.

use std::path::Path;

use groundcontrol_common::Result;
use serde::{Deserialize, Serialize};

/// A chunk that may benefit from being split into smaller pieces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitSuggestion {
    /// Document path.
    pub path: String,
    /// Chunk index.
    pub chunk_index: usize,
    /// Length of the chunk in characters.
    pub char_count: usize,
    /// Reason for suggesting a split.
    pub reason: String,
    /// Coherence score (lower = less coherent, more likely to benefit from split).
    pub coherence_score: f64,
}

/// Suggest chunks that might benefit from splitting.
///
/// Heuristics:
/// - Chunks that are very long (near max token limit)
/// - Chunks with multiple heading-level transitions
/// - Chunks with high topic diversity (measured by distinct section keywords)
///
/// This is a heuristic-based approach that doesn't require embeddings.
pub fn suggest_splits(
    store: &crate::persistence::Store,
    corpus_root: Option<&Path>,
    max_chunk_chars: usize,
) -> Result<Vec<SplitSuggestion>> {
    let files = store.list_files()?;
    let mut suggestions = Vec::new();

    for file in &files {
        let chunks = store.get_chunks_for_file(&file.path)?;

        for chunk in &chunks {
            let text_opt = corpus_root.and_then(|root| {
                let full = root.join(&file.path);
                let mut f = std::fs::File::open(&full).ok()?;
                use std::io::{Read, Seek, SeekFrom};
                f.seek(SeekFrom::Start(chunk.start_byte as u64)).ok()?;
                let len = chunk.end_byte.saturating_sub(chunk.start_byte);
                let mut buf = vec![0u8; len];
                f.read_exact(&mut buf).ok()?;
                String::from_utf8(buf).ok()
            });

            let char_count = text_opt
                .as_ref()
                .map(|t| t.len())
                .unwrap_or_else(|| chunk.end_byte.saturating_sub(chunk.start_byte));

            // Heuristic 1: Very long chunks.
            if char_count > max_chunk_chars {
                suggestions.push(SplitSuggestion {
                    path: file.path.clone(),
                    chunk_index: chunk.chunk_index,
                    char_count,
                    reason: format!(
                        "chunk exceeds {} characters ({} chars)",
                        max_chunk_chars, char_count
                    ),
                    coherence_score: 0.3,
                });
                continue;
            }

            if let Some(ref text) = text_opt {
                // Heuristic 2: Multiple headings within a single chunk.
                let heading_count = text.lines().filter(|line| line.starts_with('#')).count();
                if heading_count > 1 {
                    let coherence = 1.0 / (heading_count as f64);
                    suggestions.push(SplitSuggestion {
                        path: file.path.clone(),
                        chunk_index: chunk.chunk_index,
                        char_count,
                        reason: format!("chunk contains {} headings", heading_count),
                        coherence_score: coherence,
                    });
                    continue;
                }

                // Heuristic 3: Very diverse content (many paragraph breaks relative to size).
                let paragraph_count = text.split("\n\n").count();
                if paragraph_count > 5 && char_count > 500 {
                    let coherence = 1.0 - (paragraph_count as f64 / 10.0).min(0.8);
                    suggestions.push(SplitSuggestion {
                        path: file.path.clone(),
                        chunk_index: chunk.chunk_index,
                        char_count,
                        reason: format!(
                            "chunk has {} paragraphs (high topic diversity)",
                            paragraph_count
                        ),
                        coherence_score: coherence,
                    });
                }
            }
        }
    }

    // Sort by coherence score ascending (least coherent first).
    suggestions.sort_by(|a, b| {
        a.coherence_score.partial_cmp(&b.coherence_score).unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(suggestions)
}
