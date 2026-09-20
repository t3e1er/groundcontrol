//! Cross-corpus symbol resolution, external reference linking, and federated graph traversal.

use std::collections::{HashMap, HashSet, VecDeque};

use ctxvault_common::config::EdgeClass;
use ctxvault_common::ports::{GraphStore, MetadataCatalog};
use ctxvault_common::types::{
    CodeSymbol, CodeSymbolType, EdgeProvenance, ExternalRefKind, ResolutionConfidence,
};
use ctxvault_common::Result;

use crate::engine::Engine;

use super::manager::CorpusManager;
use super::types::{CorpusHop, FederatedNode, FederatedTraversal, ResolverKind};

impl CorpusManager {
    /// Resolve a fully qualified symbol name across every managed corpus.
    ///
    /// Queries each engine's store for an exact `scope_path` match and returns
    /// `(corpus_name, symbol)` for every match found across all corpora. An empty
    /// result means the name is unknown; more than one result means the name is
    /// ambiguous and must NOT be linked.
    pub fn resolve_symbol_across_corpora(&self, qualified_name: &str) -> Vec<(String, CodeSymbol)> {
        let mut matches = Vec::new();
        for (corpus_name, engine) in &self.engines {
            match engine.store().find_symbols_by_qualified_name(qualified_name) {
                Ok(symbols) => {
                    for sym in symbols {
                        matches.push((corpus_name.clone(), sym));
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        corpus = %corpus_name,
                        qualified_name,
                        error = %e,
                        "cross-corpus symbol lookup failed"
                    );
                }
            }
        }
        matches
    }

    /// Build an in-memory SCIP moniker index: leaf identifier -> list of (corpus_name, moniker).
    ///
    /// Scans graph nodes across all engines ONCE instead of rescanning for every
    /// candidate reference, keeping resolution bounded and preventing quadratic allocations.
    fn build_scip_index(&self) -> HashMap<String, Vec<(String, String)>> {
        let mut scip_by_leaf: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for (corpus_name, engine) in &self.engines {
            for node in engine.graph().node_paths() {
                if crate::graph::scip::looks_like_moniker(&node) {
                    if let Some(leaf) = crate::graph::scip::moniker_leaf(&node) {
                        scip_by_leaf.entry(leaf).or_default().push((corpus_name.clone(), node));
                    }
                }
            }
        }
        scip_by_leaf
    }

    fn resolve_ref_via_scip_index(
        scip_index: &HashMap<String, Vec<(String, String)>>,
        source_corpus: &str,
        leaf: &str,
    ) -> Option<(String, CodeSymbol)> {
        let candidates = scip_index.get(leaf)?;
        let mut matches = candidates.iter().filter(|(c, _)| c != source_corpus);
        let (target_corpus, moniker) = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        let symbol = CodeSymbol {
            file_path: String::new(),
            name: leaf.to_string(),
            scope_path: moniker.clone(),
            symbol_type: CodeSymbolType::Function,
            language: String::new(),
            signature: String::new(),
            docstring: None,
            start_line: 0,
            end_line: 0,
        };
        Some((target_corpus.clone(), symbol))
    }

    fn resolve_ref_with_scip_index(
        &self,
        scip_index: &HashMap<String, Vec<(String, String)>>,
        source_corpus: &str,
        raw_target: &str,
    ) -> Option<(String, CodeSymbol, ResolverKind)> {
        // Clean the raw target to its leaf identifier, mirroring the code
        // extractor's `resolve_callee` cleaning (last segment after "::" then ".").
        let leaf = raw_target.rsplit("::").next().unwrap_or(raw_target);
        let leaf = leaf.rsplit('.').next().unwrap_or(leaf);

        // Tier 1 (highest trust): SCIP monikers via pre-indexed lookup.
        if let Some((corpus, symbol)) =
            Self::resolve_ref_via_scip_index(scip_index, source_corpus, leaf)
        {
            return Some((corpus, symbol, ResolverKind::Scip));
        }

        // Tier 2 (fallback): qualified-name matching (Phase 3 path).
        // Try the raw target directly first (preserves qualified infra resource names
        // like "aws_s3_bucket.b"), falling back to the leaf identifier.
        let mut resolved = self.resolve_symbol_across_corpora(raw_target);
        if resolved.len() != 1 && raw_target != leaf {
            resolved = self.resolve_symbol_across_corpora(leaf);
        }
        if resolved.len() == 1 {
            let (target_corpus, symbol) = &resolved[0];
            if target_corpus != source_corpus {
                return Some((target_corpus.clone(), symbol.clone(), ResolverKind::QualName));
            }
        }

        None
    }

