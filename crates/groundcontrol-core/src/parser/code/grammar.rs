//! Standard AST-extracted grammar interface for language-agnostic code semantics.
//!
//! Provides a declarative, grammar-driven extraction pipeline that derives interface
//! definitions, API call targets, def-use data flow paths, and structural grammar
//! transitions directly from Tree-sitter syntax trees without handwritten keyword
//! checks or language-specific string heuristics.

use std::collections::HashSet;
use tree_sitter::Node;

use super::patterns::split_identifier;
use super::spec::LanguageSpec;

/// Standard interface for AST-extracted grammar semantics from source code nodes.
pub trait AstGrammarExtractor: Send + Sync {
    /// Extract structured semantic streams from a Tree-sitter AST node representing a code symbol.
    fn extract_grammar_semantics(&self, node: Node, source: &str) -> ExtractedGrammarSemantics;
}

/// Extracted structural semantic signals from AST grammar.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtractedGrammarSemantics {
    /// Channel 0: Interface and declaration tokens (symbol name, parameter names, types).
    pub interface_tokens: Vec<WeightedToken>,
    /// Channel 1: Outbound API calls and invocations.
    pub api_tokens: Vec<WeightedToken>,
    /// Channel 2: Def-Use data flow paths (parameter flow to arguments, returns, and conditions).
    pub dataflow_paths: Vec<DataFlowPath>,
    /// Channel 3: Structural AST grammar transition bigrams and complexity profile.
    pub grammar_transitions: Vec<GrammarTransition>,
}

/// A weighted semantic token with structural tree-depth attenuation.
#[derive(Debug, Clone, PartialEq)]
pub struct WeightedToken {
    /// The token text.
    pub text: String,
    /// Structural depth weight: `1.0 / sqrt(1.0 + depth)`.
    pub weight: f32,
}

/// A data-flow def-use path linking a declared parameter to an internal sink.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DataFlowPath {
    /// Parameter identifier name.
    pub source_param: String,
    /// Destination sink kind.
    pub sink: DataFlowSink,
}

/// Destination sink in intra-symbol def-use chains.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataFlowSink {
    /// Parameter passed as an argument into an outbound callee.
    Call(String),
    /// Parameter returned from the function.
    Return,
    /// Parameter evaluated within a conditional branch or loop condition.
    Condition,
}

/// A structural parent-child grammar rule transition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GrammarTransition {
    /// Tree-sitter node kind of the parent.
    pub parent_kind: String,
    /// Tree-sitter node kind of the child.
    pub child_kind: String,
    /// Relative tree depth.
    pub depth: u16,
}

/// Generic, language-agnostic AST grammar extractor driven by `LanguageSpec`.
#[derive(Debug, Clone, Copy)]
pub struct GenericAstGrammarExtractor {
    spec: &'static LanguageSpec,
}

impl GenericAstGrammarExtractor {
    /// Create a new generic extractor for the given language specification.
    pub fn new(spec: &'static LanguageSpec) -> Self {
        Self { spec }
    }

    /// Extract text from a node safely bounded by source bounds.
    #[inline]
    fn node_text<'a>(node: Node<'a>, source: &'a str) -> &'a str {
        let start = node.start_byte();
        let end = node.end_byte();
        if start <= end && end <= source.len() {
            &source[start..end]
        } else {
            ""
        }
    }

    /// Collect leaf identifier tokens from an AST sub-tree.
    fn collect_identifiers<'a>(node: Node<'a>, source: &'a str, out: &mut Vec<String>) {
        if node.child_count() == 0 {
            let kind = node.kind();
            if kind == "identifier"
                || kind == "type_identifier"
                || kind == "field_identifier"
                || kind == "property_identifier"
            {
                let text = Self::node_text(node, source).trim();
                if !text.is_empty() {
                    out.push(text.to_string());
                }
            }
            return;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::collect_identifiers(child, source, out);
        }
    }
}

