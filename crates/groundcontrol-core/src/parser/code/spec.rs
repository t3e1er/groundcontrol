//! Declarative language specifications for polyglot AST code extraction.
//!
//! Replaces procedural per-language classification functions with a unified declarative
//! table mapping Tree-sitter AST node kinds to [`CodeSymbolType`] and semantic roles.

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
}

/// Retrieve the declarative specification for a given supported language.
pub fn get_language_spec(lang: SupportedLanguage) -> &'static LanguageSpec {
    match lang {
        SupportedLanguage::Rust => &RUST_SPEC,
        SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => &TS_JS_SPEC,
        SupportedLanguage::Tsx => &TSX_SPEC,
        SupportedLanguage::Python => &PYTHON_SPEC,
        SupportedLanguage::Go => &GO_SPEC,
        SupportedLanguage::C => &C_SPEC,
        SupportedLanguage::Cpp => &CPP_SPEC,
        SupportedLanguage::Java => &JAVA_SPEC,
        SupportedLanguage::CSharp => &CSHARP_SPEC,
        SupportedLanguage::Ruby => &RUBY_SPEC,
        SupportedLanguage::Php => &PHP_SPEC,
        SupportedLanguage::Swift => &SWIFT_SPEC,
        SupportedLanguage::Elixir => &ELIXIR_SPEC,
        SupportedLanguage::Lua => &LUA_SPEC,
        SupportedLanguage::Bash => &BASH_SPEC,
        SupportedLanguage::Kotlin => &KOTLIN_SPEC,
        SupportedLanguage::Scala => &SCALA_SPEC,
        SupportedLanguage::Zig => &ZIG_SPEC,
        SupportedLanguage::Dart => &DART_SPEC,
        SupportedLanguage::Sql => &SQL_SPEC,
        SupportedLanguage::Yaml => &YAML_SPEC,
        SupportedLanguage::Dockerfile => &DOCKERFILE_SPEC,
        SupportedLanguage::Proto => &PROTO_SPEC,
        SupportedLanguage::Solidity => &SOLIDITY_SPEC,
        SupportedLanguage::Html => &HTML_SPEC,
        SupportedLanguage::Css => &CSS_SPEC,
        SupportedLanguage::Json => &JSON_SPEC,
        SupportedLanguage::Toml => &TOML_SPEC,
        SupportedLanguage::Ocaml => &OCAML_SPEC,
        SupportedLanguage::Haskell => &HASKELL_SPEC,
        SupportedLanguage::Cmake => &CMAKE_SPEC,
        SupportedLanguage::Make => &MAKE_SPEC,
        SupportedLanguage::Julia => &JULIA_SPEC,
        SupportedLanguage::Graphql => &GRAPHQL_SPEC,
        SupportedLanguage::R => &R_SPEC,
        SupportedLanguage::Hcl => &HCL_SPEC,
        SupportedLanguage::Nix => &NIX_SPEC,
        SupportedLanguage::Cuda => &CUDA_SPEC,
        SupportedLanguage::Verilog => &VERILOG_SPEC,
        SupportedLanguage::Tlaplus => &TLAPLUS_SPEC,
        SupportedLanguage::Starlark => &STARLARK_SPEC,
        SupportedLanguage::Bicep => &BICEP_SPEC,
        SupportedLanguage::Gleam => &GLEAM_SPEC,
        SupportedLanguage::PowerShell => &POWERSHELL_SPEC,
        SupportedLanguage::D => &D_SPEC,
        SupportedLanguage::Wgsl => &WGSL_SPEC,
    }
}

