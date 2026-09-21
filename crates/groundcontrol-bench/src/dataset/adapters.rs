//! Converters and adapters for public code retrieval benchmark formats:
//! CodeSearchNet (AdvTest), RepoBench-R, and SWE-bench Lite.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;

use super::schema::{BenchmarkDataset, BenchmarkQuery, RelevanceJudgment};

/// Error encountered during benchmark conversion.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// IO error reading dataset file.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON deserialization error.
    #[error("JSON error at line {line}: {source}")]
    Json {
        /// Line number (1-based) where error occurred.
        line: usize,
        /// Underlying serde error.
        #[source]
        source: serde_json::Error,
    },
    /// Invalid dataset structure.
    #[error("Invalid dataset format: {0}")]
    InvalidFormat(String),
}

/// Supported external benchmark formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicBenchmarkFormat {
    /// CodeSearchNet / CSN-AdvTest JSONL format.
    CodeSearchNet,
    /// RepoBench-R cross-file retrieval JSONL format.
    RepoBench,
    /// SWE-bench / SWE-bench Lite JSON/JSONL format.
    SweBench,
}

/// Raw record for CodeSearchNet / AdvTest.
#[derive(Debug, Deserialize)]
struct CsnRecord {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    docstring: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    repo_name: Option<String>,
    #[serde(default)]
    repo: Option<String>,
}

/// Raw record for RepoBench-R.
#[derive(Debug, Deserialize)]
struct RepoBenchRecord {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    context: Option<serde_json::Value>,
    #[serde(default)]
    gold_snippet_path: Option<String>,
    #[serde(default)]
    gold_path: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    repo_name: Option<String>,
}

/// Raw record for SWE-bench Lite.
#[derive(Debug, Deserialize)]
struct SweBenchRecord {
    #[serde(default)]
    instance_id: Option<String>,
    #[serde(default)]
    problem_statement: Option<String>,
    #[serde(default)]
    patch: Option<String>,
    #[serde(default)]
    repo: Option<String>,
}

/// High-level adapter for converting public benchmark files to `BenchmarkDataset`.
pub struct PublicBenchmarkAdapter;

