//! Parameter sweeping and benchmark suite execution.

use std::collections::HashMap;

use groundcontrol_core::engine::Engine;
use serde::{Deserialize, Serialize};

use crate::dataset::schema::BenchmarkDataset;
use crate::metrics::ir::{IrEvaluator, QueryEvaluationMetrics};
use crate::metrics::latency::{LatencyStats, LatencyTracker};
use crate::profile::index_profiler::IndexingProfileReport;
use crate::runners::{QueryRunner, QueryRunnerOptions, RetrievalMode};
use rayon::prelude::*;

/// Aggregated performance and retrieval metrics for an individual mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeEvaluationSummary {
    /// Evaluated retrieval mode.
    pub mode: RetrievalMode,
    /// Cutoff K used for Recall@K, Precision@K, MRR@K, NDCG@K.
    pub k: usize,
    /// Mean Recall@K across all queries.
    pub mean_recall: f64,
    /// Mean Precision@K across all queries.
    pub mean_precision: f64,
    /// Mean Reciprocal Rank at K (MRR@K).
    pub mean_mrr: f64,
    /// Mean NDCG@K (with graded relevance).
    pub mean_ndcg: f64,
    /// Mean score separation ratio.
    pub mean_score_separation: f64,
    /// Latency statistics across all query evaluations.
    pub latency: LatencyStats,
    /// Category breakdown metrics (e.g. exact vs concept).
    pub category_metrics: HashMap<String, CategoryAblationMetrics>,
}

/// Category-specific ablation metrics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CategoryAblationMetrics {
    /// Number of queries in this category.
    pub count: usize,
    /// Mean Recall@K.
    pub mean_recall: f64,
    /// Mean MRR@K.
    pub mean_mrr: f64,
    /// Mean NDCG@K.
    pub mean_ndcg: f64,
}

/// Comprehensive benchmark suite report combining indexing and retrieval ablation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSuiteReport {
    /// Timestamp of the benchmark run.
    pub timestamp_unix: u64,
    /// Total number of evaluated queries.
    pub query_count: usize,
    /// Cutoff rank K used for evaluation.
    pub k: usize,
    /// Per-mode evaluation summaries.
    pub modes: Vec<ModeEvaluationSummary>,
    /// Optional indexing profiling report if indexing was run.
    pub indexing: Option<IndexingProfileReport>,
    /// Optional benchmark name (e.g. "swe_bench", "codesearchnet", "repobench").
    #[serde(default)]
    pub benchmark: Option<String>,
    /// Optional target repository name (e.g. "pallets__flask").
    #[serde(default)]
    pub repository: Option<String>,
}

/// Benchmark suite evaluator.
pub struct BenchmarkSuite;

impl BenchmarkSuite {
    /// Evaluate a set of retrieval modes against a dataset.
    pub fn evaluate_modes(
        engine: &Engine,
        dataset: &BenchmarkDataset,
        modes: &[RetrievalMode],
        k: usize,
        modality: groundcontrol_common::types::Modality,
    ) -> groundcontrol_common::Result<Vec<ModeEvaluationSummary>> {
        let mut summaries = Vec::new();

        let runner_opts = QueryRunnerOptions { limit: k.max(10), modality, decompose: false };

        for &mode in modes {
            let (latencies, query_metrics) = dataset
                .queries
                .par_iter()
                .fold(
                    || {
                        (
                            LatencyTracker::new(),
                            Vec::<(Option<String>, QueryEvaluationMetrics)>::new(),
                        )
                    },
                    |(mut lat_acc, mut met_acc), query| {
                        match QueryRunner::execute(engine, query, mode, &runner_opts) {
                            Ok((results, ms)) => {
                                lat_acc.record(ms);
                                let m = IrEvaluator::evaluate(&results, &query.expected, k);
                                met_acc.push((query.category.clone(), m));
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Query '{}' failed for mode {:?}: {}",
                                    query.query,
                                    mode,
                                    e
                                );
                            }
                        }
                        (lat_acc, met_acc)
                    },
                )
                .reduce(
                    || (LatencyTracker::new(), Vec::new()),
                    |(mut lat1, mut met1), (lat2, met2)| {
                        lat1.merge(lat2);
                        met1.extend(met2);
                        (lat1, met1)
                    },
                );

            let summary = Self::aggregate_mode_metrics(mode, k, query_metrics, latencies.compute());
            summaries.push(summary);
        }

        Ok(summaries)
    }

    fn aggregate_mode_metrics(
        mode: RetrievalMode,
        k: usize,
        metrics: Vec<(Option<String>, QueryEvaluationMetrics)>,
        latency: LatencyStats,
    ) -> ModeEvaluationSummary {
        let n = metrics.len();
        if n == 0 {
            return ModeEvaluationSummary {
                mode,
                k,
                mean_recall: 0.0,
                mean_precision: 0.0,
                mean_mrr: 0.0,
                mean_ndcg: 0.0,
                mean_score_separation: 0.0,
                latency,
                category_metrics: HashMap::new(),
            };
        }

        let mut sum_recall = 0.0;
        let mut sum_precision = 0.0;
        let mut sum_mrr = 0.0;
        let mut sum_ndcg = 0.0;
        let mut sum_sep = 0.0;

        let mut cat_map: HashMap<String, (usize, f64, f64, f64)> = HashMap::new();

        for (cat_opt, m) in &metrics {
            sum_recall += m.recall_at_k;
            sum_precision += m.precision_at_k;
            sum_mrr += m.mrr_at_k;
            sum_ndcg += m.ndcg_at_k;
            sum_sep += m.score_separation;

            let cat_key = cat_opt.as_deref().unwrap_or("general").to_string();
            let entry = cat_map.entry(cat_key).or_insert((0, 0.0, 0.0, 0.0));
            entry.0 += 1;
            entry.1 += m.recall_at_k;
            entry.2 += m.mrr_at_k;
            entry.3 += m.ndcg_at_k;
        }

        let mut category_metrics = HashMap::new();
        for (cat, (cnt, rec, mrr, ndcg)) in cat_map {
            category_metrics.insert(
                cat,
                CategoryAblationMetrics {
                    count: cnt,
                    mean_recall: if cnt > 0 { rec / cnt as f64 } else { 0.0 },
                    mean_mrr: if cnt > 0 { mrr / cnt as f64 } else { 0.0 },
                    mean_ndcg: if cnt > 0 { ndcg / cnt as f64 } else { 0.0 },
                },
            );
        }

        ModeEvaluationSummary {
            mode,
            k,
            mean_recall: sum_recall / n as f64,
            mean_precision: sum_precision / n as f64,
            mean_mrr: sum_mrr / n as f64,
            mean_ndcg: sum_ndcg / n as f64,
            mean_score_separation: sum_sep / n as f64,
            latency,
            category_metrics,
        }
    }
}
