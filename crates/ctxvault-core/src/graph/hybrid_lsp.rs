//! Pure-Rust In-Engine "Hybrid LSP": Lexical Scope & Static Type Resolver.
//!
//! Tracks lexical scope stacks, variable type bindings, and imported symbols
//! directly on Tree-sitter ASTs to disambiguate method calls and receiver invocations
//! (e.g., `client.query(...)` -> `SearchClient > query`) with [`ResolutionConfidence::High`](ctxvault_common::types::ResolutionConfidence::High),
//! eliminating speculative cross-file call edges without external LSP daemons.
//!
//! # Cross-corpus scope
//!
//! `TypeEnvironment` is an *intra-file* resolver: its scope frames and type
//! bindings are built and consumed while walking a single file's AST during
//! extraction, and are not retained past that. It therefore has no cross-corpus
//! symbol table of its own. The cross-corpus resolver trust ladder
//! (`ResolverKind` in [`crate::corpus_manager`]) reserves a `HybridLsp` tier for
//! when in-engine LSP-grade data becomes queryable across corpora, but keeps the
//! *live* ladder at SCIP → qualified-name so no do-nothing tier is introduced.

use std::collections::HashMap;
use tree_sitter::Node;

use crate::parser::code::languages::SupportedLanguage;

/// Lexical scope frame tracking variable types and imports within a block.
#[derive(Debug, Clone, Default)]
pub struct ScopeFrame {
    /// Local variable name -> Inferred Type Name (e.g., "client" -> "SearchEngine").
    pub variables: HashMap<String, String>,
    /// Imported symbol name -> Module / Import path (e.g., "SearchEngine" -> "crate::search::engine").
    pub imports: HashMap<String, String>,
    /// Enclosing struct/class/impl type name if inside a container, method, or impl block.
    pub enclosing_type: Option<String>,
}

/// Lexical scope stack and type environment.
#[derive(Debug, Clone)]
pub struct TypeEnvironment {
    frames: Vec<ScopeFrame>,
    language: SupportedLanguage,
}

impl TypeEnvironment {
    /// Create a new type environment for a file.
    pub fn new(language: SupportedLanguage) -> Self {
        Self { frames: vec![ScopeFrame::default()], language }
    }

    /// Push a new nested lexical scope frame (e.g. entering function, class, or block).
    pub fn push_scope(&mut self, enclosing_type: Option<String>) {
        let mut frame = ScopeFrame::default();
        if let Some(enc) = enclosing_type {
            frame.enclosing_type = Some(enc);
        } else {
            frame.enclosing_type = self.current_enclosing_type();
        }
        self.frames.push(frame);
    }

    /// Pop the topmost scope frame.
    pub fn pop_scope(&mut self) {
        if self.frames.len() > 1 {
            self.frames.pop();
        }
    }

    /// Get the current active enclosing container/type name (if any).
    pub fn current_enclosing_type(&self) -> Option<String> {
        self.frames.iter().rev().find_map(|f| f.enclosing_type.clone())
    }

    /// Register an imported symbol name and its source import path into file scope.
    pub fn register_import(&mut self, symbol_name: String, source_path: String) {
        if let Some(file_frame) = self.frames.first_mut() {
            file_frame.imports.insert(symbol_name, source_path);
        }
    }

    /// Register a variable binding in the current innermost scope frame.
    pub fn register_variable(&mut self, var_name: String, type_name: String) {
        if let Some(frame) = self.frames.last_mut() {
            frame.variables.insert(var_name, type_name);
        }
    }

    /// Look up the inferred type of a variable or receiver by walking up the scope stack.
    pub fn resolve_variable_type(&self, var_name: &str) -> Option<String> {
        // Special case: `self` or `this` maps to current enclosing type
        if var_name == "self" || var_name == "this" {
            return self.current_enclosing_type();
        }

        for frame in self.frames.iter().rev() {
            if let Some(t) = frame.variables.get(var_name) {
                return Some(t.clone());
            }
        }
        None
    }

    /// Look up the import source of a symbol name.
    pub fn resolve_import_source(&self, symbol_name: &str) -> Option<String> {
        self.frames.first().and_then(|f| f.imports.get(symbol_name).cloned())
    }

    /// Inspect an AST node and update the type environment with bindings (declarations, parameters).
    pub fn inspect_node(&mut self, node: Node, content: &str) {
        match self.language {
            SupportedLanguage::Rust => self.inspect_rust_node(node, content),
            SupportedLanguage::TypeScript
            | SupportedLanguage::Tsx
            | SupportedLanguage::JavaScript => self.inspect_js_ts_node(node, content),
            SupportedLanguage::Python => self.inspect_python_node(node, content),
            SupportedLanguage::Go => self.inspect_go_node(node, content),
            _ => {}
        }
    }

