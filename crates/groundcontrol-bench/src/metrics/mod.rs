//! Evaluation metrics for ranking quality, statistical significance, and latency.

pub mod ir;
pub mod latency;
pub mod significance;

pub use ir::{IrEvaluator, QueryEvaluationMetrics};
pub use latency::{LatencyStats, LatencyTracker};
pub use significance::{SignificanceEvaluator, SignificanceResult};
