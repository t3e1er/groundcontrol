//! Model migration and vector re-embedding.

use std::fs;
use std::path::Path;

use tracing::info;

use groundcontrol_common::{Error, Result};

use crate::engine::state::Engine;
use crate::engine::types::PendingChunk;

impl Engine {
    /// Read the exact UTF-8 text slice of a chunk directly from the source file on disk.
    pub fn fetch_chunk_text(
        &self,
        rel_path: &str,
        start_byte: usize,
        end_byte: usize,
    ) -> Result<String> {
        let proj_path = self.projection_path(rel_path);
        let target_path = if proj_path.is_file() {
            proj_path
        } else {
            Path::new(&self.config.path).join(rel_path)
        };

        let mut file = fs::File::open(&target_path).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("failed to open '{}': {e}", target_path.display()),
            ))
        })?;

        use std::io::{Read, Seek, SeekFrom};
        file.seek(SeekFrom::Start(start_byte as u64))?;
        let len = end_byte.saturating_sub(start_byte);
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    /// Re-embed all chunks with the current model, replacing old vectors.
    pub fn reembed(&mut self) -> Result<usize> {
        if self.is_fast_mode() || self.dense.is_none() {
            return Err(Error::Index(
                "re-embedding is unavailable in fast mode. Re-index with index_mode = 'full'"
                    .to_string(),
            ));
        }

        let available = self.ensure_embedder()?;
        if !available {
            return Err(Error::Index("embedder not available — cannot re-embed".to_string()));
        }
        let embedder = self.embedder_arc().unwrap();

        let files = self.store.list_files()?;
        if let Some(ref mut dense) = self.dense {
            dense.clear()?;
        }

        let mut total_chunks = 0usize;
        let mut chunk_buffer: Vec<PendingChunk> = Vec::new();

        let corpus_path = std::path::PathBuf::from(&self.config.path);
        for file in &files {
            let is_code = crate::parser::code::is_code_file(Path::new(&file.path));
            if is_code {
                continue;
            }

            let full_path = corpus_path.join(&file.path);
            let parsed_chunks: Option<(Vec<groundcontrol_common::types::Chunk>, Option<String>)> =
                fs::read_to_string(&full_path).ok().and_then(|content| {
                    let doc =
                        crate::parser::parse_document(Path::new(&file.path), &content).ok()?;
                    let chunks = crate::parser::chunker::chunk_document(
                        &file.path,
                        &doc.content,
                        &self.config.chunking,
                    );
                    Some((chunks, doc.title))
                });

            let (chunks, title) = if let Some((c, t)) = parsed_chunks {
                (c, t.or_else(|| file.title.clone()))
            } else {
                let chunk_records = self.store.get_chunks_for_file(&file.path)?;
                let mut chunks: Vec<groundcontrol_common::types::Chunk> =
                    Vec::with_capacity(chunk_records.len());
                for cr in chunk_records {
                    let text = self
                        .fetch_chunk_text(&file.path, cr.start_byte, cr.end_byte)
                        .unwrap_or_default();
                    chunks.push(
                        groundcontrol_common::types::Chunk::new(
                            file.path.clone(),
                            cr.chunk_index,
                            text,
                            cr.start_byte,
                            cr.end_byte,
                        )
                        .with_lines(cr.start_line, cr.end_line),
                    );
                }
                (chunks, file.title.clone())
            };

            if chunks.is_empty() {
                continue;
            }

            let doc_title = title.as_deref().unwrap_or("").trim();
            for c in &chunks {
                let section = c.heading_chain.as_deref().unwrap_or("").trim();
                let text = if !doc_title.is_empty() && !section.is_empty() {
                    format!("{} > {}: {}", doc_title, section, c.text)
                } else if !doc_title.is_empty() {
                    format!("{}: {}", doc_title, c.text)
                } else if !section.is_empty() {
                    format!("{}: {}", section, c.text)
                } else {
                    c.text.clone()
                };
                let modality = c
                    .entity_kind
                    .as_ref()
                    .map(groundcontrol_common::types::EntityKind::modality_tag)
                    .unwrap_or("docs")
                    .to_string();
                chunk_buffer.push(PendingChunk {
                    doc_path: file.path.clone(),
                    chunk_index: c.chunk_index,
                    text,
                    embed_policy: c.embed_policy,
                    modality,
                });
            }

            if chunk_buffer.len() >= 64 {
                self.flush_chunk_buffer(&chunk_buffer)?;
                chunk_buffer.clear();
            }

            total_chunks += chunks.len();
        }

        if !chunk_buffer.is_empty() {
            self.flush_chunk_buffer(&chunk_buffer)?;
            chunk_buffer.clear();
        }

        let model_version = embedder.model_name().version_string().to_string();
        if let Some(ref mut vi) = self.vector_index_mut() {
            vi.set_model_version(&model_version);
            vi.clear_stale();
        }

        self.store.set_config("embedding_model", &model_version)?;
        self.commit()?;

        info!(
            "Re-embedding complete: {} chunks re-embedded with model '{}'",
            total_chunks, model_version
        );

        Ok(total_chunks)
    }
}
