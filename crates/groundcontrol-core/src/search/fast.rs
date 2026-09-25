//! Fast algorithmic hybrid search (BM25 + Binary Hamming Scan + PPR).

use std::collections::{HashMap, HashSet};

use groundcontrol_common::config::EdgeClass;
use groundcontrol_common::ports::TextIndex;
use groundcontrol_common::types::{
    EntityKind, GraphExplanation, Modality, ScoreBreakdown, SearchExplanation, SearchResult,
    SignalExplanation,
};
use groundcontrol_common::Result;

use super::fusion::{enrich_results_with_lineage, path_matches_modality};
use crate::algorithm::binaryv3::BinaryV3SearchIndex;

/// Fast algorithmic hybrid search: Tantivy BM25 + Binary Hamming Scan + Query-Time PPR (3-way RRF).
///
/// Executes sub-minute CPU SIF projection, SIMD POPCOUNT Hamming scan, and 2-hop
/// HippoRAG diffusion over Petgraph with zero ONNX neural inference in <2ms.
pub fn search_fast(
    bm25: &impl TextIndex,
    binary_index: &BinaryV3SearchIndex,
    graph: &crate::graph::KnowledgeGraph,
    query: &str,
    limit: usize,
    modality: Modality,
    edge_class_filter: Option<EdgeClass>,
    code_paths: &HashSet<String>,
) -> Result<Vec<SearchResult>> {
    const RRF_K: f64 = 60.0;

    // 1. Lexical BM25 candidates
    let bm25_results = bm25.search_with_modality(query, limit * 3, modality)?;
    let mut bm25_info: HashMap<String, (f64, usize, Option<String>, Option<usize>)> =
        HashMap::new();
    for (rank, r) in bm25_results.iter().enumerate() {
        bm25_info.entry(r.path.clone()).or_insert((
            r.score,
            rank + 1,
            r.snippet.clone(),
            r.chunk_index,
        ));
    }

    // 2. 256-Bit MRL Binary Hamming scan candidates with Bayesian prior
    let query_fp = binary_index.project_query(query);
    let binary_hits = binary_index.search_candidates(&query_fp, limit * 3, modality)?;
    let mut binary_info: HashMap<String, (f64, usize)> = HashMap::new();
    for (rank, (id, _dist, score)) in binary_hits.into_iter().enumerate() {
        let mut clean_path = id.as_str();
        if let Some(idx) = clean_path.find(":chunk:") {
            clean_path = &clean_path[..idx];
        }
        if let Some(idx) = clean_path.find('#') {
            clean_path = &clean_path[..idx];
        }
        binary_info.entry(clean_path.to_string()).or_insert((score, rank + 1));
    }

    // 3. Form seeds for Query-Time Personalized PageRank (HippoRAG diffusion)
    let mut seed_scores: Vec<(String, f64)> = Vec::new();
    let mut all_candidate_paths: HashSet<String> = bm25_info.keys().cloned().collect();
    for id in binary_info.keys() {
        all_candidate_paths.insert(id.clone());
    }

    for path in &all_candidate_paths {
        let bm25_score = bm25_info.get(path).map(|(s, ..)| *s).unwrap_or(0.0);
        let binary_score = binary_info.get(path).map(|(s, _)| *s).unwrap_or(0.0);
        let seed_score = bm25_score + binary_score;
        if seed_score > 0.0 {
            seed_scores.push((path.clone(), seed_score));
        }
    }

    // 4. Query-time PPR diffusion on Petgraph
    let ppr_scores = crate::graph::diffusion::personalized_pagerank(
        graph,
        &seed_scores,
        crate::graph::diffusion::PPR_DEFAULT_ALPHA,
        crate::graph::diffusion::PPR_DEFAULT_ITERATIONS,
        edge_class_filter,
    );

    let mut ppr_info: HashMap<String, (f64, usize)> = HashMap::new();
    for (rank, p) in ppr_scores.iter().enumerate() {
        ppr_info.insert(p.path.clone(), (p.score, rank + 1));
        all_candidate_paths.insert(p.path.clone());
    }

    // 5. 3-Way Reciprocal Rank Fusion
    let mut results: Vec<SearchResult> = all_candidate_paths
        .into_iter()
        .map(|path| {
            let (bm25_score, bm25_rank, snippet, chunk_index) =
                bm25_info.get(&path).cloned().unwrap_or((0.0, 0, None, None));
            let (binary_score, binary_rank) = binary_info.get(&path).copied().unwrap_or((0.0, 0));
            let (ppr_score, ppr_rank) = ppr_info.get(&path).copied().unwrap_or((0.0, 0));

            let bm25_rrf = if bm25_rank > 0 { 1.0 / (RRF_K + bm25_rank as f64) } else { 0.0 };
            let binary_rrf = if binary_rank > 0 { 1.0 / (RRF_K + binary_rank as f64) } else { 0.0 };
            let ppr_rrf = if ppr_rank > 0 { 1.0 / (RRF_K + ppr_rank as f64) } else { 0.0 };

            let final_score = bm25_rrf + binary_rrf + ppr_rrf;

            let is_code = path_matches_modality(&path, Modality::Code, code_paths);

            let mut res = SearchResult::new(path, final_score)
                .with_snippet(snippet)
                .with_chunk_index(chunk_index)
                .with_score_components(ScoreBreakdown {
                    bm25: bm25_score,
                    vector: binary_score,
                    graph_boost: ppr_score,
                    graph_hops: if ppr_rank > 0 {
                        Some(if ppr_rank <= limit { 1 } else { 2 })
                    } else {
                        None
                    },
                });

            if is_code {
                res.entity_kind = Some(EntityKind::CodeChunk {
                    language: String::new(),
                    scope_path: String::new(),
                    start_line: 0,
                    end_line: 0,
                });
            } else {
                res.entity_kind = Some(EntityKind::Documentation);
            }
            res
        })
        .collect();

    // 6. Sort descending by score
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });

    results.retain(|r| path_matches_modality(&r.path, modality, code_paths));
    results.truncate(limit);
    enrich_results_with_lineage(&mut results, graph);

    Ok(results)
}