impl PublicBenchmarkAdapter {
    /// Convert a dataset file of the specified public benchmark format into a `BenchmarkDataset`.
    pub fn convert_file(
        path: &Path,
        format: PublicBenchmarkFormat,
    ) -> Result<BenchmarkDataset, AdapterError> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        match format {
            PublicBenchmarkFormat::CodeSearchNet => Self::convert_csn(reader),
            PublicBenchmarkFormat::RepoBench => Self::convert_repobench(reader),
            PublicBenchmarkFormat::SweBench => Self::convert_swebench(reader),
        }
    }

    /// Convert CodeSearchNet / AdvTest JSONL stream.
    pub fn convert_csn<R: BufRead>(reader: R) -> Result<BenchmarkDataset, AdapterError> {
        let mut queries = Vec::new();

        for (line_idx, line_res) in reader.lines().enumerate() {
            let line = line_res?;
            let trimmed = line.trim_start_matches('\u{feff}').trim();
            if trimmed.is_empty() {
                continue;
            }

            let rec: CsnRecord = serde_json::from_str(trimmed)
                .map_err(|e| AdapterError::Json { line: line_idx + 1, source: e })?;

            let query_text = rec.query.or(rec.docstring).unwrap_or_default().trim().to_string();

            if query_text.is_empty() {
                continue;
            }

            let raw_path = rec.path.unwrap_or_else(|| "unknown".to_string());
            let clean_path = if let Some(hash_pos) = raw_path.find('#') {
                raw_path[..hash_pos].to_string()
            } else {
                raw_path
            };
            let grade = 3; // exact symbol/file ground truth
            let judgment = RelevanceJudgment::new(clean_path, grade);

            let id = format!(
                "csn_{}_{:05}",
                rec.language.as_deref().unwrap_or("polyglot"),
                line_idx + 1
            );

            let repo = rec.repo_name.or(rec.repo);
            queries.push(BenchmarkQuery {
                id,
                query: query_text,
                expected: vec![judgment],
                repository: repo.clone(),
                category: repo.or(rec.language).or_else(|| Some("codesearchnet".to_string())),
            });
        }

        Ok(BenchmarkDataset { name: Some("CodeSearchNet-AdvTest".to_string()), queries })
    }

    /// Convert RepoBench-R JSONL stream.
    pub fn convert_repobench<R: BufRead>(reader: R) -> Result<BenchmarkDataset, AdapterError> {
        let mut queries = Vec::new();

        for (line_idx, line_res) in reader.lines().enumerate() {
            let line = line_res?;
            let trimmed = line.trim_start_matches('\u{feff}').trim();
            if trimmed.is_empty() {
                continue;
            }

            let rec: RepoBenchRecord = serde_json::from_str(trimmed)
                .map_err(|e| AdapterError::Json { line: line_idx + 1, source: e })?;

            let query_text = if let Some(q) = rec.query {
                let trimmed_q = q.trim();
                if !trimmed_q.is_empty() {
                    trimmed_q.to_string()
                } else {
                    extract_repobench_context(rec.context)
                }
            } else {
                extract_repobench_context(rec.context)
            };

            if query_text.is_empty() {
                continue;
            }

            let gold_path = rec
                .gold_snippet_path
                .or(rec.gold_path)
                .or(rec.file_path)
                .unwrap_or_else(|| "unknown".to_string());

            let id = rec.id.unwrap_or_else(|| format!("repobench_{:05}", line_idx + 1));

            let repo = rec.repo_name;
            queries.push(BenchmarkQuery {
                id,
                query: query_text,
                expected: vec![RelevanceJudgment::new(gold_path, 3)],
                repository: repo.clone(),
                category: repo.or_else(|| Some("repobench".to_string())),
            });
        }

        Ok(BenchmarkDataset { name: Some("RepoBench-R".to_string()), queries })
    }

    /// Convert SWE-bench / SWE-bench Lite JSON/JSONL stream.
    /// Extracts ground truth file targets from the unified git diff patch.
    pub fn convert_swebench<R: BufRead>(mut reader: R) -> Result<BenchmarkDataset, AdapterError> {
        let mut content = String::new();
        reader.read_to_string(&mut content)?;

        let trimmed = content.trim_start_matches('\u{feff}').trim();
        let records: Vec<SweBenchRecord> = if trimmed.starts_with('[') {
            serde_json::from_str(trimmed).map_err(|e| AdapterError::Json { line: 1, source: e })?
        } else {
            // Treat as JSONL
            let mut recs = Vec::new();
            for (idx, line) in trimmed.lines().enumerate() {
                let l = line.trim();
                if l.is_empty() {
                    continue;
                }
                let rec: SweBenchRecord = serde_json::from_str(l)
                    .map_err(|e| AdapterError::Json { line: idx + 1, source: e })?;
                recs.push(rec);
            }
            recs
        };

        let mut queries = Vec::new();

        for (idx, rec) in records.into_iter().enumerate() {
            let query_text = rec.problem_statement.unwrap_or_default().trim().to_string();
            if query_text.is_empty() {
                continue;
            }

            let mut target_paths = BTreeSet::new();
            if let Some(patch) = rec.patch {
                for line in patch.lines() {
                    if let Some(rest) = line.strip_prefix("--- a/") {
                        let path = rest.trim();
                        if path != "/dev/null" && !path.is_empty() {
                            target_paths.insert(path.to_string());
                        }
                    } else if let Some(rest) = line.strip_prefix("+++ b/") {
                        let path = rest.trim();
                        if path != "/dev/null" && !path.is_empty() {
                            target_paths.insert(path.to_string());
                        }
                    }
                }
            }

            let judgments =
                target_paths.into_iter().map(|p| RelevanceJudgment::new(p, 3)).collect();

            let id = rec.instance_id.unwrap_or_else(|| format!("swe_{:05}", idx + 1));

            let repo = rec.repo;
            queries.push(BenchmarkQuery {
                id,
                query: query_text,
                expected: judgments,
                repository: repo.clone(),
                category: repo.or_else(|| Some("swe-bench".to_string())),
            });
        }

        Ok(BenchmarkDataset { name: Some("SWE-bench-Lite".to_string()), queries })
    }
}

