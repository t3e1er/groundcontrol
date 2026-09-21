//! Dataset loader supporting both simple expected paths and graded judgments.

use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::schema::{BenchmarkDataset, BenchmarkQuery, RelevanceJudgment};

/// Raw flexible JSON query structure to accept various legacy and new schema variants.
#[derive(Debug, Deserialize)]
struct RawQueryItem {
    id: Option<String>,
    query: String,
    expected: Option<Vec<Value>>,
    expected_relevant: Option<Vec<String>>,
    repository: Option<String>,
    category: Option<String>,
}

/// Utility for loading benchmark datasets from disk.
pub struct DatasetLoader;

impl DatasetLoader {
    /// Load a benchmark dataset from a JSON file.
    pub fn load_from_file(path: &Path) -> Result<BenchmarkDataset, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read dataset file {}: {e}", path.display()))?;
        Self::load_from_str(&content)
    }

    /// Load a benchmark dataset from a JSON string.
    pub fn load_from_str(json_str: &str) -> Result<BenchmarkDataset, String> {
        let raw_val: Value =
            serde_json::from_str(json_str).map_err(|e| format!("Invalid JSON dataset: {e}"))?;

        let items: Vec<RawQueryItem> = if let Some(arr) = raw_val.as_array() {
            serde_json::from_value(Value::Array(arr.clone()))
                .map_err(|e| format!("Failed to parse query list: {e}"))?
        } else if let Some(queries_val) = raw_val.get("queries").and_then(|q| q.as_array()) {
            serde_json::from_value(Value::Array(queries_val.clone()))
                .map_err(|e| format!("Failed to parse queries property: {e}"))?
        } else {
            return Err(
                "Expected JSON array of queries or an object with a 'queries' array".to_string()
            );
        };

        let mut queries = Vec::new();
        for (idx, item) in items.into_iter().enumerate() {
            let id = item.id.unwrap_or_else(|| format!("q_{:03}", idx + 1));
            let mut judgments = Vec::new();

            if let Some(expected_arr) = item.expected {
                for entry in expected_arr {
                    if let Some(path_str) = entry.as_str() {
                        judgments.push(RelevanceJudgment::new(path_str, 1));
                    } else if let Some(obj) = entry.as_object() {
                        if let Some(p) = obj.get("path").and_then(|v| v.as_str()) {
                            let grade =
                                obj.get("grade").and_then(|g| g.as_u64()).unwrap_or(1) as u8;
                            judgments.push(RelevanceJudgment::new(p, grade));
                        }
                    }
                }
            } else if let Some(paths) = item.expected_relevant {
                for p in paths {
                    judgments.push(RelevanceJudgment::new(p, 1));
                }
            }

            let repo = item.repository.clone().or_else(|| item.category.clone());
            queries.push(BenchmarkQuery {
                id,
                query: item.query,
                expected: judgments,
                repository: repo,
                category: item.category,
            });
        }

        Ok(BenchmarkDataset { name: None, queries })
    }
}