/// Explain scoring breakdown for fast algorithmic search.
pub fn search_explain_fast(
    bm25: &impl TextIndex,
    binary_index: &BinaryV3SearchIndex,
    graph: &crate::graph::KnowledgeGraph,
    query: &str,
    limit: usize,
    modality: Modality,
    edge_class_filter: Option<EdgeClass>,
    code_paths: &HashSet<String>,
) -> Result<Vec<SearchExplanation>> {
    const RRF_K: f64 = 60.0;

    let bm25_results = bm25.search_with_modality(query, limit * 3, modality)?;
    let mut bm25_info: HashMap<String, (f64, usize, Option<String>, Option<usize>)> =
        HashMap::new();
    for (rank, r) in bm25_results.iter().enumerate() {
        bm25_info.entry(r.path.clone()).or_insert((
            r.score,
            rank + 1,
            r.snippet.clone(),
            r.chunk_index,
        ));
    }

    let query_fp = binary_index.project_query(query);
    let binary_hits = binary_index.search_candidates(&query_fp, limit * 3, modality)?;
    let mut binary_info: HashMap<String, (f64, usize)> = HashMap::new();
    for (rank, (id, _dist, score)) in binary_hits.into_iter().enumerate() {
        let mut clean_path = id.as_str();
        if let Some(idx) = clean_path.find(":chunk:") {
            clean_path = &clean_path[..idx];
        }
        if let Some(idx) = clean_path.find('#') {
            clean_path = &clean_path[..idx];
        }
        binary_info.entry(clean_path.to_string()).or_insert((score, rank + 1));
    }

    let mut seed_scores: Vec<(String, f64)> = Vec::new();
    let mut all_candidate_paths: HashSet<String> = bm25_info.keys().cloned().collect();
    for id in binary_info.keys() {
        all_candidate_paths.insert(id.clone());
    }

    for path in &all_candidate_paths {
        let bm25_score = bm25_info.get(path).map(|(s, ..)| *s).unwrap_or(0.0);
        let binary_score = binary_info.get(path).map(|(s, _)| *s).unwrap_or(0.0);
        let seed_score = bm25_score + binary_score;
        if seed_score > 0.0 {
            seed_scores.push((path.clone(), seed_score));
        }
    }

    let ppr_scores = crate::graph::diffusion::personalized_pagerank(
        graph,
        &seed_scores,
        crate::graph::diffusion::PPR_DEFAULT_ALPHA,
        crate::graph::diffusion::PPR_DEFAULT_ITERATIONS,
        edge_class_filter,
    );

    let mut ppr_info: HashMap<String, (f64, usize)> = HashMap::new();
    for (rank, p) in ppr_scores.iter().enumerate() {
        ppr_info.insert(p.path.clone(), (p.score, rank + 1));
        all_candidate_paths.insert(p.path.clone());
    }

    let mut explanations: Vec<SearchExplanation> = all_candidate_paths
        .into_iter()
        .map(|path| {
            let (bm25_score, bm25_rank, snippet, chunk_index) =
                bm25_info.get(&path).cloned().unwrap_or((0.0, 0, None, None));
            let (binary_score, binary_rank) = binary_info.get(&path).copied().unwrap_or((0.0, 0));
            let (ppr_score, ppr_rank) = ppr_info.get(&path).copied().unwrap_or((0.0, 0));

            let bm25_rrf = if bm25_rank > 0 { 1.0 / (RRF_K + bm25_rank as f64) } else { 0.0 };
            let binary_rrf = if binary_rank > 0 { 1.0 / (RRF_K + binary_rank as f64) } else { 0.0 };
            let ppr_rrf = if ppr_rank > 0 { 1.0 / (RRF_K + ppr_rank as f64) } else { 0.0 };

            let final_score = bm25_rrf + binary_rrf + ppr_rrf;

            SearchExplanation {
                path,
                final_score,
                bm25: SignalExplanation {
                    raw_score: bm25_score,
                    rank: bm25_rank,
                    rrf_contribution: bm25_rrf,
                },
                vector: SignalExplanation {
                    raw_score: binary_score,
                    rank: binary_rank,
                    rrf_contribution: binary_rrf,
                },
                graph: GraphExplanation {
                    boost: ppr_score,
                    min_hops: if ppr_rank > 0 {
                        Some(if ppr_rank <= limit { 1 } else { 2 })
                    } else {
                        None
                    },
                    rank: ppr_rank,
                    rrf_contribution: ppr_rrf,
                },
                snippet,
                chunk_index,
            }
        })
        .collect();

    explanations.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });

    explanations.retain(|e| path_matches_modality(&e.path, modality, code_paths));
    explanations.truncate(limit);

    Ok(explanations)
}
