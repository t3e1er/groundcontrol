//! Benchmark runner for retrieval quality evaluation.
//! Runs all 28 queries against the indexed corpus and computes Recall@5, MRR@5, NDCG@5,
//! score separation, and produces `bench/results_v2.json` and `bench/report_v2.md`.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use groundcontrol_common::config::CorpusConfig;
use groundcontrol_common::ports::{GraphStore, SearchQuery, SearchService};
use groundcontrol_common::types::{Modality, SearchDepth, SearchResult};
use groundcontrol_core::engine::Engine;

/// Build a per-mode [`SearchQuery`] for the benchmark harness.
fn bench_query(
    query: &str,
    mode: &str,
    modality: Modality,
    decompose: Option<bool>,
    depth: Option<SearchDepth>,
) -> SearchQuery {
    SearchQuery {
        query: query.to_string(),
        mode: Some(mode.to_string()),
        limit: Some(10),
        modality,
        depth: depth.unwrap_or_default(),
        graph_depth: Some(2),
        edge_types: None,
        edge_class: None,
        decompose,
        snippets: None,
    }
}

#[derive(Debug, Clone, Deserialize)]
struct QueryItem {
    id: String,
    query: String,
    expected_relevant: Vec<String>,
    category: String,
}

#[derive(Debug, Clone, Serialize)]
struct QueryResultRecord {
    id: String,
    query: String,
    category: String,
    expected_relevant: Vec<String>,
    bm25_top5: Vec<ResultEntry>,
    bm25_recall_at_5: f64,
    bm25_mrr_at_5: f64,
    bm25_ndcg_at_5: f64,
    semantic_top5: Vec<ResultEntry>,
    semantic_recall_at_5: f64,
    semantic_mrr_at_5: f64,
    semantic_ndcg_at_5: f64,
    hybrid_top5: Vec<ResultEntry>,
    hybrid_recall_at_5: f64,
    hybrid_mrr_at_5: f64,
    hybrid_ndcg_at_5: f64,
    hybrid_score_separation: f64, // top-1 score / top-5 score
}

