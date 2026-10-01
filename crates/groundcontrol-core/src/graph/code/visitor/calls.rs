//! Call expression extraction and callee symbol resolution across languages.

use std::path::Path;

use groundcontrol_common::types::{
    CodeSymbol, Edge, EdgeProvenance, ExternalRefKind, ResolutionConfidence,
};
use tree_sitter::Node;

use super::state::CallAndImportVisitor;

impl<'a> CallAndImportVisitor<'a> {
    pub(super) fn extract_call(&mut self, node: Node) {
        let Some(caller) = self.current_caller.clone() else {
            return;
        };
        let caller = &caller;

        let Some((receiver, callee)) = self.extract_call_parts(node) else {
            return;
        };

        if let Some((target_sym, confidence)) = self.resolve_callee(receiver.as_deref(), &callee) {
            let key = (caller.clone(), target_sym.scope_path.clone());
            if !self.visited_calls.contains(&key) && caller != &target_sym.scope_path {
                self.visited_calls.insert(key);
                self.edges.push(Edge {
                    source: caller.clone(),
                    target: target_sym.scope_path.clone(),
                    edge_type: "calls".to_string(),
                    weight: 0.8,
                    provenance: EdgeProvenance::CodeCalls,
                    target_corpus: None,
                    confidence: Some(confidence),
                    target_path: None,
                    target_symbol: None,
                    target_kind: None,
                });
            }

            // Pure AST test-to-target linking: if caller is a test function and callee is not an assertion sink
            if self.test_callers.contains(caller) && !Self::is_assertion_sink(&callee) {
                let test_key = (caller.clone(), target_sym.scope_path.clone(), "tests".to_string());
                if !self.visited_edges.contains(&test_key) && caller != &target_sym.scope_path {
                    self.visited_edges.insert(test_key);
                    self.edges.push(Edge {
                        source: caller.clone(),
                        target: target_sym.scope_path.clone(),
                        edge_type: "tests".to_string(),
                        weight: 0.9,
                        provenance: EdgeProvenance::CodeTests,
                        target_corpus: None,
                        confidence: Some(confidence),
                        target_path: None,
                        target_symbol: None,
                        target_kind: None,
                    });
                }
            }
        } else {
            // Unresolved callee: do NOT emit a phantom edge to a non-existent node.
            // Only record as an external reference if it's not a local self/this method,
            // so cross-corpus federation can link it if exported by another corpus.
            if receiver.as_deref() != Some("self") && receiver.as_deref() != Some("this") {
                let target = match receiver.as_deref() {
                    Some(rec) if !rec.is_empty() => format!("{}.{}", rec, callee),
                    _ => callee.clone(),
                };
                let key = (caller.clone(), target.clone());
                if !self.visited_calls.contains(&key) && caller != &target {
                    self.visited_calls.insert(key);
                    let caller = caller.clone();
                    self.record_external_ref(caller, target, ExternalRefKind::Call);
                }
            }
        }
    }

    pub(super) fn extract_call_parts(&self, node: Node) -> Option<(Option<String>, String)> {
        let kind = node.kind();
        if kind == "method_call_expression" {
            let method = node.child_by_field_name("name")?;
            let receiver =
                node.child_by_field_name("receiver").map(|r| self.node_text(r).trim().to_string());
            return Some((receiver, self.node_text(method).trim().to_string()));
        }

        if kind == "method_invocation" {
            let method = node.child_by_field_name("name")?;
            let receiver =
                node.child_by_field_name("object").map(|r| self.node_text(r).trim().to_string());
            return Some((receiver, self.node_text(method).trim().to_string()));
        }

        if kind == "call_expression" || kind == "call" || kind == "function_call" {
            let func = node.child_by_field_name("function").or_else(|| node.child(0))?;
            let func_kind = func.kind();

            if func_kind == "field_expression" {
                let field = func.child_by_field_name("field")?;
                let receiver = func
                    .child_by_field_name("value")
                    .or_else(|| func.child_by_field_name("argument"))
                    .map(|a| self.node_text(a).trim().to_string());
                return Some((receiver, self.node_text(field).trim().to_string()));
            }

            if func_kind == "scoped_identifier" {
                let name = func.child_by_field_name("name")?;
                let path =
                    func.child_by_field_name("path").map(|p| self.node_text(p).trim().to_string());
                return Some((path, self.node_text(name).trim().to_string()));
            }

            if func_kind == "member_expression" {
                let prop = func.child_by_field_name("property")?;
                let obj = func
                    .child_by_field_name("object")
                    .map(|o| self.node_text(o).trim().to_string());
                return Some((obj, self.node_text(prop).trim().to_string()));
            }

            if func_kind == "attribute" {
                let attr = func.child_by_field_name("attribute")?;
                let val = func
                    .child_by_field_name("value")
                    .or_else(|| func.child_by_field_name("object"))
                    .map(|v| self.node_text(v).trim().to_string());
                return Some((val, self.node_text(attr).trim().to_string()));
            }

            if func_kind == "selector_expression" {
                let field = func.child_by_field_name("field")?;
                let operand = func
                    .child_by_field_name("operand")
                    .map(|o| self.node_text(o).trim().to_string());
                return Some((operand, self.node_text(field).trim().to_string()));
            }

            if func_kind == "identifier" || func_kind == "property_identifier" {
                return Some((None, self.node_text(func).trim().to_string()));
            }

            let full = self.node_text(func).trim();
            if let Some((rec, method)) = full.rsplit_once('.') {
                return Some((Some(rec.trim().to_string()), method.trim().to_string()));
            }
            if let Some((rec, method)) = full.rsplit_once("::") {
                return Some((Some(rec.trim().to_string()), method.trim().to_string()));
            }
            if let Some((rec, method)) = full.rsplit_once("->") {
                return Some((Some(rec.trim().to_string()), method.trim().to_string()));
            }
            if !full.is_empty() {
                return Some((None, full.to_string()));
            }
        }

        if kind == "invocation_expression" {
            if let Some(expr) = node.child_by_field_name("expression") {
                if expr.kind() == "member_access_expression" {
                    let name = expr.child_by_field_name("name")?;
                    let expr_node = expr
                        .child_by_field_name("expression")
                        .map(|e| self.node_text(e).trim().to_string());
                    return Some((expr_node, self.node_text(name).trim().to_string()));
                }
            }
        }

        None
    }

