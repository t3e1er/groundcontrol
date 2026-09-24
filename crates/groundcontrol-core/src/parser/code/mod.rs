//! Polyglot source code parsing, Tree-sitter AST extraction, and structural chunking (`cAST`).

pub mod chunker;
pub mod grammar;
pub mod languages;
pub mod patterns;
pub mod scope;
pub mod spec;

pub use chunker::{CodeChunker, CodeParseResult};
pub use grammar::{
    AstGrammarExtractor, DataFlowPath, DataFlowSink, ExtractedGrammarSemantics,
    GenericAstGrammarExtractor, GrammarTransition, WeightedToken,
};
pub use languages::{detect_language, is_code_file, SupportedLanguage};
pub use patterns::{extract_semantic_tokens, split_identifier};
pub use scope::{normalize_scope_path, scope_matches};
pub use spec::{get_language_spec, LanguageSpec};