    /// Resolve `raw_target` (a caller's unresolved call/import target) to a
    /// unique symbol in a DIFFERENT corpus, walking the resolver trust ladder.
    pub fn resolve_ref_across_corpora(
        &self,
        source_corpus: &str,
        raw_target: &str,
    ) -> Option<(String, CodeSymbol, ResolverKind)> {
        let scip_index = self.build_scip_index();
        self.resolve_ref_with_scip_index(&scip_index, source_corpus, raw_target)
    }

    /// Post-index linking pass that injects cross-corpus doc→code edges.
    ///
    /// For each corpus (the "doc side"), every document's outgoing
    /// frontmatter-provenance edge targets are treated as candidate doc→code
    /// links. A candidate is linked only when it:
    ///
    /// 1. does NOT already resolve to a node within the same corpus's graph, and
    /// 2. resolves to EXACTLY ONE `(corpus, symbol)` across all corpora.
    ///
    /// When both hold, an edge is injected into the doc's corpus graph pointing at
    /// a distinct cross-corpus node keyed `"<corpus>::<scope_path>"`, tagged with
    /// [`EdgeProvenance::DocumentsCode`], `target_corpus`, and
    /// [`ResolutionConfidence::High`]. Ambiguous (>1) or unresolved (0) candidates
    /// produce no edge, so no false or dangling edges are ever created.
    ///
    /// The pass is idempotent: re-running relies on the graph's same-type edge
    /// de-duplication, so repeated invocations neither duplicate edges nor grow
    /// the graph unbounded. Returns the number of cross-corpus edges created.
    pub fn link_cross_corpus_symbols(&mut self) -> Result<usize> {
        // Phase 1: gather link decisions using immutable access (no borrow conflict).
        // Each decision: (doc_corpus, doc_path, edge_type, target_corpus, node_key, title).
        struct CrossLink {
            doc_corpus: String,
            doc_path: String,
            edge_type: String,
            target_corpus: String,
            node_key: String,
            title: Option<String>,
            /// Repo-relative path of the resolved symbol in `target_corpus`.
            target_path: String,
            /// Fully qualified scope path of the resolved symbol.
            target_symbol: String,
        }

        let mut decisions: Vec<CrossLink> = Vec::new();

        for (doc_corpus, engine) in &self.engines {
            let graph = engine.graph();
            for doc_path in graph.node_paths() {
                for (edge_type, raw_target) in graph.outgoing_frontmatter_targets(&doc_path) {
                    // Skip candidates that already resolve within the same corpus.
                    if graph.contains_node(&raw_target)
                        && raw_target != doc_path
                        && Self::is_intra_corpus_symbol(engine, &raw_target)
                    {
                        continue;
                    }

                    let resolved = self.resolve_symbol_across_corpora(&raw_target);
                    // Only unambiguous, single, cross-corpus matches are linked.
                    if resolved.len() != 1 {
                        continue;
                    }
                    let (target_corpus, symbol) = &resolved[0];
                    // Must be a DIFFERENT corpus (intra-corpus already handled by key match).
                    if target_corpus == doc_corpus {
                        continue;
                    }

                    let node_key = format!("{}::{}", target_corpus, symbol.scope_path);
                    decisions.push(CrossLink {
                        doc_corpus: doc_corpus.clone(),
                        doc_path: doc_path.clone(),
                        edge_type,
                        target_corpus: target_corpus.clone(),
                        node_key,
                        title: Some(symbol.name.clone()),
                        target_path: symbol.file_path.clone(),
                        target_symbol: symbol.scope_path.clone(),
                    });
                }
            }
        }

        // Phase 2: apply decisions with mutable access to each doc corpus graph.
        let mut created = 0usize;
        for link in decisions {
            let engine = self.get_engine_mut(&link.doc_corpus)?;
            let graph = engine.graph_mut();
            graph.add_node(&link.node_key, link.title.as_deref());
            graph.add_cross_corpus_edge(
                &link.doc_path,
                &link.node_key,
                &link.edge_type,
                1.0,
                EdgeProvenance::DocumentsCode,
                EdgeClass::Structural,
                Some(link.target_corpus),
                Some(ResolutionConfidence::High),
                Some(link.target_path),
                Some(link.target_symbol),
                Some("Symbol".to_string()),
            );
            created += 1;
        }

        Ok(created)
    }