static RUST_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Rust,
    function_node_kinds: &["function_item"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["struct_item"],
    interface_node_kinds: &[],
    trait_node_kinds: &["trait_item"],
    enum_node_kinds: &["enum_item"],
    module_node_kinds: &["mod_item", "impl_item"],
    type_alias_node_kinds: &["type_item"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["line_comment", "block_comment"],
    callable_node_kinds: &["function_item"],
    import_node_kinds: &["use_declaration"],
    call_node_kinds: &["call_expression", "method_call_expression"],
};

static TS_JS_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::TypeScript,
    function_node_kinds: &["function_declaration", "function"],
    method_node_kinds: &["method_definition"],
    class_node_kinds: &["class_declaration", "class"],
    struct_node_kinds: &[],
    interface_node_kinds: &["interface_declaration"],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_declaration"],
    module_node_kinds: &["module", "internal_module"],
    type_alias_node_kinds: &["type_alias_declaration"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &[
        "function_declaration",
        "method_definition",
        "function",
        "arrow_function",
    ],
    import_node_kinds: &["import_statement"],
    call_node_kinds: &["call_expression", "new_expression"],
};

static TSX_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Tsx,
    function_node_kinds: &["function_declaration", "function"],
    method_node_kinds: &["method_definition"],
    class_node_kinds: &["class_declaration", "class"],
    struct_node_kinds: &[],
    interface_node_kinds: &["interface_declaration"],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_declaration"],
    module_node_kinds: &["module", "internal_module"],
    type_alias_node_kinds: &["type_alias_declaration"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &[
        "function_declaration",
        "method_definition",
        "function",
        "arrow_function",
    ],
    import_node_kinds: &["import_statement"],
    call_node_kinds: &["call_expression", "new_expression"],
};

static PYTHON_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Python,
    function_node_kinds: &["function_definition"],
    method_node_kinds: &[],
    class_node_kinds: &["class_definition"],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_definition"],
    import_node_kinds: &["import_statement", "import_from_statement"],
    call_node_kinds: &["call"],
};

static GO_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Go,
    function_node_kinds: &["function_declaration"],
    method_node_kinds: &["method_declaration"],
    class_node_kinds: &[],
    struct_node_kinds: &["type_declaration", "type_spec"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_declaration", "method_declaration"],
    import_node_kinds: &["import_declaration", "import_spec"],
    call_node_kinds: &["call_expression"],
};

static C_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::C,
    function_node_kinds: &["function_definition"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["struct_specifier"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_specifier"],
    module_node_kinds: &[],
    type_alias_node_kinds: &["type_definition"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_definition"],
    import_node_kinds: &["preproc_include"],
    call_node_kinds: &["call_expression"],
};

static CPP_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Cpp,
    function_node_kinds: &["function_definition"],
    method_node_kinds: &[],
    class_node_kinds: &["class_specifier"],
    struct_node_kinds: &["struct_specifier"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_specifier"],
    module_node_kinds: &["namespace_definition"],
    type_alias_node_kinds: &["type_definition", "alias_declaration"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_definition"],
    import_node_kinds: &["preproc_include"],
    call_node_kinds: &["call_expression"],
};

static JAVA_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Java,
    function_node_kinds: &["method_declaration", "constructor_declaration"],
    method_node_kinds: &[],
    class_node_kinds: &["class_declaration"],
    struct_node_kinds: &["record_declaration"],
    interface_node_kinds: &["interface_declaration"],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_declaration"],
    module_node_kinds: &["package_declaration"],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["line_comment", "block_comment"],
    callable_node_kinds: &["method_declaration", "constructor_declaration"],
    import_node_kinds: &["import_declaration"],
    call_node_kinds: &["method_invocation"],
};

static CSHARP_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::CSharp,
    function_node_kinds: &["method_declaration", "constructor_declaration"],
    method_node_kinds: &[],
    class_node_kinds: &["class_declaration"],
    struct_node_kinds: &["struct_declaration", "record_declaration"],
    interface_node_kinds: &["interface_declaration"],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_declaration"],
    module_node_kinds: &["namespace_declaration"],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["method_declaration", "constructor_declaration"],
    import_node_kinds: &["using_directive"],
    call_node_kinds: &["invocation_expression"],
};

static RUBY_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Ruby,
    function_node_kinds: &[],
    method_node_kinds: &["method", "singleton_method"],
    class_node_kinds: &["class"],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &["module"],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["method", "singleton_method"],
    import_node_kinds: &["call"],
    call_node_kinds: &["call", "method_call"],
};

static PHP_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Php,
    function_node_kinds: &["function_definition"],
    method_node_kinds: &["method_declaration"],
    class_node_kinds: &["class_declaration"],
    struct_node_kinds: &[],
    interface_node_kinds: &["interface_declaration"],
    trait_node_kinds: &["trait_declaration"],
    enum_node_kinds: &["enum_declaration"],
    module_node_kinds: &["namespace_definition"],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_definition", "method_declaration"],
    import_node_kinds: &["namespace_use_declaration"],
    call_node_kinds: &["function_call_expression", "member_call_expression"],
};

