//! Artifact parsing, lossy file reading, and chunk to pending conversion.

use std::fs;
use std::path::Path;

use groundcontrol_common::config::{ChunkingConfig, IndexMode};
use groundcontrol_common::types::{Chunk, ParsedArtifact};
use groundcontrol_common::Result;

use crate::classifier::FileClassifier;
use crate::engine::types::PendingChunk;

/// Convert a domain chunk into a pending chunk for vector embedding.
pub(crate) fn chunk_to_pending(
    chunk: &Chunk,
    doc_path: &str,
    doc_title: &str,
    is_code: bool,
) -> PendingChunk {
    let section = chunk.heading_chain.as_deref().unwrap_or("").trim();
    let text = if !doc_title.is_empty() && !section.is_empty() {
        format!("{} > {}: {}", doc_title, section, chunk.text)
    } else if !doc_title.is_empty() {
        format!("{}: {}", doc_title, chunk.text)
    } else if !section.is_empty() {
        format!("{}: {}", section, chunk.text)
    } else {
        chunk.text.clone()
    };
    let modality = chunk
        .entity_kind
        .as_ref()
        .map(groundcontrol_common::types::EntityKind::modality_tag)
        .unwrap_or_else(|| if is_code { "code" } else { "docs" })
        .to_string();
    PendingChunk {
        doc_path: doc_path.to_string(),
        chunk_index: chunk.chunk_index,
        text,
        embed_policy: chunk.embed_policy,
        modality,
    }
}

impl crate::engine::state::Engine {
    /// Helper to read file bytes and decode as UTF-8 lossily so non-UTF8 characters never throw.
    pub(crate) fn read_file_lossy(path: &Path) -> std::io::Result<String> {
        let bytes = fs::read(path)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Parse a single file (polyglot code or markdown) in a thread-safe, lock-free manner.
pub(crate) fn parse_file_record(
    rel_path: &str,
    full_path: &Path,
    bytes: &[u8],
    hash: String,
    classifier: &FileClassifier,
    chunking_config: &ChunkingConfig,
    _index_mode: IndexMode,
) -> Result<ParsedArtifact> {
    crate::parser::ArtifactParser::parse(
        rel_path,
        full_path,
        bytes,
        hash,
        classifier,
        chunking_config,
    )
}