    /// Post-index linking pass that resolves code [`ExternalRef`](ctxvault_common::types::ExternalRef)s across corpora.
    pub fn resolve_external_refs(&mut self) -> Result<usize> {
        // Phase 1: gather link decisions using immutable access (no borrow conflict).
        struct CrossRef {
            /// Corpus whose ExternalRef this is (owns the forward edge source).
            source_corpus: String,
            /// Scope path of the caller/importer node in the source graph.
            caller_scope_path: String,
            /// Kind of reference (drives edge type + provenance).
            kind: ExternalRefKind,
            /// Corpus the target symbol was uniquely resolved in.
            target_corpus: String,
            /// The uniquely resolved target symbol.
            symbol: CodeSymbol,
            /// Resolver tier that produced the match (drives edge confidence).
            resolver: ResolverKind,
        }

        let mut decisions: Vec<CrossRef> = Vec::new();
        let scip_index = self.build_scip_index();

        for (source_corpus, engine) in &self.engines {
            let refs = engine.store().get_external_refs()?;
            let mut memo: HashMap<String, Option<(String, CodeSymbol, ResolverKind)>> =
                HashMap::new();

            for ext in refs {
                let resolved = match memo.get(&ext.raw_target) {
                    Some(res) => res.clone(),
                    None => {
                        let res = self.resolve_ref_with_scip_index(
                            &scip_index,
                            source_corpus,
                            &ext.raw_target,
                        );
                        memo.insert(ext.raw_target.clone(), res.clone());
                        res
                    }
                };

                let Some((target_corpus, symbol, resolver)) = resolved else {
                    continue;
                };

                decisions.push(CrossRef {
                    source_corpus: source_corpus.clone(),
                    caller_scope_path: ext.caller_scope_path,
                    kind: ext.kind,
                    target_corpus,
                    symbol,
                    resolver,
                });
            }
        }

        // Phase 2: apply decisions with mutable access to each corpus graph.
        let mut created = 0usize;
        for dec in decisions {
            let (edge_type, provenance) = match dec.kind {
                ExternalRefKind::Import => ("imports", EdgeProvenance::CodeImports),
                _ => ("calls", EdgeProvenance::CodeCalls),
            };

            let confidence = match dec.resolver {
                ResolverKind::Scip | ResolverKind::HybridLsp | ResolverKind::QualName => {
                    ResolutionConfidence::High
                }
            };

            let target_kind = if dec.symbol.language == "hcl" || dec.symbol.language == "bicep" {
                "Resource".to_string()
            } else {
                "Symbol".to_string()
            };

            // Forward edge: caller (source corpus) -> "<target_corpus>::<scope_path>".
            let forward_target = format!("{}::{}", dec.target_corpus, dec.symbol.scope_path);
            {
                let engine = self.get_engine_mut(&dec.source_corpus)?;
                let graph = engine.graph_mut();
                graph.add_node(&dec.caller_scope_path, None);
                graph.add_node(&forward_target, Some(&dec.symbol.name));
                graph.add_cross_corpus_edge(
                    &dec.caller_scope_path,
                    &forward_target,
                    edge_type,
                    1.0,
                    provenance.clone(),
                    EdgeClass::Code,
                    Some(dec.target_corpus.clone()),
                    Some(confidence),
                    Some(dec.symbol.file_path.clone()),
                    Some(dec.symbol.scope_path.clone()),
                    Some(target_kind.clone()),
                );
            }

            // Reverse edge: resolved symbol (target corpus) -> proxy caller node.
            let reverse_target = format!("{}::{}", dec.source_corpus, dec.caller_scope_path);
            {
                let engine = self.get_engine_mut(&dec.target_corpus)?;
                let graph = engine.graph_mut();
                graph.add_node(&dec.symbol.scope_path, Some(&dec.symbol.name));
                graph.add_node(&reverse_target, None);
                graph.add_cross_corpus_edge(
                    &dec.symbol.scope_path,
                    &reverse_target,
                    edge_type,
                    1.0,
                    provenance,
                    EdgeClass::Code,
                    Some(dec.source_corpus.clone()),
                    Some(confidence),
                    None,
                    Some(dec.caller_scope_path.clone()),
                    Some(target_kind),
                );
            }

            created += 1;
        }

        Ok(created)
    }

