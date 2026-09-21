//! Core implementation of the [`SearchService`] port.
//!
//! [`CoreSearchService`] owns the search-mode dispatch (`bm25` | `semantic` |
//! `hybrid` | `graph` | `explain`) that previously lived inline in the MCP
//! `handle_search` tool. It borrows the resolved retrieval backends it needs
//! (the BM25 index, the optional vector index, the knowledge graph, an optional
//! embedder) plus the corpus's code-path set, and forwards each mode to the
//! existing `crate::search` free functions — preserving their fallbacks,
//! defaults, and error messages byte-for-byte.
//!
//! # Boundary
//!
//! The service receives **already-resolved** inputs. Engine-specific concerns —
//! lazy embedder initialization (`ensure_embedder`), fast-mode detection,
//! resolving `embedder_ref` / `vector_index` presence / `code_paths_set`, and
//! (after this seam) the `detail`/verbosity shaping and JSON serialization —
//! stay in the MCP adapter. The service reproduces the semantic-mode fast-mode
//! guard purely from its inputs: a semantic search with no vector index present
//! yields the same "unavailable in fast mode" error.
//!
//! # `explain` dummy vector index
//!
//! The `explain` mode calls `crate::search::search_explain`, which takes a
//! `&VectorIndex` unconditionally. When no vector index is present (fast mode),
//! the original code constructed an empty 384-dim fallback index. That fallback
//! now lives here, inside the adapter's own crate (core is allowed to name
//! `VectorIndex`), so the MCP layer no longer needs to reference
//! `VectorIndex::new_default`.

use std::collections::HashSet;
use std::sync::Arc;

use groundcontrol_common::config::EdgeClass;
use groundcontrol_common::ports::{SearchQuery, SearchService};
use groundcontrol_common::types::{Modality, SearchExplanation, SearchResult};
use groundcontrol_common::{Error, Result};

use crate::embedding::Embedder;
use crate::graph::KnowledgeGraph;
use crate::index::BM25Index;
use crate::search::{self, binary::BinarySearchIndex};
use crate::vector_index::VectorIndex;

/// Core adapter implementing the [`SearchService`] port.
///
/// Holds borrowed references to the retrieval backends resolved by the caller
/// (composition happens per request in the MCP adapter). It owns the five-arm
/// mode dispatch and forwards to the concrete `crate::search` free functions.
pub struct CoreSearchService<'a> {
    bm25: &'a BM25Index,
    vector_index: Option<&'a VectorIndex>,
    binary_index: Option<&'a BinarySearchIndex>,
    graph: &'a KnowledgeGraph,
    embedder: Option<Arc<Embedder>>,
    code_paths: HashSet<String>,
}

impl<'a> CoreSearchService<'a> {
    /// Construct a search service over the given resolved backends.
    ///
    /// - `bm25`: the corpus BM25 index (always present).
    /// - `vector_index`: the vector index, or `None` in fast mode.
    /// - `binary_index`: the binary fingerprints index for fast CPU search.
    /// - `graph`: the corpus knowledge graph.
    /// - `embedder`: an initialized embedder (owned `Arc` clone), or `None`
    ///   when unavailable. Holding the `Arc` (rather than a borrow) frees the
    ///   caller from keeping a separate `Arc` alive across the service's borrow.
    /// - `code_paths`: the set of code-node keys used for modality filtering.
    pub fn new(
        bm25: &'a BM25Index,
        vector_index: Option<&'a VectorIndex>,
        binary_index: Option<&'a BinarySearchIndex>,
        graph: &'a KnowledgeGraph,
        embedder: Option<Arc<Embedder>>,
        code_paths: HashSet<String>,
    ) -> Self {
        Self { bm25, vector_index, binary_index, graph, embedder, code_paths }
    }
}

