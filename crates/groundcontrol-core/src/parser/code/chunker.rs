//! AST-aware structural code chunker (`cAST` pattern).
//!
//! Decomposes polyglot source code files into syntactically complete AST nodes
//! (functions, methods, classes, traits, structs) with bound docstrings and
//! prepended hierarchical scope breadcrumbs for optimal hybrid retrieval.

use std::path::Path;

use groundcontrol_common::config::ChunkingConfig;
use groundcontrol_common::types::{Chunk, ChunkEmbedPolicy, CodeSymbol, CodeSymbolType};
use tree_sitter::{Node, Parser};

use super::grammar::{AstGrammarExtractor, ExtractedGrammarSemantics, GenericAstGrammarExtractor};
use super::languages::{detect_language, SupportedLanguage};

/// Result of parsing a code file: chunks for embedding/BM25 and extracted code symbols.
#[derive(Debug, Clone)]
pub struct CodeParseResult {
    /// Extracted syntactic code chunks.
    pub chunks: Vec<Chunk>,
    /// Extracted code symbol definitions.
    pub symbols: Vec<CodeSymbol>,
    /// Extracted AST grammar semantics for each symbol.
    pub grammar_semantics: Vec<ExtractedGrammarSemantics>,
}

/// AST-aware code chunker.
pub struct CodeChunker;

impl CodeChunker {
    /// Maximum size of a source file eligible for full Tree-sitter AST parsing (2 MB).
    /// Files exceeding this limit (e.g. 100MB generated C grammars, huge minified bundles)
    /// are skipped to prevent CPU exhaustion on generated data tables.
    pub const MAX_CODE_FILE_SIZE_BYTES: usize = 2 * 1024 * 1024;

    /// Parse and chunk a source code file.
    pub fn parse_and_chunk(
        file_path: &Path,
        content: &str,
        config: &ChunkingConfig,
    ) -> Option<CodeParseResult> {
        if content.len() > Self::MAX_CODE_FILE_SIZE_BYTES {
            tracing::info!(
                "Skipping Tree-sitter AST parsing for oversized file {} ({} bytes > {} max)",
                file_path.display(),
                content.len(),
                Self::MAX_CODE_FILE_SIZE_BYTES
            );
            return None;
        }

        let lang = detect_language(file_path)?;
        let mut parser = Parser::new();
        if let Err(e) = parser.set_language(&lang.tree_sitter_language()) {
            tracing::warn!("Failed to set language for {}: {:?}", file_path.display(), e);
            return None;
        }

        let Some(tree) = parser.parse(content, None) else {
            tracing::warn!("Failed to parse content for {}", file_path.display());
            return None;
        };
        let max_chars = config.max_tokens.max(256) * 4;
        let mut extractor =
            AstExtractor::new(file_path.to_string_lossy().to_string(), content, lang, max_chars);
        extractor.traverse(tree.root_node());

        Some(CodeParseResult {
            chunks: extractor.chunks,
            symbols: extractor.symbols,
            grammar_semantics: extractor.grammar_semantics,
        })
    }
}

struct AstExtractor<'a> {
    file_path: String,
    content: &'a str,
    language: SupportedLanguage,
    max_chars: usize,
    scope_stack: Vec<String>,
    chunks: Vec<Chunk>,
    symbols: Vec<CodeSymbol>,
    grammar_semantics: Vec<ExtractedGrammarSemantics>,
    chunk_index: usize,
    depth: usize,
}

impl<'a> AstExtractor<'a> {
    fn new(
        file_path: String,
        content: &'a str,
        language: SupportedLanguage,
        max_chars: usize,
    ) -> Self {
        Self {
            file_path,
            content,
            language,
            max_chars,
            scope_stack: Vec::new(),
            chunks: Vec::new(),
            symbols: Vec::new(),
            grammar_semantics: Vec::new(),
            chunk_index: 0,
            depth: 0,
        }
    }

    fn current_scope(&self) -> String {
        if self.scope_stack.is_empty() {
            self.file_path.clone()
        } else {
            self.scope_stack.join(" > ")
        }
    }

    const MAX_AST_DEPTH: usize = 256;

    fn traverse(&mut self, node: Node) {
        if self.depth >= Self::MAX_AST_DEPTH {
            return;
        }
        self.depth += 1;
        self.traverse_inner(node);
        self.depth -= 1;
    }