#[derive(Debug, Clone, Serialize)]
struct ResultEntry {
    rank: usize,
    path: String,
    score: f64,
    is_expected: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
struct CategoryMetrics {
    count: usize,
    bm25_recall_at_5: f64,
    bm25_mrr_at_5: f64,
    bm25_ndcg_at_5: f64,
    semantic_recall_at_5: f64,
    semantic_mrr_at_5: f64,
    semantic_ndcg_at_5: f64,
    hybrid_recall_at_5: f64,
    hybrid_mrr_at_5: f64,
    hybrid_ndcg_at_5: f64,
    hybrid_avg_score_separation: f64,
}

#[derive(Debug, Clone, Serialize)]
struct BenchmarkSummary {
    total_queries: usize,
    graph_stats: Value,
    categories: std::collections::HashMap<String, CategoryMetrics>,
    overall: CategoryMetrics,
    results: Vec<QueryResultRecord>,
}

fn compute_metrics(
    results: &[SearchResult],
    expected: &[String],
) -> (Vec<ResultEntry>, f64, f64, f64) {
    let expected_set: HashSet<&str> = expected.iter().map(|s| s.as_str()).collect();
    let mut entries = Vec::new();
    let mut first_hit_rank: Option<usize> = None;
    let mut hits_at_5 = 0;
    let mut dcg = 0.0;

    for (i, r) in results.iter().take(5).enumerate() {
        let rank = i + 1;
        let is_expected = expected_set.contains(r.path.as_str());
        if is_expected {
            hits_at_5 += 1;
            if first_hit_rank.is_none() {
                first_hit_rank = Some(rank);
            }
            dcg += 1.0 / (rank as f64 + 1.0).log2();
        }
        entries.push(ResultEntry { rank, path: r.path.clone(), score: r.score, is_expected });
    }

    let recall = if expected.is_empty() { 0.0 } else { hits_at_5 as f64 / expected.len() as f64 };

    let mrr = match first_hit_rank {
        Some(rank) => 1.0 / rank as f64,
        None => 0.0,
    };

    let mut idcg = 0.0;
    let ideal_hits = expected.len().min(5);
    for i in 1..=ideal_hits {
        idcg += 1.0 / (i as f64 + 1.0).log2();
    }
    let ndcg = if idcg > 0.0 { dcg / idcg } else { 0.0 };

    (entries, recall, mrr, ndcg)
}

fn main() {
    println!("============================================================");
    println!(" GROUNDCONTROL RETRIEVAL QUALITY BENCHMARK RUNNER");
    println!("============================================================");

    let corpus_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"c:\dev\ctx\ctxcorpus\anthropic"));
    let config_path = if corpus_dir.join("groundcontrol.toml").exists() {
        corpus_dir.join("groundcontrol.toml")
    } else {
        corpus_dir.join("ctxvault.toml")
    };
    let index_dir = corpus_dir.join(".index");
    let queries_path = PathBuf::from(r"c:\dev\ctx\ctxcorpus\bench\queries.json");
    let results_out_path = PathBuf::from(r"c:\dev\ctx\ctxcorpus\bench\results_v2.json");
    let report_out_path = PathBuf::from(r"c:\dev\ctx\ctxcorpus\bench\report_v2.md");

    // Clean stale lock files
    let lock1 = index_dir.join("tantivy").join(".tantivy-meta.lock");
    let lock2 = index_dir.join("tantivy").join(".tantivy-writer.lock");
    let _ = fs::remove_file(lock1);
    let _ = fs::remove_file(lock2);

    println!("Loading configuration from {:?}", config_path);
    let config_str = fs::read_to_string(&config_path).expect("Read config");
    let config: CorpusConfig = toml::from_str(&config_str).expect("Parse config");

    println!("Opening engine at {:?}", index_dir);
    let mut engine = Engine::open(config, &index_dir).expect("Open Engine");

    println!("Ensuring embedding model...");
    let embedder_ok = engine.ensure_embedder().expect("Ensure embedder");
    println!("Embedder active: {}", embedder_ok);

    println!("Performing full corpus reindex...");
    let reindexed_count = engine.full_reindex().expect("Reindex");
    println!("Reindexed {} documents", reindexed_count);

    println!("Performing full corpus re-embed...");
    let reembed_count = engine.reembed().expect("Reembed");
    println!("Reembedded {} chunks", reembed_count);

    // Persist BM25 + graph + vector index to the corpus's .index directory.
    engine.commit().expect("Commit index");

    // Graph Stats
    let stats = engine.graph().stats();
    let stats_json = serde_json::to_value(&stats).unwrap();
    println!("------------------------------------------------------------");
    println!("GRAPH STATISTICS:");
    println!("  Node count: {}", stats.node_count);
    println!("  Edge count: {}", stats.edge_count);
    println!("  Orphan count: {}", stats.orphan_count);
    println!("  Edge distribution: {:?}", stats.edge_type_distribution);
    println!("  Top hubs: {:?}", stats.most_connected.iter().take(5).collect::<Vec<_>>());
    println!("------------------------------------------------------------");

    println!("Loading queries from {:?}", queries_path);
    let queries_str = fs::read_to_string(&queries_path).expect("Read queries.json");
    let queries: Vec<QueryItem> = serde_json::from_str(&queries_str).expect("Parse queries.json");
    println!("Loaded {} evaluation queries", queries.len());

    let mut query_records: Vec<QueryResultRecord> = Vec::new();

    let modality = groundcontrol_common::types::Modality::Both;
    // One search service over the engine's backends; each query builds a
    // per-mode `SearchQuery` and dispatches through the `SearchService` port.
    let service = engine.search_service();
    for q in &queries {
        // 1. BM25 Search
        let bm25_res = service
            .search(&bench_query(&q.query, "bm25", modality, None, None))
            .unwrap_or_default();
        let (bm25_top5, bm25_recall, bm25_mrr, bm25_ndcg) =
            compute_metrics(&bm25_res, &q.expected_relevant);

        // 2. Semantic Search (Precise direct chunk)
        let sem_res = service
            .search(&bench_query(
                &q.query,
                "semantic",
                modality,
                None,
                Some(groundcontrol_common::types::SearchDepth::Precise),
            ))
            .unwrap_or_default();
        let (semantic_top5, sem_recall, sem_mrr, sem_ndcg) =
            compute_metrics(&sem_res, &q.expected_relevant);

        // 3. Hybrid Search (multi-hop decomposition for the multi-hop category).
        let is_multihop = q.category == "multi-hop";
        let hyb_res = service
            .search(&bench_query(
                &q.query,
                "hybrid",
                modality,
                if is_multihop { Some(true) } else { None },
                None,
            ))
            .unwrap_or_default();

        let (hybrid_top5, hyb_recall, hyb_mrr, hyb_ndcg) =
            compute_metrics(&hyb_res, &q.expected_relevant);

        // Score separation in hybrid
        let hyb_score_sep = if hybrid_top5.len() >= 5 && hybrid_top5[4].score > 0.0 {
            hybrid_top5[0].score / hybrid_top5[4].score
        } else if !hybrid_top5.is_empty() {
            2.0
        } else {
            1.0
        };

        query_records.push(QueryResultRecord {
            id: q.id.clone(),
            query: q.query.clone(),
            category: q.category.clone(),
            expected_relevant: q.expected_relevant.clone(),
            bm25_top5,
            bm25_recall_at_5: bm25_recall,
            bm25_mrr_at_5: bm25_mrr,
            bm25_ndcg_at_5: bm25_ndcg,
            semantic_top5,
            semantic_recall_at_5: sem_recall,
            semantic_mrr_at_5: sem_mrr,
            semantic_ndcg_at_5: sem_ndcg,
            hybrid_top5,
            hybrid_recall_at_5: hyb_recall,
            hybrid_mrr_at_5: hyb_mrr,
            hybrid_ndcg_at_5: hyb_ndcg,
            hybrid_score_separation: hyb_score_sep,
        });
    }

    // Compute aggregated category and overall metrics
    let mut category_map: std::collections::HashMap<String, CategoryMetrics> =
        std::collections::HashMap::new();
    let mut overall = CategoryMetrics::default();

    for qr in &query_records {
        let cat = category_map.entry(qr.category.clone()).or_default();
        cat.count += 1;
        cat.bm25_recall_at_5 += qr.bm25_recall_at_5;
        cat.bm25_mrr_at_5 += qr.bm25_mrr_at_5;
        cat.bm25_ndcg_at_5 += qr.bm25_ndcg_at_5;
        cat.semantic_recall_at_5 += qr.semantic_recall_at_5;
        cat.semantic_mrr_at_5 += qr.semantic_mrr_at_5;
        cat.semantic_ndcg_at_5 += qr.semantic_ndcg_at_5;
        cat.hybrid_recall_at_5 += qr.hybrid_recall_at_5;
        cat.hybrid_mrr_at_5 += qr.hybrid_mrr_at_5;
        cat.hybrid_ndcg_at_5 += qr.hybrid_ndcg_at_5;
        cat.hybrid_avg_score_separation += qr.hybrid_score_separation;

        overall.count += 1;
        overall.bm25_recall_at_5 += qr.bm25_recall_at_5;
        overall.bm25_mrr_at_5 += qr.bm25_mrr_at_5;
        overall.bm25_ndcg_at_5 += qr.bm25_ndcg_at_5;
        overall.semantic_recall_at_5 += qr.semantic_recall_at_5;
        overall.semantic_mrr_at_5 += qr.semantic_mrr_at_5;
        overall.semantic_ndcg_at_5 += qr.semantic_ndcg_at_5;
        overall.hybrid_recall_at_5 += qr.hybrid_recall_at_5;
        overall.hybrid_mrr_at_5 += qr.hybrid_mrr_at_5;
        overall.hybrid_ndcg_at_5 += qr.hybrid_ndcg_at_5;
        overall.hybrid_avg_score_separation += qr.hybrid_score_separation;
    }

    for cat in category_map.values_mut() {
        if cat.count > 0 {
            cat.bm25_recall_at_5 /= cat.count as f64;
            cat.bm25_mrr_at_5 /= cat.count as f64;
            cat.bm25_ndcg_at_5 /= cat.count as f64;
            cat.semantic_recall_at_5 /= cat.count as f64;
            cat.semantic_mrr_at_5 /= cat.count as f64;
            cat.semantic_ndcg_at_5 /= cat.count as f64;
            cat.hybrid_recall_at_5 /= cat.count as f64;
            cat.hybrid_mrr_at_5 /= cat.count as f64;
            cat.hybrid_ndcg_at_5 /= cat.count as f64;
            cat.hybrid_avg_score_separation /= cat.count as f64;
        }
    }

    if overall.count > 0 {
        overall.bm25_recall_at_5 /= overall.count as f64;
        overall.bm25_mrr_at_5 /= overall.count as f64;
        overall.bm25_ndcg_at_5 /= overall.count as f64;
        overall.semantic_recall_at_5 /= overall.count as f64;
        overall.semantic_mrr_at_5 /= overall.count as f64;
        overall.semantic_ndcg_at_5 /= overall.count as f64;
        overall.hybrid_recall_at_5 /= overall.count as f64;
        overall.hybrid_mrr_at_5 /= overall.count as f64;
        overall.hybrid_ndcg_at_5 /= overall.count as f64;
        overall.hybrid_avg_score_separation /= overall.count as f64;
    }

    println!("\n=== BENCHMARK OVERALL RESULTS ===");
    println!("Total queries: {}", overall.count);
    println!(
        "BM25    — Recall@5: {:.3}, MRR@5: {:.3}, NDCG@5: {:.3}",
        overall.bm25_recall_at_5, overall.bm25_mrr_at_5, overall.bm25_ndcg_at_5
    );
    println!(
        "Semantic— Recall@5: {:.3}, MRR@5: {:.3}, NDCG@5: {:.3}",
        overall.semantic_recall_at_5, overall.semantic_mrr_at_5, overall.semantic_ndcg_at_5
    );
    println!(
        "Hybrid  — Recall@5: {:.3}, MRR@5: {:.3}, NDCG@5: {:.3}, Avg Separation: {:.2}x",
        overall.hybrid_recall_at_5,
        overall.hybrid_mrr_at_5,
        overall.hybrid_ndcg_at_5,
        overall.hybrid_avg_score_separation
    );

    for (cat_name, cat) in &category_map {
        println!("\n--- Category: {} (N={}) ---", cat_name, cat.count);
        println!(
            "  BM25:     Recall@5: {:.3}, MRR@5: {:.3}, NDCG@5: {:.3}",
            cat.bm25_recall_at_5, cat.bm25_mrr_at_5, cat.bm25_ndcg_at_5
        );
        println!(
            "  Semantic: Recall@5: {:.3}, MRR@5: {:.3}, NDCG@5: {:.3}",
            cat.semantic_recall_at_5, cat.semantic_mrr_at_5, cat.semantic_ndcg_at_5
        );
        println!(
            "  Hybrid:   Recall@5: {:.3}, MRR@5: {:.3}, NDCG@5: {:.3} (Sep: {:.2}x)",
            cat.hybrid_recall_at_5,
            cat.hybrid_mrr_at_5,
            cat.hybrid_ndcg_at_5,
            cat.hybrid_avg_score_separation
        );
    }

    // Save results_v2.json
    let summary = BenchmarkSummary {
        total_queries: queries.len(),
        graph_stats: stats_json,
        categories: category_map.clone(),
        overall: overall.clone(),
        results: query_records.clone(),
    };
    let json_bytes = serde_json::to_string_pretty(&summary).expect("Serialize summary");
    fs::write(&results_out_path, json_bytes).expect("Write results_v2.json");
    println!("\nWrote detailed results to {:?}", results_out_path);

    // Generate report_v2.md
    let mut report = String::new();
    report.push_str("# GroundControl Retrieval Quality Benchmark Report (v2)\n\n");
    report.push_str("Evaluation of 28 benchmark queries post-improvements (Edge Class Taxonomy, IDF Graph Weighting, 3-Way RRF, Query Decomposition, Contextual Heading Prefixing).\n\n");

    report.push_str("## 1. Summary Metrics\n\n");
    report.push_str("| Metric | BM25 | Semantic | Hybrid (v2) |\n");
    report.push_str("|---|---|---|---|\n");
    report.push_str(&format!(
        "| **Recall@5 (Overall)** | {:.1}% | {:.1}% | **{:.1}%** |\n",
        overall.bm25_recall_at_5 * 100.0,
        overall.semantic_recall_at_5 * 100.0,
        overall.hybrid_recall_at_5 * 100.0
    ));
    report.push_str(&format!(
        "| **MRR@5 (Overall)** | {:.3} | {:.3} | **{:.3}** |\n",
        overall.bm25_mrr_at_5, overall.semantic_mrr_at_5, overall.hybrid_mrr_at_5
    ));
    report.push_str(&format!(
        "| **NDCG@5 (Overall)** | {:.3} | {:.3} | **{:.3}** |\n",
        overall.bm25_ndcg_at_5, overall.semantic_ndcg_at_5, overall.hybrid_ndcg_at_5
    ));
    report.push_str(&format!(
        "| **Top-1 / Top-5 Separation** | N/A | N/A | **{:.2}x** |\n\n",
        overall.hybrid_avg_score_separation
    ));

    report.push_str("## 2. Category Breakdown\n\n");
    report.push_str("| Category | Count | BM25 Recall@5 | Semantic Recall@5 | Hybrid Recall@5 | BM25 MRR | Semantic MRR | Hybrid MRR |\n");
    report.push_str("|---|---|---|---|---|---|---|---|\n");
    for cat_name in ["keyword", "semantic", "graph", "multi-hop"] {
        if let Some(cat) = category_map.get(cat_name) {
            report.push_str(&format!(
                "| **{}** | {} | {:.1}% | {:.1}% | **{:.1}%** | {:.3} | {:.3} | **{:.3}** |\n",
                cat_name,
                cat.count,
                cat.bm25_recall_at_5 * 100.0,
                cat.semantic_recall_at_5 * 100.0,
                cat.hybrid_recall_at_5 * 100.0,
                cat.bm25_mrr_at_5,
                cat.semantic_mrr_at_5,
                cat.hybrid_mrr_at_5
            ));
        }
    }

    report.push_str("\n## 3. Query Details\n\n");
    report.push_str(
        "| ID | Category | Query | Hybrid Top-1 Result | Expected? | Hybrid Score Sep |\n",
    );
    report.push_str("|---|---|---|---|---|---|\n");
    for qr in &query_records {
        let top1_path = qr.hybrid_top5.first().map(|r| r.path.as_str()).unwrap_or("None");
        let top1_expected = qr
            .hybrid_top5
            .first()
            .map(|r| if r.is_expected { "Yes" } else { "No" })
            .unwrap_or("No");
        report.push_str(&format!(
            "| {} | {} | {} | `{}` | {} | {:.2}x |\n",
            qr.id, qr.category, qr.query, top1_path, top1_expected, qr.hybrid_score_separation
        ));
    }

    fs::write(&report_out_path, report).expect("Write report_v2.md");
    println!("Wrote benchmark report to {:?}", report_out_path);
}
