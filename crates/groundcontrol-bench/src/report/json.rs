//! JSON report serializer for machine-readable benchmarks and CI regression tracking.

use crate::sweep::BenchmarkSuiteReport;

/// JSON serializer for benchmark suite reports.
pub struct JsonReporter;

impl JsonReporter {
    /// Serialize a report to a pretty-printed JSON string.
    pub fn to_string(report: &BenchmarkSuiteReport) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(report)
    }
}
