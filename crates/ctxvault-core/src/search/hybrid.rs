//! Hybrid search strategies: BM25 + Graph proximity and 3-way BM25 + Vector + Graph fusion.

use std::collections::{HashMap, HashSet};

use ctxvault_common::config::EdgeClass;
use ctxvault_common::ports::{GraphStore, TextIndex, VectorStore};
use ctxvault_common::types::{EntityKind, Modality, ScoreBreakdown, SearchResult};
use ctxvault_common::Result;

use super::fusion::{enrich_results_with_lineage, path_matches_modality};

/// Hybrid search: seeds from BM25, then boosts scores based on graph proximity.
///
/// Strategy:
/// 1. SEED: BM25(query, limit * 3)
/// 2. EXPAND: For each seed, BFS over edges to depth `graph_depth`
/// 3. RANK: RRF fusion of BM25 rank + graph proximity rank (k=60)
/// 4. RETURN: Top `limit` results
///
/// Note: This is the BM25+graph variant. For true 3-signal hybrid (BM25+vector+graph),
/// use `search_hybrid_full`.
pub fn search_hybrid(
    bm25: &impl TextIndex,
    graph: &impl GraphStore,
    query: &str,
    limit: usize,
    graph_depth: usize,
    edge_type_filter: Option<&[String]>,
    edge_class_filter: Option<EdgeClass>,
    modality: Modality,
    code_paths: &HashSet<String>,
) -> Result<Vec<SearchResult>> {
    if modality == Modality::Both {
        let doc_results = search_hybrid_single(
            bm25,
            graph,
            query,
            limit,
            graph_depth,
            edge_type_filter,
            edge_class_filter,
            Modality::Docs,
            code_paths,
        )?;

        let code_results = search_hybrid_single(
            bm25,
            graph,
            query,
            limit,
            graph_depth,
            edge_type_filter,
            edge_class_filter,
            Modality::Code,
            code_paths,
        )?;

        let mut combined = doc_results;
        combined.extend(code_results);
        return Ok(combined);
    }

    search_hybrid_single(
        bm25,
        graph,
        query,
        limit,
        graph_depth,
        edge_type_filter,
        edge_class_filter,
        modality,
        code_paths,
    )
}

fn search_hybrid_single(
    bm25: &impl TextIndex,
    graph: &impl GraphStore,
    query: &str,
    limit: usize,
    graph_depth: usize,
    edge_type_filter: Option<&[String]>,
    edge_class_filter: Option<EdgeClass>,
    modality: Modality,
    code_paths: &HashSet<String>,
) -> Result<Vec<SearchResult>> {
    const RRF_K: f64 = 60.0;

    // 1. Get BM25 seeds (over-fetch to allow graph reranking), modality-filtered.
    let bm25_results = bm25.search_with_modality(query, limit * 3, modality)?;

    if bm25_results.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Build BM25 rank map: key -> (raw_score, rank_1based, snippet, chunk_index).
    let mut bm25_info: HashMap<
        (String, Option<usize>),
        (f64, usize, Option<String>, Option<usize>),
    > = HashMap::new();
    for (rank, r) in bm25_results.iter().enumerate() {
        let key = if modality == Modality::Code {
            (r.path.clone(), r.chunk_index)
        } else {
            (r.path.clone(), None)
        };
        let _ =
            bm25_info.entry(key).or_insert((r.score, rank + 1, r.snippet.clone(), r.chunk_index));
    }

    // 3. Graph expansion: BFS from each BM25 seed, accumulate proximity scores.
    let mut graph_boost_map: HashMap<String, (f64, usize)> = HashMap::new(); // path -> (total_boost, min_hops)

    for r in &bm25_results {
        let neighbors =
            graph.traverse_bfs(&r.path, graph_depth, edge_type_filter, edge_class_filter);
        for (neighbor_path, hops) in neighbors {
            if hops == 0 {
                continue;
            }
            // Hub suppression / in-degree penalty to guard against noisy neighbors (O(1) in-degree)
            let in_degree = graph.in_degree(&neighbor_path);
            let hub_dampener = 1.0 / (1.0 + (in_degree as f64 / 10.0)).sqrt();
            let boost = (1.0 / (hops as f64)) * hub_dampener;
            let entry = graph_boost_map.entry(neighbor_path).or_insert((0.0, hops));
            entry.0 += boost;
            if hops < entry.1 {
                entry.1 = hops;
            }
        }
    }

    // 4. Rank graph-discovered nodes by proximity score.
    let mut graph_ranked: Vec<(String, f64, usize)> =
        graph_boost_map.into_iter().map(|(path, (boost, hops))| (path, boost, hops)).collect();
    graph_ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut graph_rank_map: HashMap<(String, Option<usize>), (f64, usize, usize)> = HashMap::new(); // key -> (boost, min_hops, rank_1based)
    for (rank, (path, boost, hops)) in graph_ranked.iter().enumerate() {
        let key = (path.clone(), None);
        let _ = graph_rank_map.insert(key, (*boost, *hops, rank + 1));
    }

    // 5. Collect all unique keys from both signals.
    let all_keys: HashSet<(String, Option<usize>)> =
        bm25_info.keys().chain(graph_rank_map.keys()).cloned().collect();

    // 6. RRF fusion: combine BM25 rank and graph rank.
    let mut results: Vec<SearchResult> = all_keys
        .into_iter()
        .map(|(path, chunk_key)| {
            let (bm25_score, bm25_rank, snippet, chunk_index) =
                bm25_info.get(&(path.clone(), chunk_key)).cloned().unwrap_or((0.0, 0, None, None));

            let (graph_boost, min_hops, graph_rank) =
                graph_rank_map.get(&(path.clone(), None)).copied().unwrap_or((0.0, 0, 0));

            let bm25_rrf = if bm25_rank > 0 { 1.0 / (RRF_K + bm25_rank as f64) } else { 0.0 };
            // Pure graph discoveries receive a higher RRF K denominator (120 vs 60)
            // so they don't displace strong direct keyword matches.
            let k_factor = if bm25_rank == 0 { RRF_K * 2.0 } else { RRF_K };
            let graph_rrf = if graph_rank > 0 { 1.0 / (k_factor + graph_rank as f64) } else { 0.0 };

            let final_score = bm25_rrf + graph_rrf;

            let mut res = SearchResult::new(path, final_score)
                .with_snippet(snippet)
                .with_chunk_index(chunk_index.or(chunk_key))
                .with_score_components(ScoreBreakdown {
                    bm25: bm25_score,
                    vector: 0.0,
                    graph_boost,
                    graph_hops: if min_hops > 0 { Some(min_hops) } else { None },
                });
            if modality == Modality::Code {
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

    // 7. Sort descending by RRF score, with deterministic tie-breaking on direct match then path.
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let a_direct = a.score_components.as_ref().map_or(0.0, |c| c.bm25 + c.vector);
                let b_direct = b.score_components.as_ref().map_or(0.0, |c| c.bm25 + c.vector);
                b_direct.partial_cmp(&a_direct).unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.chunk_index.cmp(&b.chunk_index))
    });
    // Modality filter: graph-expanded paths may introduce the other modality.
    results.retain(|r| path_matches_modality(&r.path, modality, code_paths));
    results.truncate(limit);
    enrich_results_with_lineage(&mut results, graph);

    Ok(results)
}

