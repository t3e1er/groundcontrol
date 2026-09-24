//! Polyglot AST visitor traversing tree-sitter nodes for call, import, and structural relationships.

pub mod calls;
pub mod defs;
pub mod imports;
pub mod state;

use tree_sitter::Node;

use crate::graph::hybrid_lsp::clean_type_name;
use crate::parser::code::languages::SupportedLanguage;
use crate::parser::code::spec::get_language_spec;

pub(crate) use state::CallAndImportVisitor;

impl<'a> CallAndImportVisitor<'a> {
    const MAX_AST_DEPTH: usize = 256;

    pub(crate) fn visit(&mut self, node: Node) {
        if self.depth >= Self::MAX_AST_DEPTH {
            return;
        }
        self.depth += 1;
        self.visit_inner(node);
        self.depth -= 1;
    }

    fn visit_inner(&mut self, node: Node) {
        let kind = node.kind();
        let spec = get_language_spec(self.language);

        // Track container scope (class, struct, trait, interface, or Rust impl)
        let is_rust_impl = self.language == SupportedLanguage::Rust && kind == "impl_item";
        let is_container = is_rust_impl
            || spec.class_node_kinds.contains(&kind)
            || spec.struct_node_kinds.contains(&kind)
            || spec.trait_node_kinds.contains(&kind)
            || spec.interface_node_kinds.contains(&kind);

        if is_container {
            let container_name = if is_rust_impl {
                node.child_by_field_name("type").map(|t| clean_type_name(self.node_text(t)))
            } else if let Some(n) = node.child_by_field_name("name") {
                Some(clean_type_name(self.node_text(n)))
            } else {
                self.extract_name_from_descendants(node)
            };

            let prev_container = self.current_container.take();
            self.current_container = container_name.clone().or_else(|| prev_container.clone());
            self.type_env.push_scope(container_name);

            // Container-level edge extractions
            self.extract_container_edges(node);

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.visit(child);
            }

            self.type_env.pop_scope();
            self.current_container = prev_container;
            return;
        }

        // Track caller function/method scope
        if spec.is_callable(kind) {
            let start_line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;
            let matching_sym = self
                .file_symbols
                .iter()
                .filter(|s| s.start_line <= start_line && s.end_line >= end_line)
                .min_by_key(|s| s.end_line - s.start_line);

            let prev_caller = self.current_caller.take();
            if let Some(sym) = matching_sym {
                self.current_caller = Some(sym.scope_path.clone());
            } else {
                self.current_caller = prev_caller.clone();
            }

            self.type_env.push_scope(None);

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.type_env.inspect_node(child, self.content);
                self.visit(child);
            }

            self.type_env.pop_scope();
            self.current_caller = prev_caller;
            return;
        }

        // Inside a body: inspect statements/declarations for variable bindings
        self.type_env.inspect_node(node, self.content);

        // Language-specific AST relationship extractions:
        match self.language {
            SupportedLanguage::TypeScript
            | SupportedLanguage::Tsx
            | SupportedLanguage::JavaScript => {
                if kind == "decorator" {
                    self.extract_ts_decorator(node);
                }
            }
            SupportedLanguage::Python => {
                if kind == "decorated_definition" {
                    self.extract_python_decorated(node);
                }
            }
            SupportedLanguage::Rust => {
                if kind == "macro_invocation" {
                    self.extract_rust_macro(node);
                }
            }
            _ => {}
        }

        // Extract imports
        if spec.is_import(kind) {
            self.extract_import(node);
        }

        // Extract call expressions
        if spec.is_call(kind) {
            self.extract_call(node);
        }

        // Extract trait implementations
        self.extract_implements(node);

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child);
        }
    }
}