    fn traverse_inner(&mut self, node: Node) {
        let lang = self.language;

        let symbol_info = self.classify_node(node);

        if let Some((sym_type, name, signature)) = symbol_info {
            // Extract standard AST grammar semantics for this symbol
            let spec = crate::parser::code::spec::get_language_spec(lang);
            let grammar_extractor = GenericAstGrammarExtractor::new(spec);
            let sem = grammar_extractor.extract_grammar_semantics(node, self.content);
            self.grammar_semantics.push(sem);

            let start_byte = node.start_byte();
            let end_byte = node.end_byte();
            let start_line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;

            let parent_scope = self.current_scope();
            let full_scope = if parent_scope == self.file_path {
                name.clone()
            } else {
                format!("{parent_scope} > {name}")
            };

            let docstring = self.extract_docstring(node);
            let raw_node_text = &self.content[start_byte..end_byte];

            // Build enriched chunk text with AST scope breadcrumb
            let comment = lang.comment_prefix();
            let breadcrumb = format!(
                "{comment} Scope: {full_scope}\n{comment} Language: {}\n{comment} File: {}\n",
                lang.name(),
                self.file_path
            );

            let chunk_text = if let Some(ref doc) = docstring {
                format!("{breadcrumb}{doc}\n{raw_node_text}")
            } else {
                format!("{breadcrumb}{raw_node_text}")
            };

            let skeleton_text = if let Some(ref doc) = docstring {
                format!("{breadcrumb}{doc}\n{signature}")
            } else {
                format!("{breadcrumb}{signature}")
            };

            // Register symbol definition
            self.symbols.push(CodeSymbol {
                file_path: self.file_path.clone(),
                name: name.clone(),
                scope_path: full_scope.clone(),
                symbol_type: sym_type,
                language: lang.name().to_string(),
                signature,
                docstring: docstring.clone(),
                start_line,
                end_line,
            });

            // If it's a container type (class, struct, trait, impl), push to scope stack and traverse children
            let is_container = crate::parser::code::spec::LanguageSpec::is_container(sym_type);

            // Large container nodes (e.g. large impl blocks) have their child functions emitted separately.
            // For the container itself, emit header up to max_chars to describe the container.
            let emit_text = if is_container && raw_node_text.len() > self.max_chars {
                let mut truncated = raw_node_text;
                if let Some((idx, _)) =
                    truncated.char_indices().nth(self.max_chars.saturating_sub(breadcrumb.len()))
                {
                    truncated = &truncated[..idx];
                }
                format!("{breadcrumb}{truncated}")
            } else if chunk_text.len() > self.max_chars {
                let mut truncated = &chunk_text[..];
                if let Some((idx, _)) = truncated.char_indices().nth(self.max_chars) {
                    truncated = &truncated[..idx];
                }
                truncated.to_string()
            } else {
                chunk_text
            };

            // Extract AST pattern tokens & normalized identifier expansions (Pillar 1)
            let sem_tokens =
                super::patterns::extract_semantic_tokens(raw_node_text, &name, &full_scope);
            let final_emit_text = if sem_tokens.is_empty() {
                emit_text
            } else {
                format!("{emit_text}\n// Semantic tokens: {}", sem_tokens.join(" "))
            };

            // Register AST chunk
            let embed_policy =
                classify_embed_policy(sym_type, raw_node_text, lang, &self.file_path);
            let chunk = Chunk::new(
                &self.file_path,
                self.chunk_index,
                final_emit_text,
                start_byte,
                end_byte,
            )
            .with_code_metadata(lang.name(), &full_scope, start_line, end_line)
            .with_embed_policy(embed_policy)
            .with_skeleton_text(skeleton_text);
            self.chunks.push(chunk);
            self.chunk_index += 1;

            if is_container {
                self.scope_stack.push(name);
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.traverse(child);
                }
                self.scope_stack.pop();
                return;
            }
        }

