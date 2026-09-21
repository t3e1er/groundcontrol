//! CSV report generator for data science analysis (Pandas / R / Excel).

use crate::sweep::BenchmarkSuiteReport;

/// CSV tabular exporter for benchmark suite reports.
pub struct CsvReporter;

impl CsvReporter {
    /// Render retrieval ablation metrics as a CSV table.
    pub fn render_retrieval_csv(report: &BenchmarkSuiteReport) -> String {
        let benchmark = report.benchmark.as_deref().unwrap_or("benchmark");
        let repository = report.repository.as_deref().unwrap_or("repository");
        let mut csv = String::new();
        csv.push_str("benchmark,repository,mode,k,mean_recall,mean_precision,mean_mrr,mean_ndcg,mean_score_separation,latency_p50_ms,latency_p90_ms,latency_p95_ms,latency_p99_ms,latency_mean_ms,qps\n");

        for m in &report.modes {
            csv.push_str(&format!(
                "{},{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.3},{:.3},{:.3},{:.3},{:.3},{:.1}\n",
                benchmark,
                repository,
                m.mode.as_str(),
                m.k,
                m.mean_recall,
                m.mean_precision,
                m.mean_mrr,
                m.mean_ndcg,
                m.mean_score_separation,
                m.latency.p50_ms,
                m.latency.p90_ms,
                m.latency.p95_ms,
                m.latency.p99_ms,
                m.latency.mean_ms,
                m.latency.qps,
            ));
        }

        csv
    }
}