    /// Federated (cross-corpus continuation) traversal.
    pub fn federated_traverse(
        &self,
        start_corpus: &str,
        start_node: &str,
        per_corpus_depth: usize,
        max_corpus_hops: usize,
        continue_across: bool,
    ) -> Result<FederatedTraversal> {
        let start_engine = self.get_engine(start_corpus)?;
        let mut result = FederatedTraversal::default();

        if !start_engine.graph().contains_node(start_node) {
            return Ok(result);
        }

        let mut visited: HashSet<(String, String)> = HashSet::new();
        let mut queue: VecDeque<(String, String, usize, usize)> = VecDeque::new();

        let _ = visited.insert((start_corpus.to_string(), start_node.to_string()));
        queue.push_back((start_corpus.to_string(), start_node.to_string(), 0, 0));
        result.nodes.push(FederatedNode {
            corpus: start_corpus.to_string(),
            node: start_node.to_string(),
            depth: 0,
        });

        while let Some((corpus, node, intra_depth, corpus_hops_used)) = queue.pop_front() {
            let Ok(engine) = self.get_engine(&corpus) else {
                continue;
            };

            let mut edges = engine.graph().outgoing_edges(&node);
            edges.sort_by(|a, b| {
                a.edge_type.cmp(&b.edge_type).then_with(|| a.target.cmp(&b.target))
            });

            for edge in edges {
                match &edge.target_corpus {
                    None => {
                        if intra_depth >= per_corpus_depth {
                            continue;
                        }
                        let key = (corpus.clone(), edge.target.clone());
                        if visited.contains(&key) {
                            continue;
                        }
                        let _ = visited.insert(key);
                        result.nodes.push(FederatedNode {
                            corpus: corpus.clone(),
                            node: edge.target.clone(),
                            depth: intra_depth + 1,
                        });
                        queue.push_back((
                            corpus.clone(),
                            edge.target.clone(),
                            intra_depth + 1,
                            corpus_hops_used,
                        ));
                    }
                    Some(to_corpus) => {
                        let to_node = edge.target_symbol.clone().or_else(|| {
                            edge.target
                                .strip_prefix(&format!("{}::", to_corpus))
                                .map(|s| s.to_string())
                        });

                        result.hops.push(CorpusHop {
                            from_corpus: corpus.clone(),
                            from_node: node.clone(),
                            to_corpus: to_corpus.clone(),
                            to_node: to_node.clone(),
                            edge_type: edge.edge_type.clone(),
                            target_kind: edge.target_kind.clone(),
                            confidence: edge.confidence,
                            corpus_depth: corpus_hops_used + 1,
                        });

                        if !continue_across || corpus_hops_used >= max_corpus_hops {
                            continue;
                        }
                        let Some(to_node) = to_node else {
                            continue;
                        };
                        let Ok(to_engine) = self.get_engine(to_corpus) else {
                            continue;
                        };
                        if !to_engine.graph().contains_node(&to_node) {
                            continue;
                        }
                        let key = (to_corpus.clone(), to_node.clone());
                        if visited.contains(&key) {
                            continue;
                        }
                        let _ = visited.insert(key);
                        result.nodes.push(FederatedNode {
                            corpus: to_corpus.clone(),
                            node: to_node.clone(),
                            depth: 0,
                        });
                        queue.push_back((to_corpus.clone(), to_node, 0, corpus_hops_used + 1));
                    }
                }
            }
        }

        Ok(result)
    }

    /// Whether a graph node keyed by `scope_path` corresponds to a code symbol
    /// defined in this engine's own corpus (as opposed to a bare doc target that
    /// merely happens to share the string).
    pub(crate) fn is_intra_corpus_symbol(engine: &Engine, scope_path: &str) -> bool {
        engine
            .store()
            .find_symbols_by_qualified_name(scope_path)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }
}