        // Default: traverse all children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.traverse(child);
        }
    }

    fn extract_docstring(&self, node: Node) -> Option<String> {
        // 1. Python body docstring
        if self.language == SupportedLanguage::Python {
            if let Some(body) = node.child_by_field_name("body") {
                if let Some(first_stmt) = body.child(0) {
                    if first_stmt.kind() == "expression_statement" {
                        let text = self.node_text(first_stmt).trim();
                        if (text.starts_with("\"\"\"") && text.ends_with("\"\"\""))
                            || (text.starts_with("'''") && text.ends_with("'''"))
                        {
                            return Some(text.to_string());
                        }
                    }
                }
            }
        }

        // 2. Check preceding siblings of this node or its parent (e.g. export_statement)
        let mut target_node = node;
        if let Some(parent) = node.parent() {
            if parent.kind() == "export_statement" {
                target_node = parent;
            }
        }

        let mut prev = target_node.prev_sibling();
        let mut comments = Vec::new();

        while let Some(sibling) = prev {
            let kind = sibling.kind();
            if kind.contains("comment") || kind == "line_comment" || kind == "block_comment" {
                let text = self.node_text(sibling);
                comments.push(text.trim().to_string());
                prev = sibling.prev_sibling();
            } else {
                break;
            }
        }

        // 3. Also check child comments if grammar places comments inside node
        if comments.is_empty() {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let kind = child.kind();
                if kind.contains("comment") || kind == "line_comment" || kind == "block_comment" {
                    comments.push(self.node_text(child).trim().to_string());
                } else if !kind.is_empty() && kind != "decorator" && kind != "attribute_item" {
                    break;
                }
            }
        }

        if comments.is_empty() {
            None
        } else {
            comments.reverse();
            Some(comments.join("\n"))
        }
    }

    fn node_text(&self, node: Node) -> &str {
        &self.content[node.start_byte()..node.end_byte()]
    }

    fn find_child_identifier(&self, node: Node) -> Option<String> {
        if let Some(n) = node.child_by_field_name("name") {
            return Some(self.node_text(n).to_string());
        }
        if let Some(d) = node.child_by_field_name("declarator") {
            if let Some(n) = d.child_by_field_name("declarator") {
                return Some(self.node_text(n).to_string());
            }
            let mut cursor = d.walk();
            for child in d.children(&mut cursor) {
                let kind = child.kind();
                if kind == "identifier" || kind == "type_identifier" || kind == "field_identifier" {
                    return Some(self.node_text(child).to_string());
                }
            }
            return Some(self.node_text(d).to_string());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind == "identifier"
                || kind == "type_identifier"
                || kind == "property_identifier"
                || kind == "field_identifier"
                || kind == "simple_identifier"
                || kind == "function_name"
                || kind == "type_name"
            {
                return Some(self.node_text(child).to_string());
            }
        }
        // Check 1 level down for header/wrapper nodes (e.g. Verilog module_header)
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let mut sub_cursor = child.walk();
            for grandchild in child.children(&mut sub_cursor) {
                let kind = grandchild.kind();
                if kind == "identifier"
                    || kind == "type_identifier"
                    || kind == "property_identifier"
                    || kind == "field_identifier"
                    || kind == "simple_identifier"
                    || kind == "function_name"
                    || kind == "type_name"
                {
                    return Some(self.node_text(grandchild).to_string());
                }
            }
        }
        None
    }

    fn classify_node(&self, node: Node) -> Option<(CodeSymbolType, String, String)> {
        let lang = self.language;
        let spec = crate::parser::code::spec::get_language_spec(lang);

        // Rust custom constructs
        if lang == SupportedLanguage::Rust {
            if node.kind() == "impl_item" {
                let type_name = if let Some(n) = node.child_by_field_name("type") {
                    self.node_text(n).to_string()
                } else {
                    let mut found = None;
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        let kind = child.kind();
                        if kind == "type_identifier"
                            || kind == "generic_type"
                            || kind == "primitive_type"
                            || kind == "scoped_type_identifier"
                        {
                            found = Some(self.node_text(child).to_string());
                        }
                    }
                    found.unwrap_or_else(|| "impl".to_string())
                };

                let trait_name = node
                    .child_by_field_name("trait")
                    .map(|n| format!("{} for ", self.node_text(n)));
                let full_name = format!("{}{}", trait_name.unwrap_or_default(), type_name);
                let sig = self.extract_first_line(node);
                return Some((CodeSymbolType::Module, full_name, sig));
            }

            if node.kind() == "mod_item" {
                let name = self.find_child_identifier(node)?;
                let sig = format!("mod {name}");
                return Some((CodeSymbolType::Module, name, sig));
            }
        }

        // Elixir custom constructs
        if lang == SupportedLanguage::Elixir && node.kind() == "call" {
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            if let Some(target) = children.first() {
                let target_name = self.node_text(*target).trim();
                if target_name == "defmodule" {
                    let name = children
                        .get(1)
                        .map(|n| {
                            let text = self.node_text(*n).trim();
                            text.split_whitespace().next().unwrap_or(text).to_string()
                        })
                        .unwrap_or_else(|| "Module".to_string());
                    let sig = self.extract_first_line(node);
                    return Some((CodeSymbolType::Module, name, sig));
                } else if target_name == "def" || target_name == "defp" || target_name == "defmacro"
                {
                    let name = children
                        .get(1)
                        .map(|n| {
                            let t = self.node_text(*n).trim();
                            t.split('(').next().unwrap_or(t).trim().to_string()
                        })
                        .unwrap_or_else(|| "function".to_string());
                    let sig = self.extract_first_line(node);
                    return Some((CodeSymbolType::Function, name, sig));
                } else if target_name == "defprotocol" || target_name == "defimpl" {
                    let name = children
                        .get(1)
                        .map(|n| self.node_text(*n).trim().to_string())
                        .unwrap_or_else(|| "Protocol".to_string());
                    let sig = self.extract_first_line(node);
                    return Some((CodeSymbolType::Trait, name, sig));
                }
            }
            return None;
        }

        // HCL custom block handling: capture block type and labels (e.g. `aws_s3_bucket.b` or `module.vpc`)
        if lang == SupportedLanguage::Hcl && node.kind() == "block" {
            let mut cursor = node.walk();
            let mut block_type = String::new();
            let mut labels = Vec::new();

            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" && block_type.is_empty() {
                    block_type = self.node_text(child).to_string();
                } else if child.kind() == "string_lit" {
                    let text = self.node_text(child).trim_matches('"').to_string();
                    labels.push(text);
                }
            }

            let name = match labels.len() {
                2 => format!("{}.{}", labels[0], labels[1]),
                1 => {
                    if block_type.is_empty() || block_type == "resource" || block_type == "data" {
                        labels[0].clone()
                    } else {
                        format!("{}.{}", block_type, labels[0])
                    }
                }
                _ => {
                    if !block_type.is_empty() {
                        block_type
                    } else {
                        self.find_child_identifier(node).unwrap_or_else(|| "block".to_string())
                    }
                }
            };

            let sig = self.extract_first_line(node);
            return Some((CodeSymbolType::Struct, name, sig));
        }

        let mut sym_type = spec.classify_symbol(node)?;
        if lang == SupportedLanguage::Python
            && sym_type == CodeSymbolType::Function
            && !self.scope_stack.is_empty()
        {
            sym_type = CodeSymbolType::Method;
        }

        let name = if node.kind() == "init_declaration" {
            "init".to_string()
        } else if let Some(field) = spec.name_field {
            node.child_by_field_name(field)
                .map(|n| self.node_text(n).to_string())
                .or_else(|| self.find_child_identifier(node))?
        } else {
            self.find_child_identifier(node)?
        };

        let sig = self.extract_first_line(node);
        Some((sym_type, name, sig))
    }

    fn extract_first_line(&self, node: Node) -> String {
        let text = self.node_text(node);
        text.lines().next().unwrap_or(text).trim().trim_end_matches('{').trim().to_string()
    }
}