/// True hybrid search: fuses BM25 + Vector + Graph via Reciprocal Rank Fusion.
///
/// Strategy (from architecture doc):
/// 1. SEED: BM25(query, limit*3) ∪ Vector(query, limit*3)
/// 2. EXPAND: For each seed node, BFS over typed edges to depth D
/// 3. RANK: RRF fusion of:
///    - BM25 score rank
///    - Vector cosine similarity rank
///    - Graph proximity boost (1/hop_distance)
/// 4. RETURN: Top K results with scores + traversal path
///
/// If `query_embedding` is None, falls back to BM25+graph only.
pub fn search_hybrid_full(
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
) -> Result<Vec<SearchResult>> {
    if modality == Modality::Both {
        let doc_results = search_hybrid_full_single(
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

        let code_results = search_hybrid_full_single(
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

        let mut combined = doc_results;
        combined.extend(code_results);
        return Ok(combined);
    }

    search_hybrid_full_single(
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

fn search_hybrid_full_single(
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
) -> Result<Vec<SearchResult>> {
    tracing::debug!("hybrid search: vector results from anchor embeddings, BM25 from full corpus");
    const RRF_K: f64 = 60.0;

    // 1. Get BM25 seeds (modality-filtered).
    let bm25_results = bm25.search_with_modality(query, limit * 3, modality)?;

    // 2. Get vector seeds (if embedding available), modality-filtered.
    let vector_results = if let Some(emb) = query_embedding {
        vector_index.search(emb, limit * 3, false, modality)?
    } else {
        Vec::new()
    };

    // If both are empty, no results.
    if bm25_results.is_empty() && vector_results.is_empty() {
        return Ok(Vec::new());
    }

    // 3. Build RRF scores from BM25 ranked list.
    // Key is (path, None) for all modalities so BM25, vector, and graph fuse at file level.
    let mut rrf_map: HashMap<(String, Option<usize>), (f64, f64, f64, usize)> = HashMap::new(); // key -> (rrf_total, bm25_score, vector_score, min_hops)
    let mut best_bm25_chunk: HashMap<String, (Option<usize>, f64, Option<String>)> = HashMap::new();

    for (rank, r) in bm25_results.iter().enumerate() {
        let rrf_score = 1.0 / (RRF_K + rank as f64 + 1.0);
        let key = (r.path.clone(), None);
        let entry = rrf_map.entry(key).or_insert((0.0, 0.0, 0.0, 0));
        entry.0 += rrf_score;
        entry.1 = r.score.max(entry.1); // highest raw BM25 score

        best_bm25_chunk
            .entry(r.path.clone())
            .and_modify(|e| {
                if r.score > e.1 {
                    *e = (r.chunk_index, r.score, r.snippet.clone());
                }
            })
            .or_insert((r.chunk_index, r.score, r.snippet.clone()));
    }

    // 4. Add RRF scores from vector ranked list.
    let mut best_vector_chunk: HashMap<String, (Option<usize>, f64)> = HashMap::new();
    for (rank, vr) in vector_results.iter().enumerate() {
        let rrf_score = 1.0 / (RRF_K + rank as f64 + 1.0);
        let key = (vr.doc_path.clone(), None);
        let entry = rrf_map.entry(key).or_insert((0.0, 0.0, 0.0, 0));
        entry.0 += rrf_score;
        entry.2 = vr.score.max(entry.2); // cosine similarity

        best_vector_chunk
            .entry(vr.doc_path.clone())
            .and_modify(|e| {
                if vr.score > e.1 {
                    *e = (vr.chunk_index, vr.score);
                }
            })
            .or_insert((vr.chunk_index, vr.score));
    }

    // 5. Graph expansion: BFS from all seed paths to add graph boost.
    let seed_paths: HashSet<String> = rrf_map.keys().map(|k| k.0.clone()).collect();
    let mut graph_boost_map: HashMap<String, (f64, usize)> = HashMap::new();

    for seed_path in &seed_paths {
        let neighbors =
            graph.traverse_bfs(seed_path, graph_depth, edge_type_filter, edge_class_filter);
        for (neighbor_path, hops) in neighbors {
            if hops == 0 {
                continue;
            }
            // Hub suppression / in-degree penalty to guard against noisy neighbors (O(1) in-degree)
            let in_degree = graph.in_degree(&neighbor_path);
            let hub_dampener = 1.0 / (1.0 + (in_degree as f64 / 10.0)).sqrt();
            let boost = (1.0 / (hops as f64)) * hub_dampener;
            let entry = graph_boost_map.entry(neighbor_path).or_insert((0.0, hops));
            entry.0 += boost;
            if hops < entry.1 {
                entry.1 = hops;
            }
        }
    }

    // 6. Add graph boost as a third signal via RRF-style scoring.
    let mut graph_ranked: Vec<(String, f64, usize)> =
        graph_boost_map.into_iter().map(|(path, (boost, hops))| (path, boost, hops)).collect();
    graph_ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for (rank, (path, boost, hops)) in graph_ranked.iter().enumerate() {
        let key = (path.clone(), None);
        let is_pure_graph = !rrf_map.contains_key(&key);
        let k_factor = if is_pure_graph { RRF_K * 2.0 } else { RRF_K };
        let rrf_score = 1.0 / (k_factor + rank as f64 + 1.0);
        let entry = rrf_map.entry(key).or_insert((0.0, 0.0, 0.0, 0));
        entry.0 += rrf_score;
        if *hops > 0 && (entry.3 == 0 || *hops < entry.3) {
            entry.3 = *hops;
        }
        let _ = boost;
    }

    // 7. Build final results.
    let mut results: Vec<SearchResult> = rrf_map
        .into_iter()
        .map(|((path, _), (rrf_total, bm25_score, vector_score, min_hops))| {
            let (best_chunk_index, _, best_snippet) =
                best_bm25_chunk.get(&path).cloned().unwrap_or_else(|| {
                    let vec_chunk = best_vector_chunk.get(&path).and_then(|(ci, _)| *ci);
                    (vec_chunk, 0.0, None)
                });
            let mut res = SearchResult::new(path, rrf_total)
                .with_snippet(best_snippet)
                .with_chunk_index(best_chunk_index)
                .with_score_components(ScoreBreakdown {
                    bm25: bm25_score,
                    vector: vector_score,
                    graph_boost: if min_hops > 0 { 1.0 / (min_hops as f64) } else { 0.0 },
                    graph_hops: if min_hops > 0 { Some(min_hops) } else { None },
                });
            if modality == Modality::Code {
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

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let a_direct = a.score_components.as_ref().map_or(0.0, |c| c.bm25 + c.vector);
                let b_direct = b.score_components.as_ref().map_or(0.0, |c| c.bm25 + c.vector);
                b_direct.partial_cmp(&a_direct).unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.chunk_index.cmp(&b.chunk_index))
    });
    // Modality filter: graph-expanded paths may introduce the other modality.
    results.retain(|r| path_matches_modality(&r.path, modality, code_paths));
    results.truncate(limit);
    enrich_results_with_lineage(&mut results, graph);

    Ok(results)
}