    fn inspect_rust_node(&mut self, node: Node, content: &str) {
        let kind = node.kind();

        // 1. `let x: Type = ...` or `let x = Type::new(...)`
        if kind == "let_declaration" {
            let pattern_node = node.child_by_field_name("pattern");
            let var_name =
                pattern_node.map(|p| node_text(p, content).trim().to_string()).unwrap_or_default();
            let var_name = var_name.trim_start_matches("mut ").trim().to_string();

            // Check explicit type annotation: `let x: Type = ...`
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = clean_type_name(node_text(type_node, content));
                if !var_name.is_empty() && !type_name.is_empty() {
                    self.register_variable(var_name, type_name);
                    return;
                }
            }

            // Check constructor: `let x = Type::new(...)` or `let x = Type { ... }`
            if let Some(val_node) = node.child_by_field_name("value") {
                if let Some(inferred_type) = infer_rust_value_type(val_node, content) {
                    if !var_name.is_empty() {
                        self.register_variable(var_name, inferred_type);
                        return;
                    }
                }
            }
        }

        // 2. Parameters: `fn foo(client: &SearchClient)`
        if kind == "parameter" {
            if let (Some(pat), Some(ty)) =
                (node.child_by_field_name("pattern"), node.child_by_field_name("type"))
            {
                let var_name = node_text(pat, content).trim().to_string();
                let type_name = clean_type_name(node_text(ty, content));
                if !var_name.is_empty() && !type_name.is_empty() {
                    self.register_variable(var_name, type_name);
                }
            }
        }
    }

    fn inspect_js_ts_node(&mut self, node: Node, content: &str) {
        let kind = node.kind();

        // `const client: SearchClient = ...` or `const client = new SearchClient()`
        if kind == "variable_declarator" {
            let name_node = node.child_by_field_name("name");
            let var_name =
                name_node.map(|n| node_text(n, content).trim().to_string()).unwrap_or_default();

            // Explicit type: `const client: SearchClient`
            if let Some(ty_node) = node.child_by_field_name("type") {
                let type_name = clean_type_name(node_text(ty_node, content));
                if !var_name.is_empty() && !type_name.is_empty() {
                    self.register_variable(var_name, type_name);
                    return;
                }
            }

            // `new SearchClient(...)`
            if let Some(val_node) = node.child_by_field_name("value") {
                if val_node.kind() == "new_expression" {
                    if let Some(constructor) = val_node.child_by_field_name("constructor") {
                        let type_name = clean_type_name(node_text(constructor, content));
                        if !var_name.is_empty() && !type_name.is_empty() {
                            self.register_variable(var_name, type_name);
                            return;
                        }
                    }
                }
            }
        }

        // Function parameters: `function query(client: SearchClient)`
        if kind == "required_parameter" || kind == "optional_parameter" {
            if let (Some(pat), Some(ty)) =
                (node.child_by_field_name("pattern"), node.child_by_field_name("type"))
            {
                let var_name = node_text(pat, content).trim().to_string();
                let type_name = clean_type_name(node_text(ty, content));
                if !var_name.is_empty() && !type_name.is_empty() {
                    self.register_variable(var_name, type_name);
                }
            }
        }
    }

    fn inspect_python_node(&mut self, node: Node, content: &str) {
        // Assignment: `client = SearchClient(...)`
        if node.kind() == "assignment" {
            if let (Some(left), Some(right)) =
                (node.child_by_field_name("left"), node.child_by_field_name("right"))
            {
                let var_name = node_text(left, content).trim().to_string();
                if right.kind() == "call" {
                    if let Some(func) = right.child_by_field_name("function") {
                        let constructor_name = node_text(func, content).trim().to_string();
                        // Capitalized identifiers in Python are standard class instantiations
                        if constructor_name
                            .chars()
                            .next()
                            .map(|c| c.is_ascii_uppercase())
                            .unwrap_or(false)
                        {
                            self.register_variable(var_name, constructor_name);
                        }
                    }
                }
            }
        }
    }

    fn inspect_go_node(&mut self, node: Node, content: &str) {
        // Short var declaration: `client := NewSearchClient(...)` or `kl := &Kubelet{...}`
        if node.kind() == "short_var_declaration" {
            if let (Some(left), Some(right)) =
                (node.child_by_field_name("left"), node.child_by_field_name("right"))
            {
                let var_name = node_text(left, content).trim().to_string();
                if right.kind() == "call_expression" {
                    if let Some(func) = right.child_by_field_name("function") {
                        let fn_text = node_text(func, content).trim();
                        let fn_name = fn_text.rsplit('.').next().unwrap_or(fn_text);
                        if let Some(type_name) = fn_name.strip_prefix("New") {
                            self.register_variable(var_name, type_name.to_string());
                        }
                    }
                } else if right.kind() == "composite_literal" {
                    if let Some(ty) = right.child_by_field_name("type") {
                        let type_name = clean_type_name(node_text(ty, content));
                        self.register_variable(var_name, type_name);
                    }
                } else if right.kind() == "unary_expression" {
                    if let Some(operand) = right.child_by_field_name("operand") {
                        if operand.kind() == "composite_literal" {
                            if let Some(ty) = operand.child_by_field_name("type") {
                                let type_name = clean_type_name(node_text(ty, content));
                                self.register_variable(var_name, type_name);
                            }
                        }
                    }
                }
            }
        }

        // Receiver: `func (s *SearchEngine) Query(...)`
        if node.kind() == "method_declaration" {
            if let Some(receiver) = node.child_by_field_name("receiver") {
                if let Some(param) = receiver.child(0) {
                    let text = node_text(param, content);
                    let parts: Vec<&str> = text.split_whitespace().collect();
                    if parts.len() == 2 {
                        let var_name = parts[0].trim().to_string();
                        let type_name = clean_type_name(parts[1]);
                        self.register_variable(var_name, type_name);
                    }
                }
            }
        }
    }
}

