//! Information Retrieval (IR) evaluation metrics.

use std::collections::HashMap;

use groundcontrol_common::types::SearchResult;
use serde::{Deserialize, Serialize};

use crate::dataset::schema::RelevanceJudgment;

/// Evaluation metrics for a single query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryEvaluationMetrics {
    /// Recall at cutoff K.
    pub recall_at_k: f64,
    /// Precision at cutoff K.
    pub precision_at_k: f64,
    /// Mean Reciprocal Rank at cutoff K.
    pub mrr_at_k: f64,
    /// Normalized Discounted Cumulative Gain at cutoff K.
    pub ndcg_at_k: f64,
    /// Score separation: ratio of top-1 score to top-K score (or 0.0 if not applicable).
    pub score_separation: f64,
    /// Number of ground-truth hits present in the top-K.
    pub hits_at_k: usize,
    /// Total ground-truth relevant documents for this query.
    pub total_relevant: usize,
    /// Optional Turn-1 cluster recall at cutoff K (true if target or 1-hop neighbor is in top-K).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_recall_at_k: Option<f64>,
    /// Optional minimum graph hop distance to target among top-K candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_hop_distance: Option<usize>,
}

/// Evaluator computing standard Information Retrieval metrics.
pub struct IrEvaluator;

impl IrEvaluator {
    /// Evaluate a ranked result list against expected judgments at cutoff `k`.
    pub fn evaluate(
        results: &[SearchResult],
        judgments: &[RelevanceJudgment],
        k: usize,
    ) -> QueryEvaluationMetrics {
        if judgments.is_empty() {
            return QueryEvaluationMetrics {
                recall_at_k: 0.0,
                precision_at_k: 0.0,
                mrr_at_k: 0.0,
                ndcg_at_k: 0.0,
                score_separation: 0.0,
                hits_at_k: 0,
                total_relevant: 0,
                cluster_recall_at_k: None,
                min_hop_distance: None,
            };
        }

        let norm_judgments: Vec<(String, u8)> =
            judgments.iter().map(|j| (normalize_path(&j.path), j.grade)).collect();

        let top_k: Vec<&SearchResult> = results.iter().take(k).collect();
        let mut hits = 0;
        let mut first_hit_rank: Option<usize> = None;
        let mut dcg = 0.0;
        let mut matched_judgments = std::collections::HashSet::new();

        for (idx, res) in top_k.iter().enumerate() {
            let rank = idx + 1;
            let norm_res = normalize_path(&res.path);

            for (j_idx, (expected_path, grade)) in norm_judgments.iter().enumerate() {
                if path_matches(&norm_res, expected_path) && *grade > 0 {
                    if matched_judgments.insert(j_idx) {
                        hits += 1;
                        if first_hit_rank.is_none() {
                            first_hit_rank = Some(rank);
                        }
                        // Graded DCG: (2^rel - 1) / log2(rank + 1)
                        let gain = 2.0f64.powi(*grade as i32) - 1.0;
                        dcg += gain / (rank as f64 + 1.0).log2();
                    }
                    break;
                }
            }
        }

        let recall_at_k = hits as f64 / judgments.len() as f64;
        let precision_at_k = if k > 0 { hits as f64 / k as f64 } else { 0.0 };
        let mrr_at_k = match first_hit_rank {
            Some(r) => 1.0 / r as f64,
            None => 0.0,
        };

        // Compute IDCG (Ideal DCG)
        let mut ideal_grades: Vec<u8> = judgments.iter().map(|j| j.grade).collect();
        ideal_grades.sort_by(|a, b| b.cmp(a));
        let mut idcg = 0.0;
        for (idx, &grade) in ideal_grades.iter().take(k).enumerate() {
            let rank = idx + 1;
            let gain = 2.0f64.powi(grade as i32) - 1.0;
            idcg += gain / (rank as f64 + 1.0).log2();
        }

        let ndcg_at_k = if idcg > 0.0 { dcg / idcg } else { 0.0 };

        let score_separation = if top_k.len() >= 2 && top_k[0].score > 0.0 {
            let last_score = top_k.last().map(|r| r.score).unwrap_or(1.0);
            if last_score > 0.0 {
                top_k[0].score / last_score
            } else {
                top_k[0].score
            }
        } else {
            1.0
        };

        QueryEvaluationMetrics {
            recall_at_k,
            precision_at_k,
            mrr_at_k,
            ndcg_at_k,
            score_separation,
            hits_at_k: hits,
            total_relevant: judgments.len(),
            cluster_recall_at_k: None,
            min_hop_distance: None,
        }
    }

    /// Evaluate retrieval results including Turn-1 structural orientation metrics.
    ///
    /// - `anchor_cluster`: Set of 1-hop neighbor paths or structural anchors surrounding the ground-truth targets.
    /// - `hop_distances`: Map from candidate path to graph shortest-path distance to the nearest target.
    pub fn evaluate_with_orientation(
        results: &[SearchResult],
        judgments: &[RelevanceJudgment],
        k: usize,
        anchor_cluster: Option<&std::collections::HashSet<String>>,
        hop_distances: Option<&HashMap<String, usize>>,
    ) -> QueryEvaluationMetrics {
        let mut metrics = Self::evaluate(results, judgments, k);

        let top_k: Vec<&SearchResult> = results.iter().take(k).collect();

        // 1. Cluster Recall@K: True if target OR 1-hop anchor is in top-K
        if let Some(anchors) = anchor_cluster {
            let target_paths: std::collections::HashSet<&str> =
                judgments.iter().map(|j| j.path.as_str()).collect();

            let found = top_k.iter().any(|r| {
                target_paths.contains(r.path.as_str()) || anchors.contains(r.path.as_str())
            });
            metrics.cluster_recall_at_k = Some(if found { 1.0 } else { 0.0 });
        }

        // 2. Minimum hop distance to target among top-K
        if let Some(distances) = hop_distances {
            let min_hop =
                top_k.iter().filter_map(|r| distances.get(r.path.as_str()).copied()).min();
            metrics.min_hop_distance = min_hop;
        }

        metrics
    }
}

/// Normalize path string by converting backslashes to forward slashes, stripping chunk suffixes (`:chunk:N`),
/// line anchors (`#L...`), and trimming `./` / `/`.
pub fn normalize_path(p: &str) -> String {
    let mut s = p.replace('\\', "/");
    if let Some(hash_pos) = s.find('#') {
        s.truncate(hash_pos);
    }
    if let Some(chunk_pos) = s.find(":chunk:") {
        s.truncate(chunk_pos);
    }
    s.trim_start_matches("./").trim_start_matches('/').to_string()
}

/// Check if a candidate path matches an expected path (exact match or directory-boundary suffix match).
pub fn path_matches(candidate: &str, expected: &str) -> bool {
    if candidate == expected {
        return true;
    }
    if candidate.ends_with(expected) {
        let prefix_len = candidate.len() - expected.len();
        if prefix_len == 0 || candidate.as_bytes()[prefix_len - 1] == b'/' {
            return true;
        }
    }
    if expected.ends_with(candidate) {
        let prefix_len = expected.len() - candidate.len();
        if prefix_len == 0 || expected.as_bytes()[prefix_len - 1] == b'/' {
            return true;
        }
    }
    false
}
