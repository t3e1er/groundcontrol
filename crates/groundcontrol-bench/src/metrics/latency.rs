//! Latency distribution and throughput statistics.

use serde::{Deserialize, Serialize};

/// Summary of latency distribution and throughput.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatencyStats {
    /// Total number of recorded samples.
    pub count: usize,
    /// 50th percentile (median) latency in milliseconds.
    pub p50_ms: f64,
    /// 90th percentile latency in milliseconds.
    pub p90_ms: f64,
    /// 95th percentile latency in milliseconds.
    pub p95_ms: f64,
    /// 99th percentile latency in milliseconds.
    pub p99_ms: f64,
    /// Mean latency in milliseconds.
    pub mean_ms: f64,
    /// Minimum latency in milliseconds.
    pub min_ms: f64,
    /// Maximum latency in milliseconds.
    pub max_ms: f64,
    /// Queries per second (throughput).
    pub qps: f64,
}

/// Utility for tracking latency measurements and computing percentiles.
#[derive(Debug, Clone, Default)]
pub struct LatencyTracker {
    samples_ms: Vec<f64>,
}

impl LatencyTracker {
    /// Create a new latency tracker.
    pub fn new() -> Self {
        Self { samples_ms: Vec::new() }
    }

    /// Record a single query latency measurement in milliseconds.
    pub fn record(&mut self, ms: f64) {
        self.samples_ms.push(ms);
    }

    /// Merge another latency tracker's samples into this one.
    pub fn merge(&mut self, other: LatencyTracker) {
        self.samples_ms.extend(other.samples_ms);
    }

    /// Calculate latency percentiles and summary statistics.
    pub fn compute(&self) -> LatencyStats {
        if self.samples_ms.is_empty() {
            return LatencyStats {
                count: 0,
                p50_ms: 0.0,
                p90_ms: 0.0,
                p95_ms: 0.0,
                p99_ms: 0.0,
                mean_ms: 0.0,
                min_ms: 0.0,
                max_ms: 0.0,
                qps: 0.0,
            };
        }

        let mut sorted = self.samples_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let count = sorted.len();
        let total: f64 = sorted.iter().sum();
        let mean_ms = total / count as f64;
        let min_ms = sorted[0];
        let max_ms = sorted[count - 1];

        let p50_ms = Self::percentile(&sorted, 0.50);
        let p90_ms = Self::percentile(&sorted, 0.90);
        let p95_ms = Self::percentile(&sorted, 0.95);
        let p99_ms = Self::percentile(&sorted, 0.99);

        let total_seconds = total / 1000.0;
        let qps = if total_seconds > 0.0 { count as f64 / total_seconds } else { 0.0 };

        LatencyStats { count, p50_ms, p90_ms, p95_ms, p99_ms, mean_ms, min_ms, max_ms, qps }
    }

    fn percentile(sorted: &[f64], pct: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let rank = pct * (sorted.len() - 1) as f64;
        let lower = rank.floor() as usize;
        let upper = rank.ceil() as usize;
        let weight = rank - lower as f64;
        sorted[lower] * (1.0 - weight) + sorted[upper] * weight
    }
}
