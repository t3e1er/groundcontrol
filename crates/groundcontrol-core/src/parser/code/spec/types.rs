//! Declarative language specification types for polyglot AST code extraction.

use crate::parser::code::languages::SupportedLanguage;
use groundcontrol_common::types::CodeSymbolType;
use tree_sitter::Node;

/// Declarative specification for syntactic code extraction.
#[derive(Debug, Clone, Copy)]
pub struct LanguageSpec {
    /// Associated language enum variant.
    pub language: SupportedLanguage,
    /// AST node kinds representing functions.
    pub function_node_kinds: &'static [&'static str],
    /// AST node kinds representing methods.
    pub method_node_kinds: &'static [&'static str],
    /// AST node kinds representing classes.
    pub class_node_kinds: &'static [&'static str],
    /// AST node kinds representing structs.
    pub struct_node_kinds: &'static [&'static str],
    /// AST node kinds representing interfaces.
    pub interface_node_kinds: &'static [&'static str],
    /// AST node kinds representing traits.
    pub trait_node_kinds: &'static [&'static str],
    /// AST node kinds representing enums.
    pub enum_node_kinds: &'static [&'static str],
    /// AST node kinds representing modules / namespaces / packages.
    pub module_node_kinds: &'static [&'static str],
    /// AST node kinds representing type aliases.
    pub type_alias_node_kinds: &'static [&'static str],
    /// Named child field containing the symbol identifier (default is `"name"`).
    pub name_field: Option<&'static str>,
    /// Single-line comment prefix for scope breadcrumbs.
    pub comment_prefix: &'static str,
    /// AST node kinds representing doc comments or comments.
    pub doc_comment_kinds: &'static [&'static str],
    /// AST node kinds that can be caller scopes.
    pub callable_node_kinds: &'static [&'static str],
    /// AST node kinds representing import statements.
    pub import_node_kinds: &'static [&'static str],
    /// AST node kinds representing function / method calls.
    pub call_node_kinds: &'static [&'static str],
}

impl LanguageSpec {
    /// Classify a Tree-sitter AST node into a [`CodeSymbolType`], if it represents a symbol.
    pub fn classify_symbol(&self, node: Node) -> Option<CodeSymbolType> {
        let kind = node.kind();
        if self.function_node_kinds.contains(&kind) {
            Some(CodeSymbolType::Function)
        } else if self.method_node_kinds.contains(&kind) {
            Some(CodeSymbolType::Method)
        } else if self.class_node_kinds.contains(&kind) {
            Some(CodeSymbolType::Class)
        } else if self.struct_node_kinds.contains(&kind) {
            Some(CodeSymbolType::Struct)
        } else if self.interface_node_kinds.contains(&kind) {
            Some(CodeSymbolType::Interface)
        } else if self.trait_node_kinds.contains(&kind) {
            Some(CodeSymbolType::Trait)
        } else if self.enum_node_kinds.contains(&kind) {
            Some(CodeSymbolType::Enum)
        } else if self.module_node_kinds.contains(&kind) {
            Some(CodeSymbolType::Module)
        } else if self.type_alias_node_kinds.contains(&kind) {
            Some(CodeSymbolType::TypeAlias)
        } else {
            None
        }
    }

    /// Whether a node kind is callable (can act as a caller function/method in call graphs).
    pub fn is_callable(&self, kind: &str) -> bool {
        self.callable_node_kinds.contains(&kind)
    }

    /// Whether a node kind is an import statement.
    pub fn is_import(&self, kind: &str) -> bool {
        self.import_node_kinds.contains(&kind)
    }

    /// Whether a node kind is a call expression.
    pub fn is_call(&self, kind: &str) -> bool {
        self.call_node_kinds.contains(&kind)
    }

    /// Whether a symbol type is a container (class, struct, trait, interface, module, enum).
    pub fn is_container(sym_type: CodeSymbolType) -> bool {
        matches!(
            sym_type,
            CodeSymbolType::Class
                | CodeSymbolType::Struct
                | CodeSymbolType::Trait
                | CodeSymbolType::Interface
                | CodeSymbolType::Module
                | CodeSymbolType::Enum
        )
    }

    /// Tree-sitter field name representing parameters or formal arguments list.
    pub fn parameters_field(&self) -> &'static str {
        match self.language {
            SupportedLanguage::Python
            | SupportedLanguage::Rust
            | SupportedLanguage::Go
            | SupportedLanguage::TypeScript
            | SupportedLanguage::JavaScript
            | SupportedLanguage::Java
            | SupportedLanguage::CSharp
            | SupportedLanguage::Cpp
            | SupportedLanguage::C => "parameters",
            SupportedLanguage::Ruby => "parameters",
            SupportedLanguage::Php => "parameters",
            _ => "parameters",
        }
    }

    /// Tree-sitter field name representing the declared return or result type.
    pub fn return_type_field(&self) -> &'static str {
        match self.language {
            SupportedLanguage::Go => "result",
            SupportedLanguage::Java
            | SupportedLanguage::CSharp
            | SupportedLanguage::C
            | SupportedLanguage::Cpp => "type",
            SupportedLanguage::Rust
            | SupportedLanguage::TypeScript
            | SupportedLanguage::JavaScript
            | SupportedLanguage::Python => "return_type",
            _ => "return_type",
        }
    }

    /// Tree-sitter field name representing the implementation block or body.
    pub fn body_field(&self) -> &'static str {
        match self.language {
            SupportedLanguage::Python
            | SupportedLanguage::Rust
            | SupportedLanguage::Go
            | SupportedLanguage::TypeScript
            | SupportedLanguage::JavaScript
            | SupportedLanguage::Java
            | SupportedLanguage::CSharp
            | SupportedLanguage::Cpp
            | SupportedLanguage::C => "body",
            _ => "body",
        }
    }

    /// Tree-sitter field name representing the callee / function target in a call expression.
    pub fn call_function_field(&self) -> &'static str {
        match self.language {
            SupportedLanguage::Python
            | SupportedLanguage::Rust
            | SupportedLanguage::Go
            | SupportedLanguage::TypeScript
            | SupportedLanguage::JavaScript
            | SupportedLanguage::Java
            | SupportedLanguage::CSharp
            | SupportedLanguage::Cpp
            | SupportedLanguage::C => "function",
            _ => "function",
        }
    }

    /// Tree-sitter field name representing arguments in a call expression.
    pub fn call_arguments_field(&self) -> &'static str {
        match self.language {
            SupportedLanguage::Python
            | SupportedLanguage::Rust
            | SupportedLanguage::Go
            | SupportedLanguage::TypeScript
            | SupportedLanguage::JavaScript
            | SupportedLanguage::Java
            | SupportedLanguage::CSharp
            | SupportedLanguage::Cpp
            | SupportedLanguage::C => "arguments",
            _ => "arguments",
        }
    }
}
