//! Master aggregate benchmark report generator.
//!
//! Scans a results directory recursively for sub-reports and compiles a unified
//! master leaderboard in Markdown (`summary_report.md`) and CSV (`summary_report.csv`).

use std::fs;
use std::path::{Path, PathBuf};

/// A single row in the aggregated benchmark leaderboard.
#[derive(Debug, Clone)]
pub struct AggregateRow {
    /// Benchmark suite name (e.g. "swe_bench", "codesearchnet", "repobench").
    pub benchmark: String,
    /// Repository name (e.g. "pallets/flask").
    pub repository: String,
    /// Evaluated retrieval mode.
    pub mode: String,
    /// Cutoff rank K.
    pub k: usize,
    /// Mean Recall@K.
    pub mean_recall: f64,
    /// Mean Precision@K.
    pub mean_precision: f64,
    /// Mean MRR@K.
    pub mean_mrr: f64,
    /// Mean NDCG@K.
    pub mean_ndcg: f64,
    /// Mean score separation.
    pub mean_score_separation: f64,
    /// Latency p50 in milliseconds.
    pub latency_p50_ms: f64,
    /// Latency p90 in milliseconds.
    pub latency_p90_ms: f64,
    /// Latency p95 in milliseconds.
    pub latency_p95_ms: f64,
    /// Latency p99 in milliseconds.
    pub latency_p99_ms: f64,
    /// Latency mean in milliseconds.
    pub latency_mean_ms: f64,
    /// Queries per second.
    pub qps: f64,
}

/// Aggregator that compiles multiple sub-report CSVs into a unified master leaderboard.
pub struct ReportAggregator;

impl ReportAggregator {
    /// Recursively find all `*_report.csv` files under `results_dir`, excluding `summary_report.csv`.
    pub fn find_report_csvs(dir: &Path) -> Vec<PathBuf> {
        let mut csvs = Vec::new();
        Self::collect_csvs(dir, &mut csvs);
        csvs.sort();
        csvs
    }

