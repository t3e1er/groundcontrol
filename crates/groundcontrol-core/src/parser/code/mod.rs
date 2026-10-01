//! Polyglot source code parsing, Tree-sitter AST extraction, and structural chunking (`cAST`).

pub mod chunker;
pub mod languages;
pub mod manifest;
pub mod query;
pub mod semantics;

// Facade re-exports: Language subsystem
pub use languages::{
    detect_language, detect_language_with_content, get_language_definition, get_language_spec,
    is_code_file, LanguageDefinition, LanguageSpec, SupportedLanguage, ALL_DEFINITIONS,
};

// Backward-compatible module aliases for internal references
pub use languages as definition;
pub use languages::spec;
pub use semantics::grammar;
pub use semantics::patterns;
pub use semantics::scope;

// Facade re-exports: Semantics subsystem
pub use semantics::{
    expand_abbreviation, extract_semantic_tokens, normalize_scope_path, scope_matches,
    split_identifier, AstGrammarExtractor, DataFlowPath, DataFlowSink, ExtractedGrammarSemantics,
    GenericAstGrammarExtractor, GrammarTransition, WeightedToken,
};

// Facade re-exports: Query subsystem
pub use query::{
    get_language_query, ExtractedLocalBinding, ExtractedQueryDefinition, LanguageQuery,
};

// Facade re-exports: Chunker subsystem
pub use chunker::{CodeChunker, CodeParseResult};

// Facade re-exports: Manifest subsystem
pub use manifest::{detect_manifest_in_dir, find_enclosing_manifest, PackageManifest};