fn extract_repobench_context(val: Option<serde_json::Value>) -> String {
    match val {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Array(arr)) => arr
            .into_iter()
            .filter_map(|v| match v {
                serde_json::Value::String(s) => Some(s),
                serde_json::Value::Object(obj) => obj
                    .get("snippet")
                    .and_then(|s| s.as_str())
                    .or_else(|| obj.get("identifier").and_then(|i| i.as_str()))
                    .map(ToString::to_string),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
        Some(other) => other.to_string().trim().to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_csn_conversion() {
        let jsonl = r#"
{"docstring": "Compute SHA256 checksum", "path": "crypto/sha256.go", "language": "go"}
{"docstring": "Authenticate JWT token", "path": "auth/jwt.go", "language": "go"}
"#;
        let dataset =
            PublicBenchmarkAdapter::convert_csn(jsonl.as_bytes()).expect("Conversion failed");
        assert_eq!(dataset.queries.len(), 2);
        assert_eq!(dataset.queries[0].query, "Compute SHA256 checksum");
        assert_eq!(dataset.queries[0].expected[0].path, "crypto/sha256.go");
        assert_eq!(dataset.queries[0].expected[0].grade, 3);
    }

    #[test]
    fn test_repobench_conversion() {
        let jsonl = r#"
{"id": "rb_01", "query": "import logger from utils", "gold_snippet_path": "src/utils/logger.ts", "repo_name": "ts-repo"}
"#;
        let dataset =
            PublicBenchmarkAdapter::convert_repobench(jsonl.as_bytes()).expect("Conversion failed");
        assert_eq!(dataset.queries.len(), 1);
        assert_eq!(dataset.queries[0].id, "rb_01");
        assert_eq!(dataset.queries[0].expected[0].path, "src/utils/logger.ts");
    }

    #[test]
    fn test_repobench_sequence_context() {
        let jsonl = r#"
{"id": "rb_02", "query": "import re", "context": [{"identifier": "registry", "path": "reg.py", "snippet": "class Registry:\n pass"}], "gold_snippet_path": "blip.py", "repo_name": "repo"}
"#;
        let dataset =
            PublicBenchmarkAdapter::convert_repobench(jsonl.as_bytes()).expect("Conversion failed");
        assert_eq!(dataset.queries.len(), 1);
        assert_eq!(dataset.queries[0].id, "rb_02");
        assert_eq!(dataset.queries[0].query, "import re");
        assert_eq!(dataset.queries[0].expected[0].path, "blip.py");
    }

    #[test]
    fn test_swebench_conversion() {
        let json = r#"[
  {
    "instance_id": "django__django-11001",
    "problem_statement": "Incorrect ordering of clauses in SQL query",
    "patch": "diff --git a/django/db/models/sql/query.py b/django/db/models/sql/query.py\n--- a/django/db/models/sql/query.py\n+++ b/django/db/models/sql/query.py\n@@ -1,3 +1,3 @@",
    "repo": "django/django"
  }
]"#;
        let dataset =
            PublicBenchmarkAdapter::convert_swebench(json.as_bytes()).expect("Conversion failed");
        assert_eq!(dataset.queries.len(), 1);
        assert_eq!(dataset.queries[0].id, "django__django-11001");
        assert_eq!(dataset.queries[0].expected[0].path, "django/db/models/sql/query.py");
    }
}
