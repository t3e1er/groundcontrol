//! Fusion strategies and search result enrichment helpers.

use std::collections::{HashMap, HashSet};

use groundcontrol_common::ports::GraphStore;
use groundcontrol_common::types::{Modality, ScoreBreakdown, SearchResult};

/// Classify a result path as passing a [`Modality`] filter using the set of
/// known code node keys.
///
/// A path is treated as code if it appears in `code_paths` (scope_paths, code
/// file paths, and `<corpus>::scope_path` keys derived from the code-symbol
/// catalog); otherwise it is documentation. [`Modality::Both`] accepts every
/// path.
pub fn path_matches_modality(path: &str, modality: Modality, code_paths: &HashSet<String>) -> bool {
    let normalized = path.replace('\\', "/");
    let is_code = code_paths.contains(path)
        || code_paths.contains(&normalized)
        || crate::parser::code::is_code_file(std::path::Path::new(path));
    match modality {
        Modality::Both => true,
        Modality::Code => is_code,
        Modality::Docs => !is_code,
    }
}

/// Enrich search results with structural lineage metadata from the knowledge graph.
pub fn enrich_results_with_lineage(results: &mut [SearchResult], graph: &impl GraphStore) {
    for result in results.iter_mut() {
        if result.lineage.is_none() {
            result.lineage = graph.extract_lineage_for_node(&result.path);
        }
    }
}

/// Reciprocal Rank Fusion: merges multiple ranked lists into one using default k = 60.0.
///
/// RRF score = sum over all lists of: 1 / (60 + rank_in_list).
pub fn rrf_fuse(result_lists: &[&[SearchResult]], limit: usize) -> Vec<SearchResult> {
    rrf_fuse_with_k(result_lists, limit, 60.0)
}

/// Reciprocal Rank Fusion: merges multiple ranked lists into one with a configurable `k` constant.
///
/// RRF score = sum over all lists of: 1 / (k + rank_in_list).
pub fn rrf_fuse_with_k(
    result_lists: &[&[SearchResult]],
    limit: usize,
    k: f64,
) -> Vec<SearchResult> {
    // Accumulate RRF scores per document path.
    let mut rrf_scores: HashMap<String, (f64, Option<String>, Option<usize>, ScoreBreakdown)> =
        HashMap::new();

    for list in result_lists {
        for (rank, result) in list.iter().enumerate() {
            let rrf_contribution = 1.0 / (k + rank as f64 + 1.0);

            let entry = rrf_scores.entry(result.path.clone()).or_insert_with(|| {
                (
                    0.0,
                    result.snippet.clone(),
                    result.chunk_index,
                    ScoreBreakdown { bm25: 0.0, vector: 0.0, graph_boost: 0.0, graph_hops: None },
                )
            });
            entry.0 += rrf_contribution;

            // Accumulate the vector component from the original score.
            if let Some(ref components) = result.score_components {
                if components.vector > entry.3.vector {
                    entry.3.vector = components.vector;
                }
            }
        }
    }

    // Build results sorted by RRF score.
    let mut results: Vec<SearchResult> = rrf_scores
        .into_iter()
        .map(|(path, (score, snippet, chunk_index, components))| {
            SearchResult::new(path, score)
                .with_snippet(snippet)
                .with_chunk_index(chunk_index)
                .with_score_components(components)
        })
        .collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    results
}

/// Cross-corpus Reciprocal Rank Fusion: merges per-corpus ranked lists into one
/// ranking, preserving each result's source-corpus tag and rich fields.
///
/// Each input list is `(corpus_name, results)`, already ranked descending. RRF is
/// applied over each list (K = 60, contribution `1 / (K + rank + 1)`). Results are
/// keyed by `(corpus, path)` so the same path appearing in two different corpora is
/// preserved as two distinct hits, each tagged with its origin corpus. The source
/// result's snippet, chunk index, entity kind, language, lineage, and score
/// components are carried through. The fused list is sorted by RRF score descending
/// and truncated to `limit`.
pub fn rrf_fuse_cross_corpus(
    tagged_lists: &[(String, Vec<SearchResult>)],
    limit: usize,
) -> Vec<SearchResult> {
    const K: f64 = 60.0;

    // Accumulate fused RRF score per (corpus, path); keep the first-seen rich result.
    let mut fused: HashMap<(String, String), (f64, SearchResult)> = HashMap::new();

    for (corpus_name, list) in tagged_lists {
        for (rank, result) in list.iter().enumerate() {
            let contribution = 1.0 / (K + rank as f64 + 1.0);
            let key = (corpus_name.clone(), result.path.clone());

            let entry = fused.entry(key).or_insert_with(|| {
                let tagged = result.clone().with_corpus(Some(corpus_name.clone()));
                (0.0, tagged)
            });
            entry.0 += contribution;
        }
    }

    // Apply fused score and collect.
    let mut results: Vec<SearchResult> = fused
        .into_values()
        .map(|(score, mut result)| {
            result.score = score;
            result
        })
        .collect();

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.corpus.cmp(&b.corpus))
            .then_with(|| a.path.cmp(&b.path))
    });
    results.truncate(limit);
    results
}