static SWIFT_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Swift,
    function_node_kinds: &["function_declaration"],
    method_node_kinds: &[],
    class_node_kinds: &["class_declaration"],
    struct_node_kinds: &["struct_declaration"],
    interface_node_kinds: &["protocol_declaration"],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_declaration"],
    module_node_kinds: &[],
    type_alias_node_kinds: &["typealias_declaration"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_declaration", "init_declaration"],
    import_node_kinds: &["import_declaration"],
    call_node_kinds: &["call_expression"],
};

static ELIXIR_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Elixir,
    function_node_kinds: &["call"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: None,
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["call"],
    import_node_kinds: &["call"],
    call_node_kinds: &["call"],
};

static LUA_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Lua,
    function_node_kinds: &["function_declaration", "local_function"],
    method_node_kinds: &["function_definition"],
    class_node_kinds: &[],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "--",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_declaration", "local_function"],
    import_node_kinds: &["function_call"],
    call_node_kinds: &["function_call"],
};

static BASH_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Bash,
    function_node_kinds: &["function_definition"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_definition"],
    import_node_kinds: &["command"],
    call_node_kinds: &["command"],
};

// -----------------------------------------------------------------------------
// Expanded Grammars
// -----------------------------------------------------------------------------

static KOTLIN_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Kotlin,
    function_node_kinds: &["function_declaration"],
    method_node_kinds: &[],
    class_node_kinds: &["class_declaration"],
    struct_node_kinds: &["object_declaration"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_entry"],
    module_node_kinds: &["package_header"],
    type_alias_node_kinds: &["type_alias"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["line_comment", "multiline_comment"],
    callable_node_kinds: &["function_declaration", "secondary_constructor"],
    import_node_kinds: &["import_header"],
    call_node_kinds: &["call_expression"],
};

static SCALA_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Scala,
    function_node_kinds: &["function_definition", "function_declaration"],
    method_node_kinds: &[],
    class_node_kinds: &["class_definition"],
    struct_node_kinds: &["object_definition"],
    interface_node_kinds: &[],
    trait_node_kinds: &["trait_definition"],
    enum_node_kinds: &["enum_definition"],
    module_node_kinds: &["package_clause"],
    type_alias_node_kinds: &["type_definition"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_definition"],
    import_node_kinds: &["import_declaration"],
    call_node_kinds: &["call_expression"],
};

static ZIG_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Zig,
    function_node_kinds: &["fn_proto", "fn_decl"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["container_decl"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["line_comment", "doc_comment"],
    callable_node_kinds: &["fn_proto", "fn_decl"],
    import_node_kinds: &["builtin_call"],
    call_node_kinds: &["call_expression"],
};

static DART_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Dart,
    function_node_kinds: &["function_signature", "method_signature"],
    method_node_kinds: &[],
    class_node_kinds: &["class_definition"],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &["mixin_declaration"],
    enum_node_kinds: &["enum_declaration"],
    module_node_kinds: &["library_directive"],
    type_alias_node_kinds: &["type_alias"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_signature", "method_signature"],
    import_node_kinds: &["import_or_export"],
    call_node_kinds: &["method_invocation", "function_expression_invocation"],
};

static SQL_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Sql,
    function_node_kinds: &["create_function_statement", "create_procedure_statement"],
    method_node_kinds: &[],
    class_node_kinds: &[
        "create_table_statement",
        "create_view_statement",
        "create_table",
        "create_view",
    ],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &["create_schema_statement"],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "--",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["create_function_statement", "create_procedure_statement"],
    import_node_kinds: &[],
    call_node_kinds: &["function_call"],
};

static YAML_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Yaml,
    function_node_kinds: &[],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["block_mapping_pair"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &["document"],
    type_alias_node_kinds: &[],
    name_field: Some("key"),
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["block_mapping_pair"],
    import_node_kinds: &[],
    call_node_kinds: &[],
};

static DOCKERFILE_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Dockerfile,
    function_node_kinds: &[
        "instruction",
        "run_instruction",
        "cmd_instruction",
        "entrypoint_instruction",
    ],
    method_node_kinds: &[],
    class_node_kinds: &["from_instruction"],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: None,
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["from_instruction"],
    import_node_kinds: &["from_instruction"],
    call_node_kinds: &["run_instruction"],
};