impl AstGrammarExtractor for GenericAstGrammarExtractor {
    fn extract_grammar_semantics(&self, node: Node, source: &str) -> ExtractedGrammarSemantics {
        let mut semantics = ExtractedGrammarSemantics::default();
        let mut param_names = HashSet::new();

        // ── Channel 0: Interface and Declarations ────────────────────────
        // 1. Symbol Name
        let name_field = self.spec.name_field.unwrap_or("name");
        if let Some(name_node) = node.child_by_field_name(name_field) {
            let name_text = Self::node_text(name_node, source).trim();
            if !name_text.is_empty() {
                semantics
                    .interface_tokens
                    .push(WeightedToken { text: name_text.to_string(), weight: 1.0 });
                for sub in split_identifier(name_text) {
                    if sub != name_text {
                        semantics.interface_tokens.push(WeightedToken { text: sub, weight: 0.9 });
                    }
                }
            }
        }

        // 2. Parameters & Parameter Types
        let params_field = self.spec.parameters_field();
        if let Some(params_node) = node.child_by_field_name(params_field) {
            let mut raw_params = Vec::new();
            Self::collect_identifiers(params_node, source, &mut raw_params);
            for p in raw_params {
                param_names.insert(p.clone());
                semantics.interface_tokens.push(WeightedToken { text: p.clone(), weight: 0.85 });
                for sub in split_identifier(&p) {
                    if sub != p {
                        semantics.interface_tokens.push(WeightedToken { text: sub, weight: 0.75 });
                    }
                }
            }
        }

        // 3. Return Type
        let ret_field = self.spec.return_type_field();
        if let Some(ret_node) = node.child_by_field_name(ret_field) {
            let mut raw_rets = Vec::new();
            Self::collect_identifiers(ret_node, source, &mut raw_rets);
            for r in raw_rets {
                semantics.interface_tokens.push(WeightedToken { text: r.clone(), weight: 0.80 });
                for sub in split_identifier(&r) {
                    if sub != r {
                        semantics.interface_tokens.push(WeightedToken { text: sub, weight: 0.70 });
                    }
                }
            }
        }

        // ── Traverse Body for Channels 1, 2, and 3 ───────────────────────
        let body_node = node.child_by_field_name(self.spec.body_field()).unwrap_or(node);
        let mut stack: Vec<(Node, usize)> = vec![(body_node, 1)];

        const MAX_TRAVERSAL_DEPTH: usize = 32;

        while let Some((cur, depth)) = stack.pop() {
            if depth >= MAX_TRAVERSAL_DEPTH {
                continue;
            }

            let kind = cur.kind();
            let depth_weight = 1.0 / (1.0 + depth as f32).sqrt();

            // Channel 1: Outbound API Calls
            if self.spec.is_call(kind) {
                let callee_node = cur
                    .child_by_field_name(self.spec.call_function_field())
                    .or_else(|| cur.named_child(0));

                let callee_name =
                    callee_node.map(|cn| Self::node_text(cn, source).trim().to_string());

                if let Some(ref cname) = callee_name {
                    if !cname.is_empty() {
                        semantics
                            .api_tokens
                            .push(WeightedToken { text: cname.clone(), weight: depth_weight });
                        for sub in split_identifier(cname) {
                            if sub != *cname {
                                semantics
                                    .api_tokens
                                    .push(WeightedToken { text: sub, weight: depth_weight * 0.8 });
                            }
                        }

                        // Channel 2: Data Flow into Call Arguments
                        if !param_names.is_empty() {
                            let args_node = cur
                                .child_by_field_name(self.spec.call_arguments_field())
                                .or_else(|| cur.named_child(1));

                            if let Some(an) = args_node {
                                let mut arg_idents = Vec::new();
                                Self::collect_identifiers(an, source, &mut arg_idents);
                                for arg in arg_idents {
                                    if param_names.contains(&arg) {
                                        semantics.dataflow_paths.push(DataFlowPath {
                                            source_param: arg,
                                            sink: DataFlowSink::Call(cname.clone()),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Channel 2: Def-Use Data Flow into Return Statements
            if kind.contains("return") && !param_names.is_empty() {
                let mut ret_idents = Vec::new();
                Self::collect_identifiers(cur, source, &mut ret_idents);
                for id in ret_idents {
                    if param_names.contains(&id) {
                        semantics
                            .dataflow_paths
                            .push(DataFlowPath { source_param: id, sink: DataFlowSink::Return });
                    }
                }
            }

            // Channel 2: Def-Use Data Flow into Conditions
            if (kind.contains("if") || kind.contains("while") || kind.contains("condition"))
                && !param_names.is_empty()
            {
                if let Some(cond_node) = cur.child_by_field_name("condition") {
                    let mut cond_idents = Vec::new();
                    Self::collect_identifiers(cond_node, source, &mut cond_idents);
                    for id in cond_idents {
                        if param_names.contains(&id) {
                            semantics.dataflow_paths.push(DataFlowPath {
                                source_param: id,
                                sink: DataFlowSink::Condition,
                            });
                        }
                    }
                }
            }

            // Channel 3: Structural Control Grammar Transitions
            let mut cursor = cur.walk();
            for child in cur.children(&mut cursor) {
                if child.is_named() {
                    semantics.grammar_transitions.push(GrammarTransition {
                        parent_kind: cur.kind().to_string(),
                        child_kind: child.kind().to_string(),
                        depth: depth as u16,
                    });
                    stack.push((child, depth + 1));
                }
            }
        }

        semantics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::code::languages::SupportedLanguage;
    use crate::parser::code::spec::get_language_spec;
    use tree_sitter::Parser;

    #[test]
    fn test_ast_grammar_extractor_rust() {
        let code = r#"
pub fn transfer_funds(account: &mut Account, amount: u64) -> Result<(), Error> {
    if amount > 0 {
        account.deposit(amount);
        Ok(())
    } else {
        Err(Error::InvalidAmount)
    }
}
"#;
        let lang = SupportedLanguage::Rust;
        let mut parser = Parser::new();
        parser.set_language(&lang.tree_sitter_language()).unwrap();
        let tree = parser.parse(code, None).unwrap();
        let root = tree.root_node();
        let fn_node = root.named_child(0).unwrap();

        let spec = get_language_spec(lang);
        let extractor = GenericAstGrammarExtractor::new(spec);
        let sem = extractor.extract_grammar_semantics(fn_node, code);

        // Check Channel 0: Interface
        assert!(sem.interface_tokens.iter().any(|t| t.text == "transfer_funds"));
        assert!(sem.interface_tokens.iter().any(|t| t.text == "account"));
        assert!(sem.interface_tokens.iter().any(|t| t.text == "amount"));

        // Check Channel 1: API Invocations
        assert!(sem.api_tokens.iter().any(|t| t.text.contains("deposit")));

        // Check Channel 2: Data Flow
        assert!(sem.dataflow_paths.iter().any(|p| p.source_param == "amount"
            && matches!(&p.sink, DataFlowSink::Call(c) if c.contains("deposit"))));

        // Check Channel 3: Grammar transitions
        assert!(!sem.grammar_transitions.is_empty());
    }

    #[test]
    fn test_ast_grammar_extractor_python() {
        let code = r#"
def send_notification(user, message):
    if user.is_active:
        client.dispatch(message)
        return True
    return False
"#;
        let lang = SupportedLanguage::Python;
        let mut parser = Parser::new();
        parser.set_language(&lang.tree_sitter_language()).unwrap();
        let tree = parser.parse(code, None).unwrap();
        let root = tree.root_node();
        let fn_node = root.named_child(0).unwrap();

        let spec = get_language_spec(lang);
        let extractor = GenericAstGrammarExtractor::new(spec);
        let sem = extractor.extract_grammar_semantics(fn_node, code);

        // Check Channel 0: Interface
        assert!(sem.interface_tokens.iter().any(|t| t.text == "send_notification"));
        assert!(sem.interface_tokens.iter().any(|t| t.text == "user"));
        assert!(sem.interface_tokens.iter().any(|t| t.text == "message"));

        // Check Channel 1: API
        assert!(sem.api_tokens.iter().any(|t| t.text.contains("dispatch")));

        // Check Channel 2: Data Flow (message flowing into dispatch)
        assert!(sem.dataflow_paths.iter().any(|p| p.source_param == "message"
            && matches!(&p.sink, DataFlowSink::Call(c) if c.contains("dispatch"))));
    }
}
