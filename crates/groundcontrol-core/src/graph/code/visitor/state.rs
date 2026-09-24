//! AST Visitor state, context, and resolution helpers.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use groundcontrol_common::types::{
    CodeSymbol, Edge, EdgeProvenance, ExternalRef, ExternalRefKind, ResolutionConfidence,
};
use tree_sitter::Node;

use crate::graph::hybrid_lsp::TypeEnvironment;
use crate::parser::code::languages::SupportedLanguage;

pub(crate) struct CallAndImportVisitor<'a> {
    pub(super) file_path: String,
    pub(super) content: &'a str,
    pub(super) language: SupportedLanguage,
    pub(super) file_symbols: &'a [CodeSymbol],
    pub(super) symbol_index: &'a HashMap<String, Vec<&'a CodeSymbol>>,
    pub(super) current_caller: Option<String>,
    pub(super) current_container: Option<String>,
    pub(crate) edges: Vec<Edge>,
    pub(crate) external_refs: Vec<ExternalRef>,
    pub(super) visited_calls: HashSet<(String, String)>,
    pub(super) visited_edges: HashSet<(String, String, String)>,
    pub(super) visited_external_refs: HashSet<(String, String, ExternalRefKind)>,
    pub(super) type_env: TypeEnvironment,
    pub(super) depth: usize,
}

impl<'a> CallAndImportVisitor<'a> {
    pub(crate) fn new(
        file_path: String,
        content: &'a str,
        language: SupportedLanguage,
        file_symbols: &'a [CodeSymbol],
        symbol_index: &'a HashMap<String, Vec<&'a CodeSymbol>>,
    ) -> Self {
        Self {
            file_path,
            content,
            language,
            file_symbols,
            symbol_index,
            current_caller: None,
            current_container: None,
            edges: Vec::new(),
            external_refs: Vec::new(),
            visited_calls: HashSet::new(),
            visited_edges: HashSet::new(),
            visited_external_refs: HashSet::new(),
            type_env: TypeEnvironment::new(language),
            depth: 0,
        }
    }

    pub(super) fn node_text(&self, node: Node) -> &str {
        &self.content[node.start_byte()..node.end_byte()]
    }

    pub(super) fn add_rel_edge(
        &mut self,
        source: String,
        target: String,
        edge_type: &str,
        weight: f32,
        provenance: EdgeProvenance,
        confidence: ResolutionConfidence,
    ) {
        if source == target || source.is_empty() || target.is_empty() {
            return;
        }
        let key = (source.clone(), target.clone(), edge_type.to_string());
        if !self.visited_edges.contains(&key) {
            self.visited_edges.insert(key);
            self.edges.push(Edge {
                source,
                target,
                edge_type: edge_type.to_string(),
                weight,
                provenance,
                target_corpus: None,
                confidence: Some(confidence),
                target_path: None,
                target_symbol: None,
                target_kind: None,
            });
        }
    }

    /// Record an unresolved call/import target as an [`ExternalRef`], de-duped
    /// per `(caller_scope_path, raw_target, kind)` so re-visits do not duplicate.
    pub(super) fn record_external_ref(
        &mut self,
        caller: String,
        raw_target: String,
        kind: ExternalRefKind,
    ) {
        if caller.is_empty() || raw_target.is_empty() {
            return;
        }
        let key = (caller.clone(), raw_target.clone(), kind);
        if self.visited_external_refs.insert(key) {
            self.external_refs.push(ExternalRef {
                caller_scope_path: caller,
                raw_target,
                kind,
                confidence: ResolutionConfidence::Speculative,
            });
        }
    }

    pub(super) fn resolve_target(&self, raw_target: &str) -> (String, ResolutionConfidence) {
        let clean = raw_target.rsplit("::").next().unwrap_or(raw_target);
        let clean = clean.rsplit('.').next().unwrap_or(clean);

        if let Some(m) = self.file_symbols.iter().find(|s| s.name == clean) {
            return (m.scope_path.clone(), ResolutionConfidence::High);
        }

        if let Some(candidates) = self.symbol_index.get(clean) {
            if candidates.len() == 1 {
                return (candidates[0].scope_path.clone(), ResolutionConfidence::High);
            }
            let file_dir = Path::new(&self.file_path).parent().unwrap_or_else(|| Path::new(""));
            if let Some(dir_match) = candidates.iter().find(|c| {
                Path::new(&c.file_path).parent().unwrap_or_else(|| Path::new("")) == file_dir
            }) {
                return (dir_match.scope_path.clone(), ResolutionConfidence::Medium);
            }
            if let Some(first) = candidates.first() {
                return (first.scope_path.clone(), ResolutionConfidence::Speculative);
            }
        }

        (raw_target.to_string(), ResolutionConfidence::Speculative)
    }

    pub(super) fn extract_name_from_descendants(&self, node: Node) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "object_reference" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    return Some(self.node_text(name_node).trim().to_string());
                }
                return Some(self.node_text(child).trim().to_string());
            }
        }
        None
    }
}
