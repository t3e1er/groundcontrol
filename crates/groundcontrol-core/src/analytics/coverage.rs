//! Note coverage report: identifying notes never retrieved across a test query suite.

use std::collections::HashSet;

use groundcontrol_common::Result;
use serde::{Deserialize, Serialize};

use crate::index::BM25Index;

/// Coverage report result for a test query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryCoverage {
    /// The test query.
    pub query: String,
    /// Documents retrieved for this query.
    pub retrieved: Vec<String>,
    /// Number of documents retrieved.
    pub retrieved_count: usize,
}

/// Coverage report showing which notes are never retrieved across a set of test queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Total number of notes in the corpus.
    pub total_notes: usize,
    /// Number of notes retrieved by at least one query.
    pub covered_notes: usize,
    /// Notes never retrieved by any test query (dead zones).
    pub uncovered_notes: Vec<String>,
    /// Coverage ratio (covered / total).
    pub coverage_ratio: f64,
    /// Per-query retrieval details.
    pub per_query: Vec<QueryCoverage>,
}

/// Generate a coverage report: for a set of test queries, find which notes are never retrieved.
///
/// - `bm25`: BM25 index to search.
/// - `queries`: Set of test queries.
/// - `all_note_paths`: All known note paths in the corpus.
/// - `top_k`: Number of results to consider per query.
pub fn coverage_report(
    bm25: &BM25Index,
    queries: &[&str],
    all_note_paths: &[String],
    top_k: usize,
) -> Result<CoverageReport> {
    let mut ever_retrieved: HashSet<String> = HashSet::new();
    let mut per_query = Vec::new();

    for query in queries {
        let results = bm25.search(query, top_k)?;
        let retrieved: Vec<String> = results.iter().map(|r| r.path.clone()).collect();

        for path in &retrieved {
            let _ = ever_retrieved.insert(path.clone());
        }

        per_query.push(QueryCoverage {
            query: query.to_string(),
            retrieved_count: retrieved.len(),
            retrieved,
        });
    }

    let total_notes = all_note_paths.len();
    let covered_notes = ever_retrieved.len();
    let uncovered_notes: Vec<String> =
        all_note_paths.iter().filter(|p| !ever_retrieved.contains(p.as_str())).cloned().collect();
    let coverage_ratio =
        if total_notes > 0 { covered_notes as f64 / total_notes as f64 } else { 1.0 };

    Ok(CoverageReport { total_notes, covered_notes, uncovered_notes, coverage_ratio, per_query })
}
