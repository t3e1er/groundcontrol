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
            FileClassification::GenericText => {
                Self::parse_generic_text(rel_path, bytes, hash, chunking_config)
            }
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

    fn parse_generic_text(
        rel_path: &str,
        bytes: &[u8],
        hash: String,
        _chunking_config: &ChunkingConfig,
    ) -> Result<ParsedArtifact> {
        let content = String::from_utf8_lossy(bytes).into_owned();
        let path = Path::new(rel_path);
        let file_title = path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string());

        let lines: Vec<&str> = content.lines().collect();
        let mut raw_chunks = Vec::new();
        let window_size = 100;
        let overlap = 10;
        let mut start = 0;
        let mut chunk_idx = 0;

        if lines.is_empty() {
            raw_chunks.push(
                groundcontrol_common::types::Chunk::new(rel_path, 0, "", 0, 0).with_lines(1, 1),
            );
        } else {
            let mut byte_offsets = Vec::with_capacity(lines.len() + 1);
            let mut curr_offset = 0;
            for line in &lines {
                byte_offsets.push(curr_offset);
                curr_offset += line.len() + 1; // +1 for newline
            }
            byte_offsets.push(content.len());

            while start < lines.len() {
                let end = (start + window_size).min(lines.len());
                let chunk_lines = &lines[start..end];
                let chunk_content = chunk_lines.join("\n");
                let start_byte = byte_offsets[start];
                let end_byte = byte_offsets[end].min(content.len());

                raw_chunks.push(
                    groundcontrol_common::types::Chunk::new(
                        rel_path,
                        chunk_idx,
                        chunk_content,
                        start_byte,
                        end_byte,
                    )
                    .with_lines(start + 1, end),
                );

                chunk_idx += 1;
                if end == lines.len() {
                    break;
                }
                start += window_size.saturating_sub(overlap);
            }
        }

        Ok(ParsedArtifact {
            path: rel_path.to_string(),
            hash,
            is_code: true,
            format: FileFormat::Source,
            title: file_title,
            doc_metadata: None,
            symbols: Vec::new(),
            grammar_semantics: Vec::new(),
            chunks: raw_chunks,
            graph_edges: Vec::new(),
            external_refs: Vec::new(),
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
        let doc = crate::parser::document::markdown::parse_document(path, &content)?;
        let chunks = crate::parser::document::markdown::chunk_markdown_document(
            rel_path,
            &content,
            &doc,
            chunking_config,
        );

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
        let chunks = crate::parser::document::chunker::chunk_document(
            rel_path,
            &extracted.normalized_text,
            chunking_config,
        );

        let doc = Document {
            path: rel_path.to_string(),
            frontmatter: None,
            title: extracted.title.clone(),
            tags: Vec::new(),
            links: extracted.outbound_links.clone(),
            template: None,
            content: extracted.normalized_text.clone(),
            content_hash: hash.clone(),
            body_offset: 0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use groundcontrol_common::config::CorpusConfig;
    use tempfile::TempDir;

    #[test]
    fn test_parse_generic_text_fallback() {
        let tmp = TempDir::new().unwrap();
        let config = CorpusConfig::default();
        let classifier = FileClassifier::new(tmp.path(), &config);
        let chunking_config = ChunkingConfig::default();

        let pascal_code = "program HelloWorld;\nbegin\n  writeln('Hello, Pascal!');\nend.\n";
        let rel_path = "legacy/hello.pas";
        let full_path = tmp.path().join("legacy/hello.pas");

        let artifact = ArtifactParser::parse(
            rel_path,
            &full_path,
            pascal_code.as_bytes(),
            "hash123".to_string(),
            &classifier,
            &chunking_config,
        )
        .expect("Generic text should parse successfully");

        assert!(artifact.is_code);
        assert_eq!(artifact.format, FileFormat::Source);
        assert!(!artifact.chunks.is_empty());
        assert_eq!(artifact.chunks[0].start_line, 1);
        assert!(artifact.chunks[0].text.contains("Hello, Pascal!"));
    }

    #[test]
    fn test_parse_new_polyglot_languages() {
        let tmp = TempDir::new().unwrap();
        let config = CorpusConfig::default();
        let classifier = FileClassifier::new(tmp.path(), &config);
        let chunking_config = ChunkingConfig::default();

        // 1. XML
        let xml_src = r#"<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:template name="test_template">
    <xsl:value-of select="'hello'"/>
  </xsl:template>
</xsl:stylesheet>"#;
        let xml_art = ArtifactParser::parse(
            "templates/sheet.xsl",
            &tmp.path().join("templates/sheet.xsl"),
            xml_src.as_bytes(),
            "h_xml".to_string(),
            &classifier,
            &chunking_config,
        )
        .expect("XML should parse successfully");
        assert!(xml_art.is_code);
        assert!(!xml_art.chunks.is_empty());

        // 2. VB6
        let vb6_src = "Sub CalculateTotal()\n    Dim x As Integer\nEnd Sub\n";
        let vb6_art = ArtifactParser::parse(
            "forms/frmMain.frm",
            &tmp.path().join("forms/frmMain.frm"),
            vb6_src.as_bytes(),
            "h_vb6".to_string(),
            &classifier,
            &chunking_config,
        )
        .expect("VB6 should parse successfully");
        assert!(vb6_art.is_code);
        assert!(!vb6_art.chunks.is_empty());

        // 3. PL/SQL
        let plsql_src = "CREATE PACKAGE BODY finance IS\nPROCEDURE update_balance IS\nBEGIN\n  NULL;\nEND update_balance;\nEND finance;\n";
        let plsql_art = ArtifactParser::parse(
            "db/update_bal.pkb",
            &tmp.path().join("db/update_bal.pkb"),
            plsql_src.as_bytes(),
            "h_pls".to_string(),
            &classifier,
            &chunking_config,
        )
        .expect("PL/SQL should parse successfully");
        assert!(plsql_art.is_code);
        assert!(!plsql_art.chunks.is_empty());

        // 4. COBOL
        let cobol_src = "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. HELLO.\n       PROCEDURE DIVISION.\n           DISPLAY 'HELLO'.\n           STOP RUN.\n";
        let cobol_art = ArtifactParser::parse(
            "src/hello.cbl",
            &tmp.path().join("src/hello.cbl"),
            cobol_src.as_bytes(),
            "h_cob".to_string(),
            &classifier,
            &chunking_config,
        )
        .expect("COBOL should parse successfully");
        assert!(cobol_art.is_code);
        assert!(!cobol_art.chunks.is_empty());
    }
}
