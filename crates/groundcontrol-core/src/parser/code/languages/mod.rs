//! Declarative polyglot language catalog.
//!
//! Provides a self-contained language module architecture where adding a new
//! language involves creating a single submodule folder (`languages/<lang>/`) with its
//! Tree-sitter grammar, extensions, query packs (`tags.scm`, `locals.scm`, `routes.scm`),
//! and AST node specification, and adding a 1-line tuple to the registry macro below.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub mod spec;
pub use spec::{LanguageDefinition, LanguageSpec};

macro_rules! define_languages {
    ($( ($variant:ident, $module:ident) ),* $(,)?) => {
        $( pub mod $module; )*

        /// Supported polyglot programming and configuration languages.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "lowercase")]
        pub enum SupportedLanguage {
            $(
                #[doc = concat!("Support for ", stringify!($variant))]
                $variant,
            )*
        }

        impl SupportedLanguage {
            /// Slice of all supported language variants.
            pub const ALL: &'static [SupportedLanguage] = &[
                $( SupportedLanguage::$variant, )*
            ];
        }

        /// Static registry of all supported language definitions.
        pub static ALL_DEFINITIONS: &[&'static LanguageDefinition] = &[
            $( &$module::DEFINITION, )*
        ];
    };
}

define_languages![
    (Rust, rust),
    (TypeScript, typescript),
    (Tsx, tsx),
    (JavaScript, javascript),
    (Python, python),
    (Go, go),
    (C, c),
    (Cpp, cpp),
    (Java, java),
    (CSharp, csharp),
    (Ruby, ruby),
    (Php, php),
    (Swift, swift),
    (Elixir, elixir),
    (Lua, lua),
    (Bash, bash),
    (Kotlin, kotlin),
    (Scala, scala),
    (Zig, zig),
    (Dart, dart),
    (Sql, sql),
    (Yaml, yaml),
    (Dockerfile, dockerfile),
    (Proto, proto),
    (Solidity, solidity),
    (Html, html),
    (Css, css),
    (Json, json),
    (Toml, toml),
    (Ocaml, ocaml),
    (Haskell, haskell),
    (Cmake, cmake),
    (Make, make),
    (Julia, julia),
    (Graphql, graphql),
    (R, r),
    (Hcl, hcl),
    (Nix, nix),
    (Cuda, cuda),
    (Verilog, verilog),
    (Tlaplus, tlaplus),
    (Starlark, starlark),
    (Bicep, bicep),
    (Gleam, gleam),
    (PowerShell, powershell),
    (D, d),
    (Wgsl, wgsl),
    (Erlang, erlang),
    (Xml, xml),
    (Vb6, vb6),
    (PlSql, plsql),
    (Cobol, cobol),
];

impl SupportedLanguage {
    /// Return the language definition for this variant.
    #[inline]
    pub fn definition(&self) -> &'static LanguageDefinition {
        ALL_DEFINITIONS[*self as usize]
    }

    /// Canonical language slug.
    #[inline]
    pub fn name(&self) -> &'static str {
        self.definition().name
    }

    /// File extensions associated with this language.
    #[inline]
    pub fn extensions(&self) -> &'static [&'static str] {
        self.definition().extensions
    }

    /// Exact filenames associated with this language.
    #[inline]
    pub fn filenames(&self) -> &'static [&'static str] {
        self.definition().filenames
    }

    /// Instantiated Tree-sitter grammar.
    #[inline]
    pub fn grammar(&self) -> tree_sitter::Language {
        (self.definition().grammar)()
    }

    /// Instantiated Tree-sitter grammar (alias for `grammar()`).
    #[inline]
    pub fn tree_sitter_language(&self) -> tree_sitter::Language {
        self.grammar()
    }

    /// Declarative structural AST specification.
    #[inline]
    pub fn spec(&self) -> &'static LanguageSpec {
        self.definition().spec
    }

    /// Combined Tree-sitter query pack source.
    #[inline]
    pub fn query_source(&self) -> String {
        self.definition().combined_query_source()
    }

    /// Single-line comment prefix for scope breadcrumbs.
    #[inline]
    pub fn comment_prefix(&self) -> &'static str {
        self.spec().comment_prefix
    }
}

impl fmt::Display for SupportedLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl FromStr for SupportedLanguage {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_ascii_lowercase();
        for def in ALL_DEFINITIONS {
            if def.name.eq_ignore_ascii_case(&lower) {
                return Ok(def.language);
            }
        }
        Err(())
    }
}

/// Retrieve the language definition for a given supported language.
#[inline]
pub fn get_language_definition(lang: SupportedLanguage) -> &'static LanguageDefinition {
    lang.definition()
}

/// Retrieve the declarative specification for a given supported language.
#[inline]
pub fn get_language_spec(lang: SupportedLanguage) -> &'static LanguageSpec {
    lang.spec()
}

/// Declarative dialect disambiguation rules when file extensions overlap.
///
/// Maps (extension, language, heuristic_predicate).
pub static DIALECT_DISAMBIGUATORS: &[(&str, SupportedLanguage, fn(&[u8]) -> bool)] = &[
    ("sql", SupportedLanguage::PlSql, plsql::is_plsql_dialect),
];

/// Detect programming or configuration language from a file path and optional sample content.
pub fn detect_language_with_content(path: &Path, content: Option<&[u8]>) -> Option<SupportedLanguage> {
    if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
        let lower = filename.to_ascii_lowercase();
        for def in ALL_DEFINITIONS {
            if def.filenames.contains(&lower.as_str()) {
                return Some(def.language);
            }
        }
    }

    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if let Some(bytes) = content {
        for (target_ext, lang, disambiguate) in DIALECT_DISAMBIGUATORS {
            if *target_ext == ext && disambiguate(bytes) {
                return Some(*lang);
            }
        }
    }

    for def in ALL_DEFINITIONS {
        if def.extensions.contains(&ext.as_str()) {
            return Some(def.language);
        }
    }

    None
}

/// Detect programming or configuration language from a file path extension or exact filename.
pub fn detect_language(path: &Path) -> Option<SupportedLanguage> {
    detect_language_with_content(path, None)
}

/// Check if a file path belongs to any supported code or configuration language.
#[inline]
pub fn is_code_file(path: &Path) -> bool {
    detect_language(path).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_definitions_indexed_correctly() {
        for (idx, def) in ALL_DEFINITIONS.iter().enumerate() {
            assert_eq!(def.language as usize, idx);
            assert_eq!(def.language.definition().name, def.name);
        }
    }

    #[test]
    fn test_config_specs_do_not_classify_pairs_as_structs() {
        let json_spec = get_language_spec(SupportedLanguage::Json);
        assert!(!json_spec.struct_node_kinds.contains(&"pair"));
        assert!(!json_spec.struct_node_kinds.contains(&"object"));
        assert!(!json_spec.callable_node_kinds.contains(&"pair"));

        let toml_spec = get_language_spec(SupportedLanguage::Toml);
        assert!(!toml_spec.struct_node_kinds.contains(&"pair"));
        assert!(toml_spec.struct_node_kinds.contains(&"table"));

        let yaml_spec = get_language_spec(SupportedLanguage::Yaml);
        assert!(!yaml_spec.struct_node_kinds.contains(&"block_mapping_pair"));
    }
}