/// Classify whether an AST code symbol should be a semantic anchor (embedded)
/// or graph-only (BM25 + AST graph only, no dense vector embedding).
///
/// Under Option 2 Anchor Embedding:
/// - Semantic Anchors (dense vector forward pass):
///   - Top-level domain type definitions (Struct, Class, Trait, Interface, Enum).
///   - Public modules (`pub mod`).
///   - Exported / documented public APIs (`pub fn` with doc comments, or top-level public functions).
/// - Graph-Only (indexed via BM25 exact term matching + Petgraph AST relationships, zero neural forward pass):
///   - Test files (`/tests/`, `tests.rs`, `_test.go`, etc.) and test functions (`#[test]`, `Test*`).
///   - `impl` blocks (`impl Trait for Type`, `impl Type`) — the struct and methods are already connected.
///   - Private/internal functions, leaf expressions, and undocumented internal methods.
pub fn classify_embed_policy(
    sym_type: CodeSymbolType,
    node_text: &str,
    lang: SupportedLanguage,
    file_path: &str,
) -> ChunkEmbedPolicy {
    // 1. Exclude test files entirely from anchor embeddings
    let norm_path = file_path.replace('\\', "/");
    if norm_path.starts_with("tests/")
        || norm_path.starts_with("test/")
        || norm_path.contains("/tests/")
        || norm_path.contains("/test/")
        || norm_path.ends_with("tests.rs")
        || norm_path.ends_with("_test.rs")
        || norm_path.ends_with("_test.go")
        || norm_path.contains(".test.")
        || norm_path.contains(".spec.")
    {
        return ChunkEmbedPolicy::GraphOnly;
    }

    let trimmed = node_text.trim();

    // 2. Exclude unit/integration test symbols
    if trimmed.contains("#[test]") || trimmed.contains("#[cfg(test)]") {
        return ChunkEmbedPolicy::GraphOnly;
    }

    // 3. Exclude `impl` blocks from anchor embeddings (they are implementation scopes, not top-level API definitions)
    if trimmed.starts_with("impl ") || trimmed.starts_with("impl<") {
        return ChunkEmbedPolicy::GraphOnly;
    }

    // 4. Core domain containers (Struct, Class, Trait, Interface, Enum) are always anchors
    match sym_type {
        CodeSymbolType::Class
        | CodeSymbolType::Struct
        | CodeSymbolType::Trait
        | CodeSymbolType::Interface
        | CodeSymbolType::Enum => return ChunkEmbedPolicy::Anchor,
        CodeSymbolType::Module => {
            // Only public modules are anchors
            if lang == SupportedLanguage::Rust {
                if trimmed.starts_with("pub mod")
                    || trimmed.starts_with("pub(crate) mod")
                    || trimmed.contains("pub mod")
                {
                    return ChunkEmbedPolicy::Anchor;
                } else {
                    return ChunkEmbedPolicy::GraphOnly;
                }
            } else {
                return ChunkEmbedPolicy::Anchor;
            }
        }
        _ => {}
    }

    // 5. Only functions and methods can be anchors if public/exported
    if !matches!(sym_type, CodeSymbolType::Function | CodeSymbolType::Method) {
        return ChunkEmbedPolicy::GraphOnly;
    }

    match lang {
        SupportedLanguage::Rust => {
            let is_pub = trimmed
                .lines()
                .map(|l| l.trim())
                .filter(|l| {
                    !l.is_empty()
                        && !l.starts_with("#[")
                        && !l.starts_with("//")
                        && !l.starts_with("/*")
                        && !l.starts_with('*')
                })
                .next()
                .map(|l| {
                    l.starts_with("pub fn")
                        || l.starts_with("pub(crate) fn")
                        || l.starts_with("pub ")
                        || l.starts_with("pub(")
                })
                .unwrap_or(false);

            if is_pub {
                // To keep anchor density focused on architectural entrypoints:
                // Require doc comments OR primary top-level visibility
                let has_docstring = trimmed.contains("///") || trimmed.contains("/**");
                let is_top_level = !trimmed.contains("&self") && !trimmed.contains("&mut self");
                if has_docstring || is_top_level {
                    ChunkEmbedPolicy::Anchor
                } else {
                    ChunkEmbedPolicy::GraphOnly
                }
            } else {
                ChunkEmbedPolicy::GraphOnly
            }
        }
        SupportedLanguage::Go => {
            // In Go: func Name(...) or func (r Receiver) Name(...)
            let mut text = trimmed;
            if let Some(rest) = text.strip_prefix("func") {
                text = rest.trim_start();
            }
            if text.starts_with('(') {
                if let Some(close_idx) = text.find(')') {
                    text = text[close_idx + 1..].trim_start();
                }
            }
            let is_exported = text.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false);

            if is_exported && !text.starts_with("Test") && !text.starts_with("Benchmark") {
                ChunkEmbedPolicy::Anchor
            } else {
                ChunkEmbedPolicy::GraphOnly
            }
        }
        SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => {
            let is_exported = trimmed.starts_with("export function")
                || trimmed.starts_with("export default")
                || trimmed.starts_with("export const")
                || trimmed.starts_with("export class")
                || trimmed.starts_with("export interface")
                || trimmed.starts_with("export ")
                || trimmed.contains("export ");

            if is_exported {
                ChunkEmbedPolicy::Anchor
            } else {
                ChunkEmbedPolicy::GraphOnly
            }
        }
        SupportedLanguage::Python => {
            let mut text = trimmed;
            if let Some(rest) = text.strip_prefix("async ") {
                text = rest.trim_start();
            }
            if let Some(rest) = text.strip_prefix("def ") {
                text = rest.trim_start();
            }
            let is_private = text.starts_with('_');
            let is_test = text.starts_with("test_");
            if is_private || is_test {
                ChunkEmbedPolicy::GraphOnly
            } else {
                ChunkEmbedPolicy::Anchor
            }
        }
        SupportedLanguage::Java | SupportedLanguage::CSharp => {
            if trimmed.contains("public ") && !trimmed.contains("@Test") {
                ChunkEmbedPolicy::Anchor
            } else {
                ChunkEmbedPolicy::GraphOnly
            }
        }
        _ => ChunkEmbedPolicy::Anchor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_chunking_and_symbols() {
        let code = r#"
/// High performance search engine.
pub struct SearchEngine {
    pub name: String,
}

impl SearchEngine {
    /// Execute hybrid search across all modalities.
    pub fn search_hybrid(&self, query: &str) -> Vec<String> {
        let mut results = Vec::new();
        results.push(query.to_string());
        results
    }
}
"#;
        let config = ChunkingConfig::default();
        let res = CodeChunker::parse_and_chunk(Path::new("src/search/engine.rs"), code, &config)
            .expect("should parse rust");

        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "SearchEngine" && s.symbol_type == CodeSymbolType::Struct));
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "search_hybrid" && s.symbol_type == CodeSymbolType::Function));

        // Verify scope breadcrumb in chunk text
        let search_chunk = res
            .chunks
            .iter()
            .find(|c| c.scope_path.as_deref() == Some("SearchEngine > search_hybrid"))
            .expect("should find search_hybrid chunk");
        assert!(search_chunk.text.contains("// Scope: SearchEngine > search_hybrid"));
        assert!(search_chunk.text.contains("// Language: rust"));
        assert!(search_chunk.text.contains("Execute hybrid search"));
    }

    #[test]
    fn test_python_chunking_and_symbols() {
        let code = r#"
class DataProcessor:
    """Processes large datasets."""
    
    def process_batch(self, items):
        # Process items
        return [x * 2 for x in items]
"#;
        let config = ChunkingConfig::default();
        let res = CodeChunker::parse_and_chunk(Path::new("processor.py"), code, &config)
            .expect("should parse python");

        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "DataProcessor" && s.symbol_type == CodeSymbolType::Class));
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "process_batch" && s.symbol_type == CodeSymbolType::Method));

        let batch_chunk = res
            .chunks
            .iter()
            .find(|c| c.scope_path.as_deref() == Some("DataProcessor > process_batch"))
            .expect("should find process_batch chunk");
        assert!(batch_chunk.text.contains("# Scope: DataProcessor > process_batch"));
        assert!(batch_chunk.text.contains("# Language: python"));
    }

    #[test]
    fn test_typescript_chunking_and_symbols() {
        let code = r#"
export interface UserRecord {
    id: string;
    username: string;
}

export class UserService {
    /** Fetch user profile by ID */
    async getUser(id: string): Promise<UserRecord> {
        return { id, username: "admin" };
    }
}
"#;
        let config = ChunkingConfig::default();
        let res = CodeChunker::parse_and_chunk(Path::new("src/user.ts"), code, &config)
            .expect("should parse typescript");

        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "UserRecord" && s.symbol_type == CodeSymbolType::Interface));
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "UserService" && s.symbol_type == CodeSymbolType::Class));
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "getUser" && s.symbol_type == CodeSymbolType::Method));

        let get_user_chunk = res
            .chunks
            .iter()
            .find(|c| c.scope_path.as_deref() == Some("UserService > getUser"))
            .expect("should find getUser chunk");
        assert!(get_user_chunk.text.contains("// Scope: UserService > getUser"));
        assert!(get_user_chunk.text.contains("Fetch user profile by ID"));
    }

    #[test]
    fn test_extended_languages_chunking_and_symbols() {
        let config = ChunkingConfig::default();

        // 1. Ruby
        let ruby_code = r#"
class AuthManager
  def authenticate(user, password)
    true
  end
end
"#;
        let res = CodeChunker::parse_and_chunk(Path::new("auth.rb"), ruby_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "AuthManager" && s.symbol_type == CodeSymbolType::Class));
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "authenticate" && s.symbol_type == CodeSymbolType::Method));

        // 2. PHP
        let php_code = r#"<?php