    /// Resolve a callee name to a symbol, returning the resolution confidence band.
    ///
    /// When a receiver is present and resolved by Hybrid LSP type tracking, matching
    /// container methods are resolved with [`ResolutionConfidence::High`].
    pub(super) fn resolve_callee(
        &self,
        receiver: Option<&str>,
        callee_name: &str,
    ) -> Option<(&'a CodeSymbol, ResolutionConfidence)> {
        let clean_name = callee_name.rsplit("::").next().unwrap_or(callee_name);
        let clean_name = clean_name.rsplit('.').next().unwrap_or(clean_name);

        // 1. Intra-file definition: if callee is defined in the current file with no receiver
        if receiver.is_none() {
            if let Some(local_match) = self.file_symbols.iter().find(|s| s.name == clean_name) {
                return Some((local_match, ResolutionConfidence::High));
            }
        }

        // Filter candidate symbols by language compatibility and exclude data format symbols
        let valid_candidates: Vec<&CodeSymbol> = match self.symbol_index.get(clean_name) {
            Some(candidates) => candidates
                .iter()
                .copied()
                .filter(|c| {
                    !is_data_format_symbol(&c.language)
                        && is_compatible_language(self.language.name(), &c.language)
                })
                .collect(),
            None => Vec::new(),
        };

        // 2. Tier 2: Receiver-guided resolution via lexical scope & import table
        if let Some(rec) = receiver {
            let rec_clean = rec.trim();
            // A. Check lexical scope variable type binding or import table for receiver
            let resolved_type = self.type_env.resolve_variable_type(rec_clean).or_else(|| {
                if self.import_table.symbols.contains_key(rec_clean) {
                    Some(rec_clean.to_string())
                } else if self.file_symbols.iter().any(|s| s.name == rec_clean)
                    || self.symbol_index.contains_key(rec_clean)
                {
                    Some(rec_clean.to_string())
                } else {
                    None
                }
            });

            if let Some(ref type_name) = resolved_type {
                // Check within current file symbols
                if let Some(local_match) = self.file_symbols.iter().find(|s| {
                    s.name == clean_name
                        && (s.scope_path.contains(type_name.as_str())
                            || s.scope_path == format!("{type_name} > {clean_name}"))
                }) {
                    return Some((local_match, ResolutionConfidence::High));
                }

                // Check workspace symbol catalog
                if !valid_candidates.is_empty() {
                    let type_matches: Vec<&CodeSymbol> = valid_candidates
                        .iter()
                        .copied()
                        .filter(|s| {
                            s.scope_path.contains(type_name.as_str())
                                || s.scope_path == format!("{type_name} > {clean_name}")
                        })
                        .collect();

                    if !type_matches.is_empty() {
                        // Disambiguate using ImportTable target path
                        if let Some(import_match) = type_matches.iter().find(|c| {
                            self.import_table.matches_target_path(type_name, &c.file_path)
                        }) {
                            return Some((import_match, ResolutionConfidence::High));
                        }
                        if type_matches.len() == 1 {
                            return Some((type_matches[0], ResolutionConfidence::High));
                        }
                        let file_dir =
                            Path::new(&self.file_path).parent().unwrap_or_else(|| Path::new(""));
                        if let Some(dir_match) = type_matches.iter().find(|c| {
                            Path::new(&c.file_path).parent().unwrap_or_else(|| Path::new(""))
                                == file_dir
                        }) {
                            return Some((dir_match, ResolutionConfidence::High));
                        }
                        return Some((type_matches[0], ResolutionConfidence::High));
                    }
                }
            }

            // B. Receiver is a module/namespace import (e.g. `api.fetchData()` or `server.Handle()`)
            if let Some(mod_res) = self.import_table.modules.get(rec_clean) {
                if !valid_candidates.is_empty() {
                    if let Some(matched) = valid_candidates.iter().find(|c| {
                        if let Some(ref prefix) = mod_res.target_path_prefix {
                            let norm_c = c.file_path.replace('\\', "/");
                            norm_c.starts_with(prefix) || norm_c.contains(prefix)
                        } else {
                            false
                        }
                    }) {
                        return Some((matched, ResolutionConfidence::High));
                    }
                }
            }
        }

        // 3. Tier 2: Direct call to an explicitly imported function/symbol
        if !valid_candidates.is_empty() {
            if self.import_table.symbols.contains_key(clean_name) {
                if let Some(import_match) = valid_candidates
                    .iter()
                    .find(|c| self.import_table.matches_target_path(clean_name, &c.file_path))
                {
                    return Some((import_match, ResolutionConfidence::High));
                }
            }
        }

        // 4. Local file fallback if not checked above
        if let Some(local_match) = self.file_symbols.iter().find(|s| s.name == clean_name) {
            return Some((local_match, ResolutionConfidence::High));
        }

        // 5. Tier 3: Intra-Module / Same-Package Resolution
        if !valid_candidates.is_empty() {
            // Unique bare-name match across workspace is Medium confidence (not High)
            if valid_candidates.len() == 1 {
                return Some((valid_candidates[0], ResolutionConfidence::Medium));
            }
            let file_dir = Path::new(&self.file_path).parent().unwrap_or_else(|| Path::new(""));
            if let Some(dir_match) = valid_candidates.iter().find(|c| {
                Path::new(&c.file_path).parent().unwrap_or_else(|| Path::new("")) == file_dir
            }) {
                return Some((dir_match, ResolutionConfidence::Medium));
            }
            // 6. Tier 4: Speculative fallback
            return valid_candidates.first().map(|c| (*c, ResolutionConfidence::Speculative));
        }

        None
    }