static PROTO_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Proto,
    function_node_kinds: &["rpc"],
    method_node_kinds: &[],
    class_node_kinds: &["message", "service"],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum"],
    module_node_kinds: &["package"],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["rpc"],
    import_node_kinds: &["import"],
    call_node_kinds: &["rpc"],
};

static SOLIDITY_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Solidity,
    function_node_kinds: &["function_definition"],
    method_node_kinds: &[],
    class_node_kinds: &["contract_declaration", "interface_declaration", "library_declaration"],
    struct_node_kinds: &["struct_declaration"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_declaration"],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_definition"],
    import_node_kinds: &["import_directive"],
    call_node_kinds: &["function_call"],
};

static HTML_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Html,
    function_node_kinds: &[],
    method_node_kinds: &[],
    class_node_kinds: &["element", "script_element", "style_element"],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("tag_name"),
    comment_prefix: "<!--",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["element"],
    import_node_kinds: &[],
    call_node_kinds: &[],
};

static CSS_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Css,
    function_node_kinds: &[],
    method_node_kinds: &[],
    class_node_kinds: &["rule_set", "media_statement", "keyframes_statement"],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: None,
    comment_prefix: "/*",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["rule_set"],
    import_node_kinds: &["import_statement"],
    call_node_kinds: &["call_expression"],
};