fn node_text<'a>(node: Node, content: &'a str) -> &'a str {
    &content[node.start_byte()..node.end_byte()]
}

/// Clean a raw type string by stripping references, pointers, and generics.
pub fn clean_type_name(raw: &str) -> String {
    let mut cleaned = raw.trim();
    // Strip Rust reference/pointer markers
    cleaned = cleaned.trim_start_matches('&');
    cleaned = cleaned.trim_start_matches('*');
    cleaned = cleaned.trim_start_matches("const ");
    cleaned = cleaned.trim_start_matches("mut ");
    // Strip TypeScript type prefix colon
    cleaned = cleaned.trim_start_matches(':').trim();
    // Strip generic brackets: `Option<SearchEngine>` -> `SearchEngine`
    if let Some(start) = cleaned.find('<') {
        if let Some(rel_end) = cleaned[start + 1..].rfind('>') {
            let end = start + 1 + rel_end;
            if start + 1 < end {
                let inner = &cleaned[start + 1..end];
                if !inner.contains(',') {
                    return clean_type_name(inner);
                }
            }
        }
    }
    // Strip leading path: `crate::search::SearchEngine` -> `SearchEngine`
    cleaned.rsplit("::").next().unwrap_or(cleaned).to_string()
}

fn infer_rust_value_type(val_node: Node, content: &str) -> Option<String> {
    match val_node.kind() {
        "call_expression" => {
            let func = val_node.child_by_field_name("function").or_else(|| val_node.child(0))?;
            let func_text = node_text(func, content).trim();
            if let Some((type_part, _method)) = func_text.rsplit_once("::") {
                let type_name = type_part.rsplit("::").next().unwrap_or(type_part).trim();
                if type_name.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
                    return Some(clean_type_name(type_name));
                }
            }
            None
        }
        "struct_expression" => {
            let name = val_node.child_by_field_name("name").or_else(|| val_node.child(0))?;
            let type_name = node_text(name, content).trim();
            let type_name = type_name.rsplit("::").next().unwrap_or(type_name).trim();
            if type_name.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
                Some(clean_type_name(type_name))
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_type_name_edge_cases() {
        assert_eq!(clean_type_name("Option<SearchEngine>"), "SearchEngine");
        assert_eq!(clean_type_name("-> Result<String>"), "String");
        assert_eq!(clean_type_name("foo > bar <baz>"), "baz");
        assert_eq!(clean_type_name("a > b && c < d"), "a > b && c < d");
        assert_eq!(clean_type_name("><"), "><");
        assert_eq!(clean_type_name("<>"), "<>");
        assert_eq!(clean_type_name(">>>"), ">>>");
        assert_eq!(clean_type_name("<<<"), "<<<");
        assert_eq!(clean_type_name("&mut SearchClient"), "SearchClient");
        assert_eq!(clean_type_name("*const u8"), "u8");
        assert_eq!(clean_type_name(": SearchEngine"), "SearchEngine");
        assert_eq!(clean_type_name("crate::search::SearchEngine"), "SearchEngine");
        assert_eq!(clean_type_name("Arc<Mutex<SearchEngine>>"), "SearchEngine");
        assert_eq!(clean_type_name("Result<Engine, Error>"), "Result<Engine, Error>");
    }
}
