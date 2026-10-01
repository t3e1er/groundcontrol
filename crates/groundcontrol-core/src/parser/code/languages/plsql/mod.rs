//! Oracle PL/SQL language support.

use super::spec::{LanguageDefinition, LanguageSpec};
use super::SupportedLanguage;

/// Declarative definition for PL/SQL language support.
pub static DEFINITION: LanguageDefinition = LanguageDefinition {
    language: SupportedLanguage::PlSql,
    name: "plsql",
    extensions: &[
        "pks", "pkb", "pls", "plsql", "fnc", "prc", "trg", "tps", "tpb", "pck", "bdy",
    ],
    filenames: &[],
    grammar: || tree_sitter_plsql_sqry::language(),
    tags_query: include_str!("tags.scm"),
    locals_query: include_str!("locals.scm"),
    route_overlay: include_str!("routes.scm"),
    spec: &SPEC,
};

/// Declarative AST specification for PL/SQL language support.
pub static SPEC: LanguageSpec = LanguageSpec {
    language: SupportedLanguage::PlSql,
    function_node_kinds: &[
        "function_definition",
        "procedure_definition",
        "procedure_declaration",
        "function_declaration",
        "create_procedure",
        "create_function",
        "create_trigger",
    ],
    method_node_kinds: &[],
    class_node_kinds: &["create_type", "create_type_body", "type_spec"],
    struct_node_kinds: &[],
    interface_node_kinds: &[],
    trait_node_kinds: &[],
    enum_node_kinds: &[],
    module_node_kinds: &[
        "create_package",
        "create_package_body",
        "package_spec",
        "package_body",
        "package_definition",
    ],
    type_alias_node_kinds: &[],
    name_field: None,
    comment_prefix: "--",
    doc_comment_kinds: &["comment"],
    callable_node_kinds: &[
        "function_definition",
        "procedure_definition",
        "procedure_declaration",
        "function_declaration",
        "create_procedure",
        "create_function",
        "create_trigger",
    ],
    import_node_kinds: &[],
    call_node_kinds: &["ref_call"],
};

/// Heuristic check for Oracle PL/SQL dialect keywords in shared `.sql` file contents.
pub fn is_plsql_dialect(bytes: &[u8]) -> bool {
    let check_len = bytes.len().min(4096);
    let sample = &bytes[..check_len];
    let sample_upper = sample.to_ascii_uppercase();
    let hay = match std::str::from_utf8(&sample_upper) {
        Ok(s) => s,
        Err(_) => return false,
    };
    hay.contains("PACKAGE BODY")
        || hay.contains("CREATE OR REPLACE PACKAGE")
        || hay.contains("CREATE PACKAGE")
        || hay.contains("PRAGMA AUTONOMOUS_TRANSACTION")
        || hay.contains("CREATE OR REPLACE TRIGGER")
        || hay.contains("CREATE TRIGGER")
}