    fn collect_csvs(dir: &Path, acc: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::collect_csvs(&path, acc);
                } else if path.is_file() {
                    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if file_name.ends_with("_report.csv") && file_name != "summary_report.csv" {
                        acc.push(path);
                    }
                }
            }
        }
    }

    /// Parse rows from a given CSV file.
    pub fn parse_csv(path: &Path) -> Vec<AggregateRow> {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() <= 1 {
            return Vec::new();
        }

        let header = lines[0];
        let has_benchmark_col = header.starts_with("benchmark,repository,");

        // Infer benchmark and repo fallback from path if needed
        let parent_name =
            path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("bench");

        let file_stem =
            path.file_stem().and_then(|s| s.to_str()).unwrap_or("").trim_end_matches("_report");

        let mut rows = Vec::new();

        for line in &lines[1..] {
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if has_benchmark_col && parts.len() >= 15 {
                let row = AggregateRow {
                    benchmark: parts[0].to_string(),
                    repository: parts[1].to_string(),
                    mode: parts[2].to_string(),
                    k: parts[3].parse().unwrap_or(10),
                    mean_recall: parts[4].parse().unwrap_or(0.0),
                    mean_precision: parts[5].parse().unwrap_or(0.0),
                    mean_mrr: parts[6].parse().unwrap_or(0.0),
                    mean_ndcg: parts[7].parse().unwrap_or(0.0),
                    mean_score_separation: parts[8].parse().unwrap_or(0.0),
                    latency_p50_ms: parts[9].parse().unwrap_or(0.0),
                    latency_p90_ms: parts[10].parse().unwrap_or(0.0),
                    latency_p95_ms: parts[11].parse().unwrap_or(0.0),
                    latency_p99_ms: parts[12].parse().unwrap_or(0.0),
                    latency_mean_ms: parts[13].parse().unwrap_or(0.0),
                    qps: parts[14].parse().unwrap_or(0.0),
                };
                rows.push(row);
            } else if !has_benchmark_col && parts.len() >= 13 {
                // Older format without benchmark,repository columns
                let row = AggregateRow {
                    benchmark: parent_name.to_string(),
                    repository: file_stem.replace("__", "/"),
                    mode: parts[0].to_string(),
                    k: parts[1].parse().unwrap_or(10),
                    mean_recall: parts[2].parse().unwrap_or(0.0),
                    mean_precision: parts[3].parse().unwrap_or(0.0),
                    mean_mrr: parts[4].parse().unwrap_or(0.0),
                    mean_ndcg: parts[5].parse().unwrap_or(0.0),
                    mean_score_separation: parts[6].parse().unwrap_or(0.0),
                    latency_p50_ms: parts[7].parse().unwrap_or(0.0),
                    latency_p90_ms: parts[8].parse().unwrap_or(0.0),
                    latency_p95_ms: parts[9].parse().unwrap_or(0.0),
                    latency_p99_ms: parts[10].parse().unwrap_or(0.0),
                    latency_mean_ms: parts[11].parse().unwrap_or(0.0),
                    qps: parts[12].parse().unwrap_or(0.0),
                };
                rows.push(row);
            }
        }

        rows
    }

    /// Aggregate all CSVs under `results_dir` into master Markdown and CSV strings.
    pub fn aggregate(results_dir: &Path) -> (String, String) {
        let csv_paths = Self::find_report_csvs(results_dir);
        let mut all_rows = Vec::new();

        for p in &csv_paths {
            let mut file_rows = Self::parse_csv(p);
            all_rows.append(&mut file_rows);
        }

        // Mode canonical ordering
        fn mode_priority(m: &str) -> usize {
            match m {
                "bm25" => 1,
                "binary" => 2,
                "ppr" => 3,
                "fast" => 4,
                "semantic" => 5,
                "full" => 6,
                _ => 10,
            }
        }

        all_rows.sort_by(|a, b| {
            a.benchmark
                .cmp(&b.benchmark)
                .then_with(|| a.repository.cmp(&b.repository))
                .then_with(|| mode_priority(&a.mode).cmp(&mode_priority(&b.mode)))
        });

        // 1. Render CSV
        let mut csv_out = String::new();
        csv_out.push_str("benchmark,repository,mode,k,mean_recall,mean_precision,mean_mrr,mean_ndcg,mean_score_separation,latency_p50_ms,latency_p90_ms,latency_p95_ms,latency_p99_ms,latency_mean_ms,qps\n");
        for r in &all_rows {
            csv_out.push_str(&format!(
                "{},{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.3},{:.3},{:.3},{:.3},{:.3},{:.1}\n",
                r.benchmark,
                r.repository,
                r.mode,
                r.k,
                r.mean_recall,
                r.mean_precision,
                r.mean_mrr,
                r.mean_ndcg,
                r.mean_score_separation,
                r.latency_p50_ms,
                r.latency_p90_ms,
                r.latency_p95_ms,
                r.latency_p99_ms,
                r.latency_mean_ms,
                r.qps,
            ));
        }

        // 2. Render Markdown
        let mut md_out = String::new();
        md_out.push_str("# groundcontrol Comprehensive Benchmark Master Summary\n\n");
        md_out.push_str(&format!(
            "- **Aggregated Sub-Reports**: {} evaluation suites\n- **Total Configurations**: {} benchmark rows\n\n",
            csv_paths.len(),
            all_rows.len()
        ));

        md_out.push_str("## 1. Master Leaderboard\n\n");
        md_out.push_str("| Benchmark | Repository | Mode | Recall@K | MRR@K | NDCG@K | Sep Ratio | Latency p50 | QPS |\n");
        md_out.push_str("|---|---|---|---|---|---|---|---|---|\n");

        for r in &all_rows {
            md_out.push_str(&format!(
                "| `{}` | `{}` | `{}` | **{:.3}** | **{:.3}** | **{:.3}** | {:.2}x | {:.2}ms | **{:.0}** |\n",
                r.benchmark,
                r.repository,
                r.mode,
                r.mean_recall,
                r.mean_mrr,
                r.mean_ndcg,
                r.mean_score_separation,
                r.latency_p50_ms,
                r.qps,
            ));
        }

        // Mode macro-averages
        if !all_rows.is_empty() {
            md_out.push_str("\n## 2. Mode Macro-Averages\n\n");
            md_out.push_str("| Mode | Configurations | Avg Recall@K | Avg MRR@K | Avg NDCG@K | Avg Sep Ratio | Avg Latency p50 | Avg QPS |\n");
            md_out.push_str("|---|---|---|---|---|---|---|---|\n");

            let mut by_mode: std::collections::HashMap<String, Vec<&AggregateRow>> =
                std::collections::HashMap::new();
            for r in &all_rows {
                by_mode.entry(r.mode.clone()).or_default().push(r);
            }

            let mut modes: Vec<String> = by_mode.keys().cloned().collect();
            modes.sort_by_key(|m| mode_priority(m));

            for m in modes {
                let list = &by_mode[&m];
                let count = list.len() as f64;
                let avg_recall: f64 = list.iter().map(|r| r.mean_recall).sum::<f64>() / count;
                let avg_mrr: f64 = list.iter().map(|r| r.mean_mrr).sum::<f64>() / count;
                let avg_ndcg: f64 = list.iter().map(|r| r.mean_ndcg).sum::<f64>() / count;
                let avg_sep: f64 =
                    list.iter().map(|r| r.mean_score_separation).sum::<f64>() / count;
                let avg_p50: f64 = list.iter().map(|r| r.latency_p50_ms).sum::<f64>() / count;
                let avg_qps: f64 = list.iter().map(|r| r.qps).sum::<f64>() / count;

                md_out.push_str(&format!(
                    "| `{}` | {} | **{:.3}** | **{:.3}** | **{:.3}** | {:.2}x | {:.2}ms | **{:.0}** |\n",
                    m,
                    list.len(),
                    avg_recall,
                    avg_mrr,
                    avg_ndcg,
                    avg_sep,
                    avg_p50,
                    avg_qps,
                ));
            }
        }

        md_out.push_str("\n### Metric Descriptions\n");
        md_out.push_str(
            "- **Recall@K**: Fraction of ground-truth target files retrieved in top K.\n",
        );
        md_out.push_str("- **MRR@K**: Mean Reciprocal Rank of first relevant document.\n");
        md_out.push_str("- **NDCG@K**: Normalized Discounted Cumulative Gain accounting for graded relevance.\n");
        md_out.push_str("- **Sep Ratio**: Score separation confidence margin between top-1 hit and bottom top-K hit.\n");
        md_out.push_str("- **QPS**: Measured sustained queries per second.\n\n");

        (md_out, csv_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_aggregate_reports() {
        let tmp = TempDir::new().unwrap();
        let swe_dir = tmp.path().join("swe_bench");
        fs::create_dir_all(&swe_dir).unwrap();

        let csv1 = "benchmark,repository,mode,k,mean_recall,mean_precision,mean_mrr,mean_ndcg,mean_score_separation,latency_p50_ms,latency_p90_ms,latency_p95_ms,latency_p99_ms,latency_mean_ms,qps\n\
swe_bench,pallets/flask,bm25,10,0.5000,0.1000,0.5000,0.5500,1.2000,2.100,3.100,4.100,5.100,2.500,400.0\n\
swe_bench,pallets/flask,fast,10,0.6000,0.1200,0.6000,0.6500,1.4000,2.800,3.800,4.800,5.800,3.000,357.1\n";
        fs::write(swe_dir.join("pallets__flask_report.csv"), csv1).unwrap();

        let (md, csv) = ReportAggregator::aggregate(tmp.path());
        assert!(md.contains("Master Leaderboard"));
        assert!(md.contains("pallets/flask"));
        assert!(md.contains("bm25"));
        assert!(md.contains("fast"));
        assert!(csv.contains("pallets/flask"));
    }
}
