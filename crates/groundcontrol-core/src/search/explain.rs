//! Explain search strategy — full scoring breakdown across BM25, vector, and graph.

use std::collections::{HashMap, HashSet};

use groundcontrol_common::config::EdgeClass;
use groundcontrol_common::ports::{GraphStore, TextIndex, VectorStore};
use groundcontrol_common::types::{
    GraphExplanation, Modality, SearchExplanation, SignalExplanation,
};
use groundcontrol_common::Result;

use super::fusion::path_matches_modality;

/// Full scoring breakdown search — returns detailed explanations per result.
///
/// Runs the 3-signal hybrid search (BM25 + vector + graph) and provides
/// per-result breakdown of each signal's raw score, rank, and RRF contribution.
pub fn search_explain(
    bm25: &impl TextIndex,
    vector_index: &impl VectorStore,
    graph: &impl GraphStore,
    query: &str,
    query_embedding: Option<&[f32]>,
    limit: usize,
    graph_depth: usize,
    edge_type_filter: Option<&[String]>,
    edge_class_filter: Option<EdgeClass>,
    modality: Modality,
    code_paths: &HashSet<String>,
) -> Result<Vec<SearchExplanation>> {
    if modality == Modality::Both {
        let doc_explanations = search_explain_single(
            bm25,
            vector_index,
            graph,
            query,
            query_embedding,
            limit,
            graph_depth,
            edge_type_filter,
            edge_class_filter,
            Modality::Docs,
            code_paths,
        )?;

        let code_explanations = search_explain_single(
            bm25,
            vector_index,
            graph,
            query,
            query_embedding,
            limit,
            graph_depth,
            edge_type_filter,
            edge_class_filter,
            Modality::Code,
            code_paths,
        )?;

        let mut combined = doc_explanations;
        combined.extend(code_explanations);
        return Ok(combined);
    }

    search_explain_single(
        bm25,
        vector_index,
        graph,
        query,
        query_embedding,
        limit,
        graph_depth,
        edge_type_filter,
        edge_class_filter,
        modality,
        code_paths,
    )
}

fn search_explain_single(
    bm25: &impl TextIndex,
    vector_index: &impl VectorStore,
    graph: &impl GraphStore,
    query: &str,
    query_embedding: Option<&[f32]>,
    limit: usize,
    graph_depth: usize,
    edge_type_filter: Option<&[String]>,
    edge_class_filter: Option<EdgeClass>,
    modality: Modality,
    code_paths: &HashSet<String>,
) -> Result<Vec<SearchExplanation>> {
    const RRF_K: f64 = 60.0;

    // 1. Get BM25 results (modality-filtered).
    let bm25_results = bm25.search_with_modality(query, limit * 3, modality)?;

    // 2. Get vector results (if embedding available), modality-filtered.
    let vector_results = if let Some(emb) = query_embedding {
        vector_index.search(emb, limit * 3, false, modality)?
    } else {
        Vec::new()
    };

    // 3. Build per-path BM25 signal info (score + rank).
    let mut bm25_info: HashMap<String, (f64, usize, Option<String>, Option<usize>)> =
        HashMap::new(); // path -> (score, rank_1based, snippet, chunk_index)
    for (rank, r) in bm25_results.iter().enumerate() {
        let _ = bm25_info.entry(r.path.clone()).or_insert((
            r.score,
            rank + 1,
            r.snippet.clone(),
            r.chunk_index,
        ));
    }

    // 4. Build per-path vector signal info.
    let mut vector_info: HashMap<String, (f64, usize)> = HashMap::new(); // path -> (score, rank_1based)
    for (rank, vr) in vector_results.iter().enumerate() {
        let _ = vector_info.entry(vr.doc_path.clone()).or_insert((vr.score, rank + 1));
    }

    // 5. Graph expansion from all seeds.
    let all_seed_paths: HashSet<String> =
        bm25_info.keys().chain(vector_info.keys()).cloned().collect();

    let mut graph_boost_map: HashMap<String, (f64, usize)> = HashMap::new();
    for seed_path in &all_seed_paths {
        let neighbors =
            graph.traverse_bfs(seed_path, graph_depth, edge_type_filter, edge_class_filter);
        for (neighbor_path, hops) in neighbors {
            if hops == 0 {
                continue;
            }
            let boost = 1.0 / (hops as f64);
            let entry = graph_boost_map.entry(neighbor_path).or_insert((0.0, hops));
            entry.0 += boost;
            if hops < entry.1 {
                entry.1 = hops;
            }
        }
    }

    // Sort graph entries by boost to assign ranks.
    let mut graph_ranked: Vec<(String, f64, usize)> =
        graph_boost_map.iter().map(|(path, &(boost, hops))| (path.clone(), boost, hops)).collect();
    graph_ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut graph_rank_map: HashMap<String, (f64, usize, usize)> = HashMap::new(); // path -> (boost, hops, rank_1based)
    for (rank, (path, boost, hops)) in graph_ranked.iter().enumerate() {
        let _ = graph_rank_map.insert(path.clone(), (*boost, *hops, rank + 1));
    }

    // 6. Collect all unique paths.
    let all_paths: HashSet<String> =
        bm25_info.keys().chain(vector_info.keys()).chain(graph_rank_map.keys()).cloned().collect();

    // 7. Build explanations with RRF scores.
    let mut explanations: Vec<SearchExplanation> = all_paths
        .into_iter()
        .map(|path| {
            let (bm25_score, bm25_rank, snippet, chunk_index) =
                bm25_info.get(&path).cloned().unwrap_or((0.0, 0, None, None));

            let (vector_score, vector_rank) = vector_info.get(&path).copied().unwrap_or((0.0, 0));

            let (graph_boost, graph_hops, graph_rank) =
                graph_rank_map.get(&path).copied().unwrap_or((0.0, 0, 0));

            let bm25_rrf = if bm25_rank > 0 { 1.0 / (RRF_K + bm25_rank as f64) } else { 0.0 };
            let vector_rrf = if vector_rank > 0 { 1.0 / (RRF_K + vector_rank as f64) } else { 0.0 };
            let graph_rrf = if graph_rank > 0 { 1.0 / (RRF_K + graph_rank as f64) } else { 0.0 };

            let final_score = bm25_rrf + vector_rrf + graph_rrf;

            SearchExplanation {
                path,
                final_score,
                bm25: SignalExplanation {
                    raw_score: bm25_score,
                    rank: bm25_rank,
                    rrf_contribution: bm25_rrf,
                },
                vector: SignalExplanation {
                    raw_score: vector_score,
                    rank: vector_rank,
                    rrf_contribution: vector_rrf,
                },
                graph: GraphExplanation {
                    boost: graph_boost,
                    min_hops: if graph_hops > 0 { Some(graph_hops) } else { None },
                    rank: graph_rank,
                    rrf_contribution: graph_rrf,
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
            .then_with(|| {
                let a_direct = a.bm25.raw_score + a.vector.raw_score;
                let b_direct = b.bm25.raw_score + b.vector.raw_score;
                b_direct.partial_cmp(&a_direct).unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.path.cmp(&b.path))
    });
    // Modality filter: graph-expanded paths may introduce the other modality.
    explanations.retain(|e| path_matches_modality(&e.path, modality, code_paths));
    explanations.truncate(limit);

    Ok(explanations)
}
