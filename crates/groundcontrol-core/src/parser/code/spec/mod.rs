//! Declarative language specifications for polyglot AST code extraction.
//!
//! Maps Tree-sitter AST node kinds to [`CodeSymbolType`] and semantic roles across 46 supported languages.

pub mod configs;
pub mod functional;
pub mod managed;
pub mod systems;
pub mod types;

pub use types::LanguageSpec;

use crate::parser::code::languages::SupportedLanguage;
use configs::*;
use functional::*;
use managed::*;
use systems::*;

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
