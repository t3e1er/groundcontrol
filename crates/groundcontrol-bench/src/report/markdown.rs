//! Markdown report generator for benchmark results.

use crate::sweep::BenchmarkSuiteReport;

/// Formatter generating GitHub-flavored markdown benchmark reports.
pub struct MarkdownReporter;

impl MarkdownReporter {
    /// Format a benchmark suite report into a Markdown document.
    pub fn render(report: &BenchmarkSuiteReport) -> String {
        let mut md = String::new();

        md.push_str("# groundcontrol Retrieval & Indexing Benchmark Report\n\n");
        if let Some(ref b) = report.benchmark {
            md.push_str(&format!("- **Benchmark**: `{b}`\n"));
        }
        if let Some(ref r) = report.repository {
            md.push_str(&format!("- **Repository**: `{r}`\n"));
        }
        md.push_str(&format!(
            "- **Evaluated Queries**: {}\n- **Evaluation Cutoff**: K = {}\n\n",
            report.query_count, report.k
        ));

        // 1. Indexing section (if included)
        if let Some(ref idx) = report.indexing {
            md.push_str("## 1. Indexing Performance & Resource Footprint\n\n");
            md.push_str("| Metric | Value |\n");
            md.push_str("|---|---|\n");
            md.push_str(&format!("| **Corpus Path** | `{}` |\n", idx.corpus_path));
            md.push_str(&format!(
                "| **Documents / Files Indexed** | {} |\n",
                idx.documents_indexed
            ));
            md.push_str(&format!(
                "| **Total Indexing Time** | {:.2} ms ({:.2}s) |\n",
                idx.total_elapsed_ms,
                idx.total_elapsed_ms / 1000.0
            ));
            md.push_str(&format!(
                "| **Reindex Stage (AST/BM25/SIF)** | {:.2} ms |\n",
                idx.reindex_stage_ms
            ));
            md.push_str(&format!("| **Commit & Flush Stage** | {:.2} ms |\n", idx.commit_stage_ms));
            if let Some(reembed_ms) = idx.reembed_stage_ms {
                md.push_str(&format!("| **Dense ONNX Reembed Stage** | {:.2} ms |\n", reembed_ms));
            }
            md.push_str(&format!(
                "| **Indexing Throughput** | **{:.1} files/sec** |\n",
                idx.docs_per_second
            ));
            md.push_str(&format!(
                "| **Graph Topology** | {} nodes, {} edges |\n",
                idx.graph_node_count, idx.graph_edge_count
            ));
            md.push_str(&format!(
                "| **Peak Process RSS** | **{:.2} MB** |\n",
                idx.memory.peak_mb()
            ));
            md.push_str(&format!("| **Memory Delta** | +{:.2} MB |\n", idx.memory.delta_mb()));
            md.push_str(&format!(
                "| **Source Data on Disk** | {:.2} MB ({} files) |\n",
                idx.disk.source_mb(),
                idx.disk.source_file_count
            ));
            md.push_str(&format!(
                "| **Total Index Footprint (`.index/`)** | **{:.2} MB** |\n",
                idx.disk.index_mb()
            ));
            md.push_str(&format!(
                "| **Storage Expansion Ratio** | **{:.2}x** |\n",
                idx.disk.expansion_ratio
            ));
            md.push_str("\n### Index Disk Footprint Breakdown\n\n");
            md.push_str("| Index Component | Size (KB) |\n");
            md.push_str("|---|---|\n");
            md.push_str(&format!(
                "| SQLite Catalog (`meta.db`) | {:.1} KB |\n",
                idx.disk.meta_db_bytes as f64 / 1024.0
            ));
            md.push_str(&format!(
                "| Tantivy BM25 Postings (`tantivy/`) | {:.1} KB |\n",
                idx.disk.tantivy_bytes as f64 / 1024.0
            ));
            md.push_str(&format!(
                "| 256-Bit Binary Fingerprints (`fingerprints.bin`) | {:.1} KB |\n",
                idx.disk.fingerprints_bytes as f64 / 1024.0
            ));
            md.push_str(&format!(
                "| Petgraph Graph (`graph.bin`) | {:.1} KB |\n",
                idx.disk.graph_bytes as f64 / 1024.0
            ));
            if idx.disk.vectors_bytes > 0 {
                md.push_str(&format!(
                    "| Dense Vectors (`vectors.bin`) | {:.1} KB |\n",
                    idx.disk.vectors_bytes as f64 / 1024.0
                ));
            }
            if idx.disk.projections_bytes > 0 {
                md.push_str(&format!(
                    "| Text Projections (`projections/`) | {:.1} KB |\n",
                    idx.disk.projections_bytes as f64 / 1024.0
                ));
            }
            md.push_str("\n---\n\n");
        }

        // 2. Retrieval Ablation Table
        md.push_str("## 2. Retrieval Algorithm Quality & Latency Ablation\n\n");
        md.push_str(&format!("All IR metrics computed at cutoff **K = {}**:\n\n", report.k));

        md.push_str("| Mode | Recall@K | Precision@K | MRR@K | NDCG@K | Sep Ratio | Latency p50 | Latency p95 | Latency p99 | QPS |\n");
        md.push_str("|---|---|---|---|---|---|---|---|---|---|\n");

        for m in &report.modes {
            md.push_str(&format!(
                "| `{}` | **{:.3}** | {:.3} | **{:.3}** | **{:.3}** | {:.2}x | {:.2}ms | {:.2}ms | {:.2}ms | **{:.0}** |\n",
                m.mode.as_str(),
                m.mean_recall,
                m.mean_precision,
                m.mean_mrr,
                m.mean_ndcg,
                m.mean_score_separation,
                m.latency.p50_ms,
                m.latency.p95_ms,
                m.latency.p99_ms,
                m.latency.qps,
            ));
        }

        md.push_str("\n### Metric Descriptions\n");
        md.push_str(
            "- **Recall@K**: Fraction of ground-truth relevant documents retrieved in the top K.\n",
        );
        md.push_str("- **MRR@K**: Mean Reciprocal Rank (1/rank of first relevant result).\n");
        md.push_str("- **NDCG@K**: Normalized Discounted Cumulative Gain accounting for graded relevance.\n");
        md.push_str("- **Sep Ratio**: Score separation between top-1 hit and bottom top-K hit (confidence margin).\n");
        md.push_str("- **QPS**: Sustained queries per second.\n\n");

        md
    }
}
