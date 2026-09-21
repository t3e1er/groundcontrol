//! Benchmark query and relevance judgment schema.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Graded relevance judgment for a document/chunk path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelevanceJudgment {
    /// Target path (relative to corpus root) or symbol identifier.
    pub path: String,
    /// Graded relevance score:
    /// - 3: Exact match / primary ground truth
    /// - 2: Highly relevant
    /// - 1: Partially relevant / related
    /// - 0: Irrelevant
    pub grade: u8,
}

impl RelevanceJudgment {
    /// Create a new relevance judgment with a specified grade.
    pub fn new(path: impl Into<String>, grade: u8) -> Self {
        Self { path: path.into(), grade }
    }
}

/// A benchmark query with ground-truth relevance judgments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkQuery {
    /// Unique identifier for this evaluation query.
    pub id: String,
    /// Natural language or code search query string.
    pub query: String,
    /// Expected relevant document/symbol paths or graded judgments.
    pub expected: Vec<RelevanceJudgment>,
    /// Target repository identifier or slug (e.g. "pallets/flask", "astropy/astropy").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Optional category (e.g. "exact_symbol", "error_handling", "concept_synonym", "cross_modal").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

impl BenchmarkQuery {
    /// Create a query with a simple list of expected paths (defaulting to grade 1).
    pub fn simple(
        id: impl Into<String>,
        query: impl Into<String>,
        paths: Vec<String>,
        category: Option<String>,
    ) -> Self {
        let expected = paths.into_iter().map(|p| RelevanceJudgment::new(p, 1)).collect();
        let cat = category;
        Self {
            id: id.into(),
            query: query.into(),
            expected,
            repository: cat.clone(),
            category: cat,
        }
    }

    /// Return the target repository, falling back to category if repository is unset.
    pub fn target_repository(&self) -> Option<&str> {
        self.repository.as_deref().or(self.category.as_deref())
    }

    /// Map expected paths to their grades for O(1) lookup.
    pub fn grade_map(&self) -> HashMap<String, u8> {
        self.expected.iter().map(|j| (j.path.clone(), j.grade)).collect()
    }
}

/// Ground-truth benchmark dataset.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BenchmarkDataset {
    /// Name or description of the dataset.
    pub name: Option<String>,
    /// List of evaluation queries.
    pub queries: Vec<BenchmarkQuery>,
}
