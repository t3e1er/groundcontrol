//! Graph traversal and Personalized PageRank (related) search strategies.

use std::collections::{HashMap, HashSet};

use groundcontrol_common::config::EdgeClass;
use groundcontrol_common::ports::{GraphStore, TextIndex};
use groundcontrol_common::types::{EntityKind, Modality, ScoreBreakdown, SearchResult};
use groundcontrol_common::Result;

use super::fusion::{enrich_results_with_lineage, path_matches_modality};

/// Graph search: find nodes matching query text in BM25, then traverse graph from matches.
/// Returns nodes reachable from query matches via typed edges.
pub fn search_graph(
    bm25: &impl TextIndex,
    graph: &impl GraphStore,
    query: &str,
    limit: usize,
    max_depth: usize,
    edge_type_filter: Option<&[String]>,
    edge_class_filter: Option<EdgeClass>,
    modality: Modality,
    code_paths: &HashSet<String>,
) -> Result<Vec<SearchResult>> {
    if modality == Modality::Both {
        let doc_results = search_graph_single(
            bm25,
            graph,
            query,
            limit,
            max_depth,
            edge_type_filter,
            edge_class_filter,
            Modality::Docs,
            code_paths,
        )?;

        let code_results = search_graph_single(
            bm25,
            graph,
            query,
            limit,
            max_depth,
            edge_type_filter,
            edge_class_filter,
            Modality::Code,
            code_paths,
        )?;

        let mut combined = doc_results;
        combined.extend(code_results);
        return Ok(combined);
    }

    search_graph_single(
        bm25,
        graph,
        query,
        limit,
        max_depth,
        edge_type_filter,
        edge_class_filter,
        modality,
        code_paths,
    )
}

fn search_graph_single(
    bm25: &impl TextIndex,
    graph: &impl GraphStore,
    query: &str,
    limit: usize,
    max_depth: usize,
    edge_type_filter: Option<&[String]>,
    edge_class_filter: Option<EdgeClass>,
    modality: Modality,
    code_paths: &HashSet<String>,
) -> Result<Vec<SearchResult>> {
    // 1. Find seed nodes via BM25 (top 5). Seeds themselves are unrestricted so
    //    traversal can bridge modalities; the final results are modality-filtered.
    let seeds = bm25.search(query, 5)?;

    if seeds.is_empty() {
        return Ok(Vec::new());
    }

    // 2. BFS from each seed, accumulating scores.
    let mut score_map: HashMap<String, (f64, usize)> = HashMap::new(); // path -> (total_score, min_hops)

    for seed in &seeds {
        let neighbors =
            graph.traverse_bfs(&seed.path, max_depth, edge_type_filter, edge_class_filter);
        for (path, hops) in neighbors {
            if hops == 0 {
                continue;
            }
            let score = 1.0 / (hops as f64);
            let entry = score_map.entry(path).or_insert((0.0, hops));
            entry.0 += score;
            if hops < entry.1 {
                entry.1 = hops;
            }
        }
    }

    // Remove seeds from results (we want discovered nodes, not the seeds themselves).
    for seed in &seeds {
        let _ = score_map.remove(&seed.path);
    }

    // 3. Build results, sort by score, take top `limit`.
    let mut results: Vec<SearchResult> = score_map
        .into_iter()
        .map(|(path, (score, min_hops))| {
            let mut res = SearchResult::new(path, score).with_score_components(ScoreBreakdown {
                bm25: 0.0,
                vector: 0.0,
                graph_boost: score,
                graph_hops: Some(min_hops),
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

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.retain(|r| path_matches_modality(&r.path, modality, code_paths));
    results.truncate(limit);
    enrich_results_with_lineage(&mut results, graph);

    Ok(results)
}

/// Related search: given seed document paths, find documents most related to them.
/// Uses multi-source BFS approximation of Personalized PageRank.
///
/// Strategy:
/// 1. For each seed, BFS to depth 3
/// 2. Accumulate score for each neighbor: `1.0 / (hop_distance * seeds.len())`
/// 3. Remove seeds from results
/// 4. Sort by accumulated score, take top `limit`
pub fn search_related(
    graph: &impl GraphStore,
    seeds: &[String],
    limit: usize,
    _damping: f64,
    _iterations: usize,
    modality: Modality,
    code_paths: &HashSet<String>,
) -> Result<Vec<SearchResult>> {
    if seeds.is_empty() {
        return Ok(Vec::new());
    }

    let num_seeds = seeds.len() as f64;
    let mut score_map: HashMap<String, (f64, usize)> = HashMap::new(); // path -> (accumulated_score, min_hops)

    for seed in seeds {
        let neighbors = graph.traverse_bfs(seed, 3, None, None);
        for (path, hops) in neighbors {
            if hops == 0 {
                continue;
            }
            let score = 1.0 / (hops as f64 * num_seeds);
            let entry = score_map.entry(path).or_insert((0.0, hops));
            entry.0 += score;
            if hops < entry.1 {
                entry.1 = hops;
            }
        }
    }

    // Remove seeds from results.
    for seed in seeds {
        let _ = score_map.remove(seed);
    }

    // Build results, sort, truncate.
    let mut results: Vec<SearchResult> = score_map
        .into_iter()
        .map(|(path, (score, min_hops))| {
            SearchResult::new(path, score).with_score_components(ScoreBreakdown {
                bm25: 0.0,
                vector: 0.0,
                graph_boost: score,
                graph_hops: Some(min_hops),
            })
        })
        .collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.retain(|r| path_matches_modality(&r.path, modality, code_paths));
    results.truncate(limit);
    enrich_results_with_lineage(&mut results, graph);

    Ok(results)
}
