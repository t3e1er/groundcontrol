//! Multi-hop query decomposition search strategy.

use std::collections::{HashMap, HashSet};

use ctxvault_common::config::EdgeClass;
use ctxvault_common::ports::{EmbeddingProvider, GraphStore, TextIndex, VectorStore};
use ctxvault_common::types::{Modality, ScoreBreakdown, SearchResult};
use ctxvault_common::Result;

use super::fusion::{enrich_results_with_lineage, path_matches_modality, rrf_fuse};
use super::hybrid::search_hybrid_full;

/// Multi-hop query decomposition search.
///
/// Strategy:
/// 1. DECOMPOSE: Split query on connecting words into sub-concepts
/// 2. SEARCH: Run BM25 + vector for each sub-concept separately
/// 3. MERGE: RRF fusion across all sub-concept result lists
/// 4. BOOST: Documents appearing in multiple sub-concept results get score multiplier
/// 5. BRIDGE: Find structural paths between seed docs from different sub-concepts
///
/// Falls back to normal hybrid search if query cannot be decomposed.
pub fn search_multihop<E: EmbeddingProvider>(
    bm25: &impl TextIndex,
    vector_index: &impl VectorStore,
    graph: &impl GraphStore,
    embedder: Option<&E>,
    query: &str,
    query_embedding: Option<&[f32]>,
    limit: usize,
    graph_depth: usize,
    edge_type_filter: Option<&[String]>,
    modality: Modality,
    code_paths: &HashSet<String>,
) -> Result<Vec<SearchResult>> {
    const RRF_K: f64 = 60.0;

    // 1. Decompose query into sub-concepts.
    let sub_concepts = decompose_query(query);

    // If decomposition yields only one concept, fall back to hybrid_full with semantic edges.
    if sub_concepts.len() <= 1 {
        return search_hybrid_full(
            bm25,
            vector_index,
            graph,
            query,
            query_embedding,
            limit,
            graph_depth,
            edge_type_filter,
            Some(EdgeClass::Semantic),
            modality,
            code_paths,
        );
    }

    // 2. Search each sub-concept separately with BM25 + Vector.
    let mut all_result_lists: Vec<Vec<SearchResult>> = Vec::new();
    let mut concept_doc_sets: Vec<HashSet<String>> = Vec::new();

    for concept in &sub_concepts {
        let clean_concept =
            concept.trim_matches(|c: char| c == '?' || c == '.' || c == '!' || c == ',').trim();

        // BM25 search for this sub-concept (modality-filtered).
        let bm25_results = bm25.search_with_modality(clean_concept, limit * 2, modality)?;

        // Vector search for this sub-concept if embedder is available.
        let concept_list = if let Some(emb_model) = embedder {
            if let Ok(emb) = emb_model.embed_query(clean_concept) {
                let vec_res = vector_index.search(&emb, limit * 2, false, modality)?;
                let vec_search: Vec<SearchResult> = vec_res
                    .into_iter()
                    .map(|vr| {
                        SearchResult::new(vr.doc_path, vr.score).with_chunk_index(vr.chunk_index)
                    })
                    .collect();
                rrf_fuse(&[&bm25_results, &vec_search], limit * 2)
            } else {
                bm25_results
            }
        } else {
            bm25_results
        };

        // Track which docs appear for this concept.
        let doc_set: HashSet<String> = concept_list.iter().map(|r| r.path.clone()).collect();
        concept_doc_sets.push(doc_set);
        all_result_lists.push(concept_list);
    }

    // Also add the full-query BM25 results as another signal (modality-filtered).
    let full_bm25 = bm25.search_with_modality(query, limit * 2, modality)?;
    all_result_lists.push(full_bm25);

    // And full-query vector results if available.
    if let Some(emb) = query_embedding {
        let vector_results = vector_index.search(emb, limit * 2, false, modality)?;
        let vector_as_search: Vec<SearchResult> = vector_results
            .into_iter()
            .map(|vr| SearchResult::new(vr.doc_path, vr.score).with_chunk_index(vr.chunk_index))
            .collect();
        all_result_lists.push(vector_as_search);
    }

    // 3. Structural bridging: find docs on structural paths between concept seeds.
    // Take top seed from each concept and find structural paths between them.
    let mut bridge_docs: HashMap<String, f64> = HashMap::new();

    if concept_doc_sets.len() >= 2 {
        // Get top-3 seeds from each concept.
        let concept_seeds: Vec<Vec<String>> = all_result_lists
            .iter()
            .take(sub_concepts.len()) // Only concept-specific lists
            .map(|list| list.iter().take(3).map(|r| r.path.clone()).collect())
            .collect();

        // For each pair of concept seed sets, find structural paths.
        for i in 0..concept_seeds.len() {
            for j in (i + 1)..concept_seeds.len() {
                for seed_a in &concept_seeds[i] {
                    for seed_b in &concept_seeds[j] {
                        // Find shortest structural path between seeds.
                        if let Some(path_nodes) = graph.shortest_path(
                            seed_a,
                            seed_b,
                            edge_type_filter,
                            Some(EdgeClass::Structural),
                        ) {
                            // Intermediate nodes on the path are bridge documents.
                            for node in &path_nodes {
                                if node != seed_a && node != seed_b {
                                    let boost = 1.0 / (path_nodes.len() as f64);
                                    *bridge_docs.entry(node.clone()).or_insert(0.0) += boost;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Add bridge documents as a rank-normalized list to RRF fusion.
    if !bridge_docs.is_empty() {
        let mut sorted_bridges: Vec<(String, f64)> = bridge_docs.clone().into_iter().collect();
        sorted_bridges.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let bridge_as_search: Vec<SearchResult> = sorted_bridges
            .into_iter()
            .map(|(path, score)| SearchResult::new(path, score))
            .collect();
        all_result_lists.push(bridge_as_search);
    }

    // 4. RRF fusion across all result lists with weights.
    let mut rrf_scores: HashMap<String, (f64, Option<String>, Option<usize>)> = HashMap::new();

    for (list_idx, list) in all_result_lists.iter().enumerate() {
        let weight = if list_idx < sub_concepts.len() {
            1.0 // Sub-concept results get full weight
        } else if list_idx < sub_concepts.len() + 2 {
            0.5 // Full-query BM25 / Vector get standard support weight
        } else {
            0.2 // Structural bridge docs get subtle support weight
        };

        for (rank, result) in list.iter().enumerate() {
            let rrf_contribution = weight / (RRF_K + rank as f64 + 1.0);
            let entry = rrf_scores.entry(result.path.clone()).or_insert((
                0.0,
                result.snippet.clone(),
                result.chunk_index,
            ));
            entry.0 += rrf_contribution;
        }
    }

    // 5. Multi-concept boost: multiply score by number of concepts a doc appears in.
    for (path, (score, _, _)) in rrf_scores.iter_mut() {
        let concept_count =
            concept_doc_sets.iter().filter(|set| set.contains(path.as_str())).count();
        if concept_count > 1 {
            *score *= concept_count as f64;
        }
    }

    // 6. Build final results, sort, truncate.
    let mut results: Vec<SearchResult> = rrf_scores
        .into_iter()
        .map(|(path, (score, snippet, chunk_index))| {
            let is_bridge = bridge_docs.contains_key(&path);
            SearchResult::new(path, score)
                .with_snippet(snippet)
                .with_chunk_index(chunk_index)
                .with_score_components(ScoreBreakdown {
                    bm25: 0.0,
                    vector: 0.0,
                    graph_boost: if is_bridge { 1.0 } else { 0.0 },
                    graph_hops: None,
                })
        })
        .collect();

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    // Modality filter: structural bridge nodes may introduce the other modality.
    results.retain(|r| path_matches_modality(&r.path, modality, code_paths));
    results.truncate(limit);
    enrich_results_with_lineage(&mut results, graph);

    Ok(results)
}

/// Decompose a complex query into sub-concepts by splitting on connecting words.
///
/// Returns a vector of sub-concept strings. If no splitting is possible,
/// returns a single-element vector with the original query.
pub fn decompose_query(query: &str) -> Vec<String> {
    // Connecting patterns to split on (order matters — try longer patterns first).
    let patterns = [
        " relate to ",
        " related to ",
        " relates to ",
        " connect to ",
        " connects to ",
        " connected to ",
        " connection between ",
        " relationship between ",
        " between ",
        " through ",
        " connect ",
        " and ",
        " to ",
        " from ",
    ];

    let query_clean = strip_question_prefix(query);

    // Try each pattern.
    for pattern in &patterns {
        if let Some(pos) = query_clean.to_lowercase().find(pattern) {
            let left = query_clean[..pos].trim();
            let right = query_clean[pos + pattern.len()..].trim();

            let left_clean = strip_question_prefix(left);
            let right_clean = strip_question_prefix(right);

            if left_clean.len() >= 3 && right_clean.len() >= 3 && left_clean != "the path" {
                let mut concepts = vec![left_clean.to_string()];
                let right_parts = decompose_query(right_clean);
                for part in right_parts {
                    if part != "the path" && part.len() >= 3 {
                        concepts.push(part);
                    }
                }
                return concepts;
            }
        }
    }

    // No decomposition possible — return the whole query (stripped of question prefix).
    if query_clean.len() >= 3 {
        vec![query_clean.to_string()]
    } else {
        vec![query.to_string()]
    }
}

/// Strip common question prefixes like "How do", "What is", etc.
fn strip_question_prefix(s: &str) -> &str {
    let prefixes = [
        "what is the path from ",
        "what is the path between ",
        "what is the path through ",
        "path from ",
        "how does ",
        "how do ",
        "how can ",
        "how is ",
        "what is ",
        "what are ",
        "what does ",
        "what do ",
        "what ",
        "how ",
        "why does ",
        "why do ",
        "why is ",
        "where does ",
        "where do ",
    ];

    let lower = s.to_lowercase();
    for prefix in &prefixes {
        if lower.starts_with(prefix) {
            return &s[prefix.len()..];
        }
    }
    s
}
