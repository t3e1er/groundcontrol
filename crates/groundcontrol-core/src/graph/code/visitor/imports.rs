//! Import and include relationship extraction across languages.

use groundcontrol_common::types::{Edge, EdgeProvenance, ResolutionConfidence};
use tree_sitter::Node;

use crate::parser::code::languages::SupportedLanguage;

use super::state::CallAndImportVisitor;

impl<'a> CallAndImportVisitor<'a> {
    pub(super) fn extract_import(&mut self, node: Node) {
        let kind = node.kind();
        match self.language {
            SupportedLanguage::Rust => {
                if kind == "use_declaration" {
                    let text = self.node_text(node).trim().trim_end_matches(';').trim();
                    if let Some(target) = text.strip_prefix("use ") {
                        let target_str = target.trim().to_string();
                        let sym = target_str.rsplit("::").next().unwrap_or(&target_str).to_string();
                        self.edges.push(Edge {
                            source: self.file_path.clone(),
                            target: target_str.clone(),
                            edge_type: "imports".to_string(),
                            weight: 0.6,
                            provenance: EdgeProvenance::CodeImports,
                            target_corpus: None,
                            confidence: Some(ResolutionConfidence::Speculative),
                            target_path: None,
                            target_symbol: None,
                            target_kind: None,
                        });
                        self.type_env.register_import(sym, target_str);
                    }
                }
            }
            SupportedLanguage::TypeScript
            | SupportedLanguage::Tsx
            | SupportedLanguage::JavaScript => {
                if kind == "import_statement" {
                    if let Some(source_node) = node.child_by_field_name("source") {
                        let raw = self
                            .node_text(source_node)
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string();
                        self.edges.push(Edge {
                            source: self.file_path.clone(),
                            target: raw,
                            edge_type: "imports".to_string(),
                            weight: 0.6,
                            provenance: EdgeProvenance::CodeImports,
                            target_corpus: None,
                            confidence: Some(ResolutionConfidence::Speculative),
                            target_path: None,
                            target_symbol: None,
                            target_kind: None,
                        });
                    }
                }
            }
            SupportedLanguage::Python => {
                if kind == "import_statement" || kind == "import_from_statement" {
                    let text = self.node_text(node).trim().to_string();
                    self.edges.push(Edge {
                        source: self.file_path.clone(),
                        target: text,
                        edge_type: "imports".to_string(),
                        weight: 0.6,
                        provenance: EdgeProvenance::CodeImports,
                        target_corpus: None,
                        confidence: Some(ResolutionConfidence::Speculative),
                        target_path: None,
                        target_symbol: None,
                        target_kind: None,
                    });
                }
            }
            SupportedLanguage::Go => {
                if kind == "import_spec" {
                    let path = self.node_text(node).trim().trim_matches('"').to_string();
                    let pkg = path.rsplit('/').next().unwrap_or(&path).to_string();
                    self.edges.push(Edge {
                        source: self.file_path.clone(),
                        target: path.clone(),
                        edge_type: "imports".to_string(),
                        weight: 0.6,
                        provenance: EdgeProvenance::CodeImports,
                        target_corpus: None,
                        confidence: Some(ResolutionConfidence::Speculative),
                        target_path: None,
                        target_symbol: None,
                        target_kind: None,
                    });
                    self.type_env.register_import(pkg, path);
                }
            }
            _ => {
                let text = self.node_text(node).trim().trim_end_matches(';').trim();
                let clean = text
                    .strip_prefix("import ")
                    .or_else(|| text.strip_prefix("#include "))
                    .or_else(|| text.strip_prefix("include "))
                    .or_else(|| text.strip_prefix("using "))
                    .unwrap_or(text)
                    .trim()
                    .trim_matches('"')
                    .trim_matches('<')
                    .trim_matches('>');
                if !clean.is_empty() && clean.len() < 200 {
                    self.edges.push(Edge {
                        source: self.file_path.clone(),
                        target: clean.to_string(),
                        edge_type: "imports".to_string(),
                        weight: 0.6,
                        provenance: EdgeProvenance::CodeImports,
                        target_corpus: None,
                        confidence: Some(ResolutionConfidence::Speculative),
                        target_path: None,
                        target_symbol: None,
                        target_kind: None,
                    });
                }
            }
        }
    }
}