class ApiClient {
    public function sendRequest($url) {
        return true;
    }
}
"#;
        let res = CodeChunker::parse_and_chunk(Path::new("client.php"), php_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "ApiClient" && s.symbol_type == CodeSymbolType::Class));
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "sendRequest" && s.symbol_type == CodeSymbolType::Method));

        // 3. Swift
        let swift_code = r#"
class NetworkService {
    func fetchData() -> String {
        return "data"
    }
}
"#;
        let res =
            CodeChunker::parse_and_chunk(Path::new("Network.swift"), swift_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "NetworkService" && s.symbol_type == CodeSymbolType::Class));
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "fetchData" && s.symbol_type == CodeSymbolType::Function));

        // 4. Elixir
        let elixir_code = r#"
defmodule MathEngine do
  def add(a, b) do
    a + b
  end
end
"#;
        let res = CodeChunker::parse_and_chunk(Path::new("math.ex"), elixir_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "MathEngine" && s.symbol_type == CodeSymbolType::Module));

        // 7. Lua
        let lua_code = r#"
local function calculate_total(price, tax)
    return price + tax
end
"#;
        let res = CodeChunker::parse_and_chunk(Path::new("math.lua"), lua_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "calculate_total" && s.symbol_type == CodeSymbolType::Function));

        // 8. Bash
        let bash_code = r#"
