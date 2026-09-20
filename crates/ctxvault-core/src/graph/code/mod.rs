//! Polyglot Code Graph Extractor & Lightweight Symbol/Import Resolver.
//!
//! Extracts structural AST relationships (`defines`, `imports`, `calls`, `implements_trait`)
//! across polyglot source code files and resolves cross-file call sites using SQLite
//! symbol catalogs and Petgraph.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use ctxvault_common::types::{
    CodeSymbol, Edge, EdgeProvenance, ExternalRef, ExternalRefKind, ResolutionConfidence,
};
use tree_sitter::Parser;

use crate::parser::code::languages::detect_language;

pub(crate) mod visitor;
use visitor::CallAndImportVisitor;

#[cfg(test)]
mod tests;

/// Extracted structural code relationship.
#[derive(Debug, Clone)]
pub struct ExtractedCodeEdge {
    /// Source node path (file path or symbol scope path).
    pub source: String,
    /// Target node path (file path or symbol scope path).
    pub target: String,
    /// Edge relationship type (e.g. "defines", "imports", "calls", "implements_trait").
    pub edge_type: String,
    /// Edge weight (0.0 - 1.0).
    pub weight: f32,
    /// Edge provenance.
    pub provenance: EdgeProvenance,
}

/// Result of extracting structural relationships from a single code file.
///
/// Carries the intra-repo structural edges (unchanged from prior behavior) plus
/// the [`ExternalRef`]s captured for call/import targets that did not resolve to
/// a local symbol. [`CodeExtraction::edges`] is byte-for-byte identical to the
/// edge set this extractor produced before external-reference capture was added;
/// external references are surfaced only via the separate
/// [`CodeExtraction::external_refs`] channel for a later cross-corpus
/// reconciliation pass and never alter the emitted `edges`.
#[derive(Debug, Clone, Default)]
pub struct CodeExtraction {
    /// Structural code edges (`defines`, `imports`, `calls`, `implements`, ...).
    pub edges: Vec<Edge>,
    /// Unresolved call/import targets ([`ExternalRefKind::Call`] /
    /// [`ExternalRefKind::Import`]) captured for later cross-corpus resolution.
    pub external_refs: Vec<ExternalRef>,
}

/// Polyglot code graph extractor.
pub struct CodeGraphExtractor;

impl CodeGraphExtractor {
    /// Build a symbol lookup index from a slice of code symbols.
    pub fn build_symbol_index<'a>(
        symbols: &'a [CodeSymbol],
    ) -> HashMap<String, Vec<&'a CodeSymbol>> {
        let mut symbol_index: HashMap<String, Vec<&CodeSymbol>> =
            HashMap::with_capacity(symbols.len());
        for sym in symbols {
            symbol_index.entry(sym.name.clone()).or_default().push(sym);
        }
        symbol_index
    }

    /// Extract all structural edges (defines, imports, calls, implements) for a single code file
    /// using a pre-computed symbol index, alongside any unresolved external references.
    ///
    /// The returned [`CodeExtraction::edges`] is byte-for-byte identical to the edge set
    /// this extractor produced before external-reference capture was added; external references
    /// are surfaced only via the separate [`CodeExtraction::external_refs`] channel.
    pub fn extract_edges_for_file_with_index(
        file_path: &Path,
        content: &str,
        file_symbols: &[CodeSymbol],
        symbol_index: &HashMap<String, Vec<&CodeSymbol>>,
    ) -> CodeExtraction {
        let mut edges = Vec::new();
        let file_path_str = file_path.to_string_lossy().replace('\\', "/");

        // 1. "defines" edges: File -> Symbol
        for sym in file_symbols {
            edges.push(Edge {
                source: file_path_str.clone(),
                target: sym.scope_path.clone(),
                edge_type: "defines".to_string(),
                weight: 1.0,
                provenance: EdgeProvenance::CodeDefines,
                target_corpus: None,
                confidence: Some(ResolutionConfidence::High),
                target_path: None,
                target_symbol: None,
                target_kind: None,
            });
        }

        // 2. Parse AST for imports and call sites
        if content.len() > crate::parser::code::chunker::CodeChunker::MAX_CODE_FILE_SIZE_BYTES {
            return CodeExtraction { edges, external_refs: Vec::new() };
        }

        let Some(lang) = detect_language(file_path) else {
            return CodeExtraction { edges, external_refs: Vec::new() };
        };

        let mut parser = Parser::new();
        if parser.set_language(&lang.tree_sitter_language()).is_err() {
            return CodeExtraction { edges, external_refs: Vec::new() };
        }

        let Some(tree) = parser.parse(content, None) else {
            return CodeExtraction { edges, external_refs: Vec::new() };
        };

        let mut visitor =
            CallAndImportVisitor::new(file_path_str, content, lang, file_symbols, symbol_index);
        visitor.visit(tree.root_node());

        let mut external_refs = visitor.external_refs;
        let mut seen_imports = HashSet::new();
        // Import edges always target an out-of-corpus module path (they never resolve to an
        // in-corpus symbol), so each is an external reference. Derive them from the produced
        // edges so the edge Vec itself is left untouched.
        for e in &visitor.edges {
            if e.provenance == EdgeProvenance::CodeImports
                && seen_imports.insert((e.source.clone(), e.target.clone()))
            {
                external_refs.push(ExternalRef {
                    caller_scope_path: e.source.clone(),
                    raw_target: e.target.clone(),
                    kind: ExternalRefKind::Import,
                    confidence: ResolutionConfidence::Speculative,
                });
            }
        }

        edges.extend(visitor.edges);
        CodeExtraction { edges, external_refs }
    }

    /// Extract all structural edges (defines, imports, calls, implements) for a single code file.
    ///
    /// Returns only the intra-repo edge set; external-reference capture is a concern of the
    /// corpus-wide second pass, which calls [`Self::extract_edges_for_file_with_index`] directly.
    pub fn extract_edges_for_file(
        file_path: &Path,
        content: &str,
        file_symbols: &[CodeSymbol],
        all_symbols: &[CodeSymbol],
    ) -> Vec<Edge> {
        let symbol_index = Self::build_symbol_index(all_symbols);
        Self::extract_edges_for_file_with_index(file_path, content, file_symbols, &symbol_index)
            .edges
    }
}