    pub(super) fn is_assertion_sink(callee: &str) -> bool {
        matches!(
            callee,
            "assert"
                | "assert_eq"
                | "assert_ne"
                | "assert_true"
                | "assert_false"
                | "assert_that"
                | "assert_nil"
                | "assert_not_nil"
                | "assertEquals"
                | "assertNotEquals"
                | "assertTrue"
                | "assertFalse"
                | "assertNull"
                | "assertNotNull"
                | "assertSame"
                | "assertNotSame"
                | "assertThat"
                | "assertThrows"
                | "fail"
                | "expect"
                | "expect_eq"
                | "expect_ne"
                | "Equal"
                | "NotEqual"
                | "True"
                | "False"
                | "Null"
                | "NotNull"
                | "Run"
                | "Parallel"
                | "Error"
                | "Errorf"
                | "Fatal"
                | "Fatalf"
                | "describe"
                | "it"
                | "test"
                | "beforeEach"
                | "afterEach"
                | "beforeAll"
                | "afterAll"
        ) || callee.starts_with("assert_")
            || callee.starts_with("assert!")
            || callee.starts_with("assert_eq!")
            || callee.starts_with("assert_ne!")
    }
}

/// Checks if two languages are compatible for cross-file call resolution.
fn is_compatible_language(source_lang: &str, target_lang: &str) -> bool {
    if source_lang.eq_ignore_ascii_case(target_lang) {
        return true;
    }
    let is_js = |l: &str| matches!(l, "typescript" | "tsx" | "javascript");
    if is_js(source_lang) && is_js(target_lang) {
        return true;
    }
    let is_c = |l: &str| matches!(l, "c" | "cpp");
    if is_c(source_lang) && is_c(target_lang) {
        return true;
    }
    false
}

/// Checks if a symbol belongs to a data serialization format (e.g. YAML, XML, JSON).
fn is_data_format_symbol(language: &str) -> bool {
    matches!(language, "json" | "yaml" | "xml" | "toml")
}