impl SearchService for CoreSearchService<'_> {
    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let mode = query.mode.as_deref().unwrap_or("hybrid");
        let limit = query.limit.unwrap_or(10);
        let modality = query.modality;

        let mut results = match mode {
            "fast" => {
                let edge_class_filter = match query.edge_class.as_deref() {
                    Some(s) => EdgeClass::from_str_name(s),
                    None => match modality {
                        Modality::Code => Some(EdgeClass::Code),
                        Modality::Docs => Some(EdgeClass::Semantic),
                        Modality::Both => None,
                    },
                };
                let empty_binary = BinarySearchIndex::new();
                let binary = self.binary_index.unwrap_or(&empty_binary);
                let results = search::search_fast(
                    self.bm25,
                    binary,
                    self.graph,
                    &query.query,
                    limit,
                    modality,
                    edge_class_filter,
                    &self.code_paths,
                )?;
                Ok(results)
            }
            "bm25" => {
                let mut results = search::search_bm25(self.bm25, &query.query, limit, modality)?;
                search::enrich_results_with_lineage(&mut results, self.graph);
                Ok(results)
            }
            "semantic" => {
                if self.vector_index.is_none() {
                    return Err(Error::Index(
                        "Semantic search is unavailable in fast mode. Re-index with index_mode = 'full' to enable vector search.".to_string(),
                    ));
                }
                let depth = query.depth;

                let embedder = match self.embedder.as_deref() {
                    Some(e) => e,
                    None => {
                        return Err(Error::Index(
                            "embedder not available — cannot perform semantic search".to_string(),
                        ));
                    }
                };
                let vector_index = self.vector_index.unwrap();

                let mut results = search::search_semantic_dual(
                    vector_index,
                    embedder,
                    &query.query,
                    limit,
                    depth,
                    modality,
                )?;
                search::enrich_results_with_lineage(&mut results, self.graph);
                Ok(results)
            }
            "hybrid" => {
                let graph_depth = query.graph_depth.unwrap_or(2);
                let edge_type_filter = query.edge_types.as_deref();
                let code_paths = &self.code_paths;

                let edge_class_filter = match query.edge_class.as_deref() {
                    Some(s) => EdgeClass::from_str_name(s),
                    None => match modality {
                        Modality::Code => Some(EdgeClass::Code),
                        Modality::Docs => Some(EdgeClass::Semantic),
                        Modality::Both => None,
                    },
                };

                // Try to get a query embedding for full 3-signal hybrid.
                // If the embedder is unavailable, fall back to BM25+graph only.
                let embedder_opt = self.embedder.as_deref();
                let query_embedding =
                    embedder_opt.and_then(|embedder| embedder.embed_query(&query.query).ok());

                let results_raw = if let Some(vector_index) = self.vector_index {
                    if query.decompose == Some(true) {
                        // Multi-hop query decomposition mode.
                        search::search_multihop(
                            self.bm25,
                            vector_index,
                            self.graph,
                            embedder_opt,
                            &query.query,
                            query_embedding.as_deref(),
                            limit,
                            graph_depth,
                            edge_type_filter,
                            modality,
                            code_paths,
                        )?
                    } else {
                        match modality {
                            Modality::Docs => search::search_hybrid_full(
                                self.bm25,
                                vector_index,
                                self.graph,
                                &query.query,
                                query_embedding.as_deref(),
                                limit,
                                graph_depth,
                                edge_type_filter,
                                edge_class_filter,
                                Modality::Docs,
                                code_paths,
                            )?,
                            Modality::Code => {
                                let empty_binary = BinarySearchIndex::new();
                                let binary = self.binary_index.unwrap_or(&empty_binary);
                                search::search_fast(
                                    self.bm25,
                                    binary,
                                    self.graph,
                                    &query.query,
                                    limit,
                                    Modality::Code,
                                    edge_class_filter,
                                    code_paths,
                                )?
                            }
                            Modality::Both => {
                                let doc_results = search::search_hybrid_full(
                                    self.bm25,
                                    vector_index,
                                    self.graph,
                                    &query.query,
                                    query_embedding.as_deref(),
                                    limit,
                                    graph_depth,
                                    edge_type_filter,
                                    Some(EdgeClass::Semantic),
                                    Modality::Docs,
                                    code_paths,
                                )?;
                                let empty_binary = BinarySearchIndex::new();
                                let binary = self.binary_index.unwrap_or(&empty_binary);
                                let code_results = search::search_fast(
                                    self.bm25,
                                    binary,
                                    self.graph,
                                    &query.query,
                                    limit,
                                    Modality::Code,
                                    Some(EdgeClass::Code),
                                    code_paths,
                                )?;
                                let mut combined = doc_results;
                                combined.extend(code_results);
                                combined
                            }
                        }
                    }
                } else {
                    // Fast Mode: Binary Hamming + BM25 + Graph.
                    let empty_binary = BinarySearchIndex::new();
                    let binary = self.binary_index.unwrap_or(&empty_binary);
                    search::search_fast(
                        self.bm25,
                        binary,
                        self.graph,
                        &query.query,
                        limit,
                        modality,
                        edge_class_filter,
                        code_paths,
                    )?
                };

                Ok(results_raw)
            }
            "graph" => {
                let max_depth = query.graph_depth.unwrap_or(3);
                let edge_type_filter = query.edge_types.as_deref();
                let code_paths = &self.code_paths;

                let edge_class_filter = match query.edge_class.as_deref() {
                    Some(s) => EdgeClass::from_str_name(s),
                    None => match modality {
                        Modality::Code => Some(EdgeClass::Code),
                        Modality::Docs => Some(EdgeClass::Semantic),
                        Modality::Both => None,
                    },
                };

                let results = search::search_graph(
                    self.bm25,
                    self.graph,
                    &query.query,
                    limit,
                    max_depth,
                    edge_type_filter,
                    edge_class_filter,
                    modality,
                    code_paths,
                )?;
                Ok(results)
            }
            other => Err(Error::Config(format!(
                "invalid search mode '{}': expected one of bm25, semantic, hybrid, graph, explain, fast",
                other
            ))),
        }?;

        for r in &mut results {
            r.graph = self.graph.format_cypher_affordances(&r.path, 3);
            r.graph_affordances = None;
        }

        Ok(results)
    }

    fn explain(&self, query: &SearchQuery) -> Result<Vec<SearchExplanation>> {
        let limit = query.limit.unwrap_or(10);
        let modality = query.modality;

        let code_paths = &self.code_paths;

        if query.mode.as_deref() == Some("fast") {
            let edge_class_filter = query.edge_class.as_deref().and_then(EdgeClass::from_str_name);
            let empty_binary = BinarySearchIndex::new();
            let binary = self.binary_index.unwrap_or(&empty_binary);
            return search::search_explain_fast(
                self.bm25,
                binary,
                self.graph,
                &query.query,
                limit,
                modality,
                edge_class_filter,
                code_paths,
            );
        }

        let graph_depth = query.graph_depth.unwrap_or(2);
        let edge_type_filter = query.edge_types.as_deref();
        let edge_class_filter = query.edge_class.as_deref().and_then(EdgeClass::from_str_name);

        // Try to get a query embedding for full 3-signal explanation.
        let query_embedding =
            self.embedder.as_deref().and_then(|embedder| embedder.embed_query(&query.query).ok());

        let dummy_vi;
        let vector_index = match self.vector_index {
            Some(vi) => vi,
            None => {
                dummy_vi = VectorIndex::new_default(384);
                &dummy_vi
            }
        };

        let explanations = search::search_explain(
            self.bm25,
            vector_index,
            self.graph,
            &query.query,
            query_embedding.as_deref(),
            limit,
            graph_depth,
            edge_type_filter,
            edge_class_filter,
            modality,
            code_paths,
        )?;

        Ok(explanations)
    }

    fn search_related(
        &self,
        seeds: &[String],
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<SearchResult>> {
        let mut results =
            search::search_related(self.graph, seeds, limit, 0.85, 20, modality, &self.code_paths)?;
        for r in &mut results {
            r.graph = self.graph.format_cypher_affordances(&r.path, 3);
            r.graph_affordances = None;
        }
        Ok(results)
    }
}
