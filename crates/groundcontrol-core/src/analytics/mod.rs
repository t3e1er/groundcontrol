//! Analytics tools: density analysis, semantic gap detection, split suggestions, coverage reports.
//!
//! These tools provide insights into corpus quality, index coverage, and opportunities
//! for improving retrieval performance.

pub mod coverage;
pub mod density;
pub mod gaps;
pub mod splits;

#[cfg(test)]
mod tests;

pub use coverage::{coverage_report, CoverageReport, QueryCoverage};
pub use density::{analyze_density, CommunityDensityInfo, DensityReport, HubInfo, TagDensity};
pub use gaps::{find_semantic_gaps, SemanticGap};
pub use splits::{suggest_splits, SplitSuggestion};