static JSON_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Json,
    function_node_kinds: &[],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["pair", "object"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("key"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["pair"],
    import_node_kinds: &[],
    call_node_kinds: &[],
};

static TOML_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Toml,
    function_node_kinds: &[],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["table", "pair"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("key"),
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["table"],
    import_node_kinds: &[],
    call_node_kinds: &[],
};

static OCAML_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Ocaml,
    function_node_kinds: &["let_binding", "value_definition"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &["module_definition", "module_binding"],
    type_alias_node_kinds: &["type_definition"],
    name_field: Some("name"),
    comment_prefix: "(*",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["let_binding", "value_definition"],
    import_node_kinds: &["open_statement"],
    call_node_kinds: &["application_expression"],
};

static HASKELL_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Haskell,
    function_node_kinds: &["function", "bind", "signature"],
    method_node_kinds: &[],
    class_node_kinds: &["class_declaration"],
    struct_node_kinds: &["data_declaration", "newtype_declaration"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &["module_declaration"],
    type_alias_node_kinds: &["type_synonym"],
    name_field: Some("name"),
    comment_prefix: "--",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function", "bind"],
    import_node_kinds: &["import"],
    call_node_kinds: &["apply"],
};

static CMAKE_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Cmake,
    function_node_kinds: &["function_def", "macro_def"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: None,
    comment_prefix: "#",
    doc_comment_kinds: &["line_comment", "bracket_comment"],
    callable_node_kinds: &["function_def", "macro_def"],
    import_node_kinds: &["normal_command"],
    call_node_kinds: &["normal_command"],
};

static MAKE_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Make,
    function_node_kinds: &[],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["rule"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: None,
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["rule"],
    import_node_kinds: &["include_directive"],
    call_node_kinds: &["recipe"],
};

static JULIA_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Julia,
    function_node_kinds: &["function_definition", "short_function_definition"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["struct_definition"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &["module_definition"],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "#",
    doc_comment_kinds: &["line_comment", "block_comment"],
    callable_node_kinds: &["function_definition", "short_function_definition"],
    import_node_kinds: &["using_statement", "import_statement"],
    call_node_kinds: &["call_expression"],
};

static GRAPHQL_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Graphql,
    function_node_kinds: &["field_definition"],
    method_node_kinds: &[],
    class_node_kinds: &["object_type_definition", "interface_type_definition", "schema_definition"],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_type_definition"],
    module_node_kinds: &[],
    type_alias_node_kinds: &["union_type_definition"],
    name_field: Some("name"),
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["field_definition"],
    import_node_kinds: &[],
    call_node_kinds: &["field"],
};

static R_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::R,
    function_node_kinds: &["function_definition"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_definition"],
    import_node_kinds: &["call"],
    call_node_kinds: &["call"],
};

static HCL_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Hcl,
    function_node_kinds: &["attribute"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["block"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: None,
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["block"],
    import_node_kinds: &[],
    call_node_kinds: &["function_call"],
};

static NIX_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Nix,
    function_node_kinds: &["function_expression", "binding"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["attrset_expression", "rec_attrset_expression"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: None,
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_expression", "binding"],
    import_node_kinds: &[],
    call_node_kinds: &["apply_expression"],
};

static CUDA_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Cuda,
    function_node_kinds: &["function_definition"],
    method_node_kinds: &[],
    class_node_kinds: &["class_specifier"],
    struct_node_kinds: &["struct_specifier"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_specifier"],
    module_node_kinds: &["namespace_definition"],
    type_alias_node_kinds: &["type_definition", "alias_declaration"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_definition"],
    import_node_kinds: &["preproc_include"],
    call_node_kinds: &["call_expression"],
};

static VERILOG_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Verilog,
    function_node_kinds: &["task_declaration", "function_declaration"],
    method_node_kinds: &[],
    class_node_kinds: &["class_declaration"],
    struct_node_kinds: &[],
    interface_node_kinds: &["interface_declaration"],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &["module_declaration"],
    type_alias_node_kinds: &[],
    name_field: None,
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["task_declaration", "function_declaration", "module_declaration"],
    import_node_kinds: &["include_compiler_directive"],
    call_node_kinds: &["system_tf_call", "tf_call"],
};

static TLAPLUS_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Tlaplus,
    function_node_kinds: &["operator_definition", "function_definition"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &["module"],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "\\*",
    doc_comment_kinds: &["comment", "block_comment"],
    callable_node_kinds: &["operator_definition", "function_definition"],
    import_node_kinds: &["extends", "instance"],
    call_node_kinds: &["bound_infix_op"],
};

static STARLARK_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Starlark,
    function_node_kinds: &["function_definition"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_definition"],
    import_node_kinds: &[],
    call_node_kinds: &["call"],
};

static BICEP_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Bicep,
    function_node_kinds: &["parameter_declaration", "variable_declaration", "output_declaration"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["resource_declaration", "module_declaration"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["resource_declaration", "module_declaration"],
    import_node_kinds: &["import_declaration"],
    call_node_kinds: &["function_call"],
};

static GLEAM_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Gleam,
    function_node_kinds: &["function"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["type_definition"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &["type_alias"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function"],
    import_node_kinds: &["import"],
    call_node_kinds: &["function_call"],
};

static POWERSHELL_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::PowerShell,
    function_node_kinds: &["function_statement"],
    method_node_kinds: &[],
    class_node_kinds: &["class_statement"],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_statement"],
    module_node_kinds: &[],
    type_alias_node_kinds: &[],
    name_field: Some("name"),
    comment_prefix: "#",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_statement"],
    import_node_kinds: &["using_statement"],
    call_node_kinds: &["command", "invocable_expression"],
};

static D_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::D,
    function_node_kinds: &["function_declaration"],
    method_node_kinds: &[],
    class_node_kinds: &["class_declaration"],
    struct_node_kinds: &["struct_declaration"],
    interface_node_kinds: &["interface_declaration"],
    trait_node_kinds: &[],
    enum_node_kinds: &["enum_declaration"],
    module_node_kinds: &["module_declaration"],
    type_alias_node_kinds: &["alias_declaration"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_declaration"],
    import_node_kinds: &["import_declaration"],
    call_node_kinds: &["call_expression"],
};

static WGSL_SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::Wgsl,
    function_node_kinds: &["function_declaration"],
    method_node_kinds: &[],
    class_node_kinds: &[],
    struct_node_kinds: &["struct_declaration"],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[],
    type_alias_node_kinds: &["type_alias_declaration"],
    name_field: Some("name"),
    comment_prefix: "//",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &["function_declaration"],
    import_node_kinds: &["enable_directive"],
    call_node_kinds: &["call_expression"],
};
