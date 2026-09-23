//! Unified artifact parsing facade.
//!
//! Provides a thread-safe, pure-CPU parsing boundary that converts raw source files,
//! Markdown documentation, and container documents (.docx, .pdf, .html) into
//! structured [`ParsedArtifact`] intermediate representations.

use std::path::Path;

use groundcontrol_common::config::ChunkingConfig;
use groundcontrol_common::types::{Document, Edge, EdgeProvenance, FileFormat, ParsedArtifact};
use groundcontrol_common::Result;

use crate::graph::code::CodeGraphExtractor;
use crate::index::classifier::{FileClassification, FileClassifier};
use crate::parser::code::chunker::CodeChunker;
use crate::parser::document::DocumentExtractorRegistry;

/// Unified parser facade for polyglot source code and rich documentation.
pub struct ArtifactParser;

impl ArtifactParser {
    /// Parse a single file (polyglot code, markdown, or container document) into a [`ParsedArtifact`].
    pub fn parse(
        rel_path: &str,
        full_path: &Path,
        bytes: &[u8],
        hash: String,
        classifier: &FileClassifier,
        chunking_config: &ChunkingConfig,
    ) -> Result<ParsedArtifact> {
        let classification = classifier.classify(full_path, Some(bytes));

        match classification {
            FileClassification::Code(_) => Self::parse_code(rel_path, bytes, hash, chunking_config),
            FileClassification::MarkdownDoc => {
                Self::parse_markdown(rel_path, bytes, hash, chunking_config)
            }
            FileClassification::Document(fmt) => {
                Self::parse_rich_document(rel_path, full_path, bytes, hash, fmt, chunking_config)
            }
            FileClassification::Ignored => Err(groundcontrol_common::Error::Parse {
                path: rel_path.to_string(),
                message: format!("file '{rel_path}' is ignored or unsupported format"),
            }),
        }
    }

    fn parse_code(
        rel_path: &str,
        bytes: &[u8],
        hash: String,
        chunking_config: &ChunkingConfig,
    ) -> Result<ParsedArtifact> {
        let content = String::from_utf8_lossy(bytes).into_owned();
        let path = Path::new(rel_path);
        let file_title = path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string());
        let parse_res = CodeChunker::parse_and_chunk(path, &content, chunking_config);

        let mut raw_chunks = Vec::new();
        let mut symbols = Vec::new();
        let mut grammar_semantics = Vec::new();
        let mut graph_edges = Vec::new();
        let mut external_refs = Vec::new();

        if let Some(res) = parse_res {
            let symbol_index = CodeGraphExtractor::build_symbol_index(&res.symbols);
            let extraction = CodeGraphExtractor::extract_edges_for_file_with_index(
                path,
                &content,
                &res.symbols,
                &symbol_index,
            );
            graph_edges = extraction.edges;
            external_refs = extraction.external_refs;

            raw_chunks = res.chunks;
            symbols = res.symbols;
            grammar_semantics = res.grammar_semantics;
        }

        Ok(ParsedArtifact {
            path: rel_path.to_string(),
            hash,
            is_code: true,
            format: FileFormat::Source,
            title: file_title,
            doc_metadata: None,
            symbols,
            grammar_semantics,
            chunks: raw_chunks,
            graph_edges,
            external_refs,
            raw_content: Some(content),
            projection_text: None,
        })
    }

    fn parse_markdown(
        rel_path: &str,
        bytes: &[u8],
        hash: String,
        chunking_config: &ChunkingConfig,
    ) -> Result<ParsedArtifact> {
        let content = String::from_utf8_lossy(bytes).into_owned();
        let path = Path::new(rel_path);
        let doc = crate::parser::parse_document(path, &content)?;
        let chunks =
            crate::parser::chunker::chunk_document(rel_path, &doc.content, chunking_config);

        let title = doc.title.clone();
        Ok(ParsedArtifact {
            path: rel_path.to_string(),
            hash,
            is_code: false,
            format: FileFormat::Source,
            title,
            doc_metadata: Some(doc),
            symbols: Vec::new(),
            grammar_semantics: Vec::new(),
            chunks,
            graph_edges: Vec::new(),
            external_refs: Vec::new(),
            raw_content: Some(content),
            projection_text: None,
        })
    }

    fn parse_rich_document(
        rel_path: &str,
        full_path: &Path,
        bytes: &[u8],
        hash: String,
        fmt: FileFormat,
        chunking_config: &ChunkingConfig,
    ) -> Result<ParsedArtifact> {
        let registry = DocumentExtractorRegistry::new();
        let extracted = registry.extract(full_path, fmt, bytes)?;
        let chunks = crate::parser::chunker::chunk_document(
            rel_path,
            &extracted.normalized_text,
            chunking_config,
        );

        let doc = Document {
            path: rel_path.to_string(),
            frontmatter: None,
            title: extracted.title.clone(),
            tags: Vec::new(),
            wikilinks: Vec::new(),
            template: None,
            content: extracted.normalized_text.clone(),
            content_hash: hash.clone(),
        };

        let mut graph_edges = Vec::new();
        for link in extracted.outbound_links {
            graph_edges.push(Edge {
                source: rel_path.to_string(),
                target: link.target,
                edge_type: "references".to_string(),
                weight: 0.8,
                provenance: EdgeProvenance::MarkdownLink,
                target_corpus: None,
                confidence: Some(groundcontrol_common::types::ResolutionConfidence::High),
                target_path: None,
                target_symbol: None,
                target_kind: None,
            });
        }

        let title = extracted.title;
        Ok(ParsedArtifact {
            path: rel_path.to_string(),
            hash,
            is_code: false,
            format: fmt,
            title,
            doc_metadata: Some(doc),
            symbols: Vec::new(),
            grammar_semantics: Vec::new(),
            chunks,
            graph_edges,
            external_refs: Vec::new(),
            raw_content: Some(extracted.normalized_text.clone()),
            projection_text: Some(extracted.normalized_text),
        })
    }
}