deploy_app() {
    echo "Deploying..."
}
"#;
        let res = CodeChunker::parse_and_chunk(Path::new("deploy.sh"), bash_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "deploy_app" && s.symbol_type == CodeSymbolType::Function));
    }

    #[test]
    fn test_classify_embed_policy() {
        // Containers are always anchors (except impl blocks)
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Struct,
                "struct Internal;",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Class,
                "class Secret {}",
                SupportedLanguage::TypeScript,
                "src/lib.ts"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Trait,
                "trait Handler {}",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Interface,
                "interface Api {}",
                SupportedLanguage::TypeScript,
                "src/lib.ts"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Enum,
                "enum Status {}",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Module,
                "pub mod internal {}",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Module,
                "impl<T> Handler for Service<T> {}",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::GraphOnly
        );

        // Test files are always GraphOnly
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Struct,
                "pub struct TestFixture {}",
                SupportedLanguage::Rust,
                "src/tests.rs"
            ),
            ChunkEmbedPolicy::GraphOnly
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "pub fn test_helper() {}",
                SupportedLanguage::Rust,
                "tests/integration.rs"
            ),
            ChunkEmbedPolicy::GraphOnly
        );

        // Rust functions/methods
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "pub fn exported() {}",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "pub(crate) fn crate_visible() {}",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "#[inline]\npub fn with_attr() {}",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "fn private_helper() {}",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::GraphOnly
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Method,
                "fn private_method(&self) {}",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::GraphOnly
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "#[test]\npub fn my_test() {}",
                SupportedLanguage::Rust,
                "src/lib.rs"
            ),
            ChunkEmbedPolicy::GraphOnly
        );

        // Go functions/methods
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "func ExportedFunction() {}",
                SupportedLanguage::Go,
                "server.go"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "func internalHelper() {}",
                SupportedLanguage::Go,
                "server.go"
            ),
            ChunkEmbedPolicy::GraphOnly
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Method,
                "func (s *Server) ListenAndServe() error {}",
                SupportedLanguage::Go,
                "server.go"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Method,
                "func (s *Server) cleanup() {}",
                SupportedLanguage::Go,
                "server.go"
            ),
            ChunkEmbedPolicy::GraphOnly
        );

        // TypeScript / JavaScript
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "export function search() {}",
                SupportedLanguage::TypeScript,
                "src/index.ts"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "function localHelper() {}",
                SupportedLanguage::TypeScript,
                "src/index.ts"
            ),
            ChunkEmbedPolicy::GraphOnly
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "export default function handler() {}",
                SupportedLanguage::JavaScript,
                "src/index.js"
            ),
            ChunkEmbedPolicy::Anchor
        );

        // Python
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "def public_endpoint(): pass",
                SupportedLanguage::Python,
                "api.py"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "def _private_worker(): pass",
                SupportedLanguage::Python,
                "api.py"
            ),
            ChunkEmbedPolicy::GraphOnly
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "async def fetch_data(): pass",
                SupportedLanguage::Python,
                "api.py"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Function,
                "async def _internal(): pass",
                SupportedLanguage::Python,
                "api.py"
            ),
            ChunkEmbedPolicy::GraphOnly
        );

        // Java / C#
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Method,
                "public void handleRequest() {}",
                SupportedLanguage::Java,
                "Server.java"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Method,
                "private void calculate() {}",
                SupportedLanguage::Java,
                "Server.java"
            ),
            ChunkEmbedPolicy::GraphOnly
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Method,
                "public async Task Execute() {}",
                SupportedLanguage::CSharp,
                "Worker.cs"
            ),
            ChunkEmbedPolicy::Anchor
        );
        assert_eq!(
            classify_embed_policy(
                CodeSymbolType::Method,
                "internal void Init() {}",
                SupportedLanguage::CSharp,
                "Worker.cs"
            ),
            ChunkEmbedPolicy::GraphOnly
        );
    }

    #[test]
    fn test_tier1_languages_chunking_and_symbols() {
        let config = ChunkingConfig::default();

        // 1. CUDA
        let cuda_code = "__global__ void my_kernel(int *a) { *a = 1; }";
        let res = CodeChunker::parse_and_chunk(Path::new("kernel.cu"), cuda_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "my_kernel" && s.symbol_type == CodeSymbolType::Function));

        // 2. Verilog
        let verilog_code = "module counter(input clk); endmodule";
        let res =
            CodeChunker::parse_and_chunk(Path::new("counter.v"), verilog_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "counter" && s.symbol_type == CodeSymbolType::Module));

        // 3. Starlark
        let starlark_code = "def helper(x):\n    return x\n";
        let res =
            CodeChunker::parse_and_chunk(Path::new("rules.bzl"), starlark_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "helper" && s.symbol_type == CodeSymbolType::Function));

        // 4. Bicep
        let bicep_code =
            "resource stg 'Microsoft.Storage/storageAccounts@2021-04-01' = { name: 'mystg' }";
        let res =
            CodeChunker::parse_and_chunk(Path::new("main.bicep"), bicep_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "stg" && s.symbol_type == CodeSymbolType::Struct));

        // 5. Gleam
        let gleam_code = "pub fn hello() { }\npub type Person { Person(name: String) }\n";
        let res =
            CodeChunker::parse_and_chunk(Path::new("main.gleam"), gleam_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "hello" && s.symbol_type == CodeSymbolType::Function));

        // 6. PowerShell
        let ps_code = "function Get-Data { param($x) }";
        let res = CodeChunker::parse_and_chunk(Path::new("script.ps1"), ps_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "Get-Data" && s.symbol_type == CodeSymbolType::Function));

        // 7. D
        let d_code = "class Greeter { void greet() {} }\nstruct Point { int x; }";
        let res = CodeChunker::parse_and_chunk(Path::new("app.d"), d_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "Greeter" && s.symbol_type == CodeSymbolType::Class));
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "Point" && s.symbol_type == CodeSymbolType::Struct));

        // 8. WGSL
        let wgsl_code = "struct VertexInput { position: vec3<f32>, };\nfn vs_main() {}";
        let res =
            CodeChunker::parse_and_chunk(Path::new("shader.wgsl"), wgsl_code, &config).unwrap();
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "VertexInput" && s.symbol_type == CodeSymbolType::Struct));
        assert!(res
            .symbols
            .iter()
            .any(|s| s.name == "vs_main" && s.symbol_type == CodeSymbolType::Function));

        // 9. HCL
        let hcl_code = "resource \"aws_s3_bucket\" \"b\" { bucket = \"my-tf-test-bucket\" }";
        let res = CodeChunker::parse_and_chunk(Path::new("main.tf"), hcl_code, &config).unwrap();
        assert!(!res.chunks.is_empty());

        // 10. Nix
        let nix_code = "{ pkgs ? import <nixpkgs> {} }: let f = x: x; in f";
        let res =
            CodeChunker::parse_and_chunk(Path::new("default.nix"), nix_code, &config).unwrap();
        assert!(!res.chunks.is_empty());

        // 11. TLA+
        let tla_code = "---- MODULE Test ----\nEXTENDS Naturals\nVARIABLE x\nInit == x = 0\n====";
        let res = CodeChunker::parse_and_chunk(Path::new("spec.tla"), tla_code, &config).unwrap();
        assert!(!res.chunks.is_empty());
    }

    #[test]
    fn test_hcl_resource_extraction() {
        let config = ChunkingConfig::default();
        let hcl_code = "resource \"aws_s3_bucket\" \"b\" { bucket = \"my-tf-test-bucket\" }";
        let res = CodeChunker::parse_and_chunk(Path::new("main.tf"), hcl_code, &config).unwrap();
        assert!(
            res.symbols
                .iter()
                .any(|s| s.name == "aws_s3_bucket.b" && s.symbol_type == CodeSymbolType::Struct),
            "HCL resource block must yield Struct symbol named 'aws_s3_bucket.b', got: {:?}",
            res.symbols
        );
        assert!(
            res.symbols.iter().any(|s| s.scope_path == "aws_s3_bucket.b"),
            "HCL resource block must have scope_path 'aws_s3_bucket.b', got: {:?}",
            res.symbols
        );
    }
}
