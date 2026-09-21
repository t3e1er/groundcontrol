//! Retrieval algorithm execution runners.

pub mod sanitizer;

use std::str::FromStr;
use std::time::Instant;

use groundcontrol_common::ports::{SearchQuery, SearchService};
use groundcontrol_common::types::{Modality, SearchResult};
use groundcontrol_core::engine::Engine;
use serde::{Deserialize, Serialize};

use crate::dataset::schema::BenchmarkQuery;
use sanitizer::sanitize_lucene_query;

/// Individual retrieval modes supported for benchmarking and ablation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode {
    /// Pure Tantivy Okapi BM25 lexical retrieval.
    Bm25,
    /// Isolated SIF projection + 256-bit MRL binary Hamming scan.
    Binary,
    /// Isolated HippoRAG 2-hop personalized PageRank diffusion on Petgraph.
    Ppr,
    /// Fast algorithmic hybrid (BM25 + Binary Hamming + PPR via 3-way RRF).
    Fast,
    /// Pure dense ONNX neural embeddings (cosine similarity).
    Semantic,
    /// Full 3-signal hybrid (BM25 + ONNX + BFS graph proximity).
    Full,
}

impl RetrievalMode {
    /// Canonical string identifier for this retrieval mode.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bm25 => "bm25",
            Self::Binary => "binary",
            Self::Ppr => "ppr",
            Self::Fast => "fast",
            Self::Semantic => "semantic",
            Self::Full => "full",
        }
    }

    /// List of all standard ablation modes.
    pub fn all() -> &'static [RetrievalMode] {
        &[Self::Bm25, Self::Binary, Self::Ppr, Self::Fast, Self::Semantic, Self::Full]
    }
}

impl FromStr for RetrievalMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "bm25" | "lexical" => Ok(Self::Bm25),
            "binary" | "sif" | "hamming" => Ok(Self::Binary),
            "ppr" | "pagerank" | "diffusion" => Ok(Self::Ppr),
            "fast" | "fast_hybrid" => Ok(Self::Fast),
            "semantic" | "vector" | "dense" => Ok(Self::Semantic),
            "full" | "hybrid" => Ok(Self::Full),
            unknown => Err(format!(
                "Unknown retrieval mode '{unknown}'. Valid modes: bm25, binary, ppr, fast, semantic, full"
            )),
        }
    }
}

/// Options configuring a query execution run.
#[derive(Debug, Clone)]
pub struct QueryRunnerOptions {
    /// Number of candidates to retrieve.
    pub limit: usize,
    /// Modality filter (`both`, `code`, or `docs`).
    pub modality: Modality,
    /// Whether to enable query decomposition.
    pub decompose: bool,
}

impl Default for QueryRunnerOptions {
    fn default() -> Self {
        Self { limit: 10, modality: Modality::Both, decompose: false }
    }
}

/// Query runner that executes searches against an `Engine` and measures elapsed time.
pub struct QueryRunner;

impl QueryRunner {
    /// Execute a benchmark query using the specified mode and return the results and elapsed milliseconds.
    pub fn execute(
        engine: &Engine,
        query: &BenchmarkQuery,
        mode: RetrievalMode,
        options: &QueryRunnerOptions,
    ) -> groundcontrol_common::Result<(Vec<SearchResult>, f64)> {
        let t_start = Instant::now();

        let sanitized = sanitize_lucene_query(&query.query);
        let search_text = if sanitized.is_empty() { query.query.clone() } else { sanitized };

        let results = match mode {
            RetrievalMode::Bm25 => {
                let sq = SearchQuery {
                    query: search_text.clone(),
                    mode: Some("bm25".to_string()),
                    limit: Some(options.limit * 5),
                    modality: options.modality,
                    decompose: Some(options.decompose),
                    ..Default::default()
                };
                let raw_hits = engine.search_service().search(&sq)?;
                deduplicate_results(
                    raw_hits.into_iter().map(|r| (r.path, r.score)),
                    options.limit,
                    options.modality,
                    &search_text,
                )
            }
            RetrievalMode::Binary => {
                // Isolated binary index search: project query and run Hamming scan across candidate pool
                let binary = engine.binary_index();
                let q_fp = binary.project_query(&search_text)?;
                let hits = binary.search_hamming(
                    &q_fp,
                    (options.limit * 50).max(500),
                    options.modality,
                )?;
                deduplicate_results(
                    hits.into_iter().map(|(id, dist)| {
                        let sim = 1.0 - (dist as f32 / 256.0);
                        (id, sim as f64)
                    }),
                    options.limit,
                    options.modality,
                    &search_text,
                )
            }
            RetrievalMode::Ppr => {
                // Isolated PPR diffusion: seed with BM25 then diffuse on Petgraph
                let sq = SearchQuery {
                    query: search_text.clone(),
                    mode: Some("bm25".to_string()),
                    limit: Some(options.limit * 5),
                    modality: options.modality,
                    ..Default::default()
                };
                let bm25_hits = engine.search_service().search(&sq)?;
                let seeds: Vec<(String, f64)> =
                    bm25_hits.into_iter().map(|r| (r.path, r.score)).collect();
                let ppr_scores = groundcontrol_core::graph::diffusion::personalized_pagerank(
                    engine.knowledge_graph(),
                    &seeds,
                    groundcontrol_core::graph::diffusion::PPR_DEFAULT_ALPHA,
                    groundcontrol_core::graph::diffusion::PPR_DEFAULT_ITERATIONS,
                    None,
                );
                deduplicate_results(
                    ppr_scores.into_iter().map(|p| (p.path, p.score)),
                    options.limit,
                    options.modality,
                    &search_text,
                )
            }
            RetrievalMode::Fast => {
                let sq = SearchQuery {
                    query: search_text.clone(),
                    mode: Some("fast".to_string()),
                    limit: Some(options.limit * 3),
                    modality: options.modality,
                    decompose: Some(options.decompose),
                    ..Default::default()
                };
                let raw_hits = engine.search_service().search(&sq)?;
                deduplicate_results(
                    raw_hits.into_iter().map(|r| (r.path, r.score)),
                    options.limit,
                    options.modality,
                    &search_text,
                )
            }
            RetrievalMode::Semantic => {
                let sq = SearchQuery {
                    query: search_text.clone(),
                    mode: Some("semantic".to_string()),
                    limit: Some(options.limit * 3),
                    modality: options.modality,
                    decompose: Some(options.decompose),
                    ..Default::default()
                };
                let raw_hits = engine.search_service().search(&sq)?;
                deduplicate_results(
                    raw_hits.into_iter().map(|r| (r.path, r.score)),
                    options.limit,
                    options.modality,
                    &search_text,
                )
            }
            RetrievalMode::Full => {
                let sq = SearchQuery {
                    query: search_text.clone(),
                    mode: Some("hybrid".to_string()),
                    limit: Some(options.limit * 3),
                    modality: options.modality,
                    decompose: Some(options.decompose),
                    ..Default::default()
                };
                let raw_hits = engine.search_service().search(&sq)?;
                deduplicate_results(
                    raw_hits.into_iter().map(|r| (r.path, r.score)),
                    options.limit,
                    options.modality,
                    &search_text,
                )
            }
        };

        let elapsed_ms = t_start.elapsed().as_secs_f64() * 1000.0;
        Ok((results, elapsed_ms))
    }
}

fn clean_path(raw: &str) -> &str {
    let mut p = raw;
    if let Some(idx) = p.find(":chunk:") {
        p = &p[..idx];
    }
    if let Some(idx) = p.find('#') {
        p = &p[..idx];
    }
    if let Some(rest) = p.strip_prefix("TinyGPT-V-main/") {
        p = rest;
    }
    p
}

fn is_file_path(p: &str) -> bool {
    p.contains('/') || p.contains('\\') || p.contains('.')
}

fn is_non_code_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let p = std::path::Path::new(&lower);
    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
        matches!(
            ext,
            "yaml" | "yml" | "json" | "toml" | "md" | "markdown" | "txt" | "spdx" | "lock" | "rst"
        )
    } else {
        false
    }
}

fn deduplicate_results(
    raw_hits: impl IntoIterator<Item = (String, f64)>,
    limit: usize,
    modality: Modality,
    query_text: &str,
) -> Vec<SearchResult> {
    use std::collections::HashMap;

    let query_terms: Vec<String> = query_text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect();

    let mut file_scores: HashMap<String, (f64, usize, f64)> = HashMap::new();

    for (id, score) in raw_hits {
        let path = clean_path(&id);
        if !is_file_path(path) {
            continue;
        }
        if modality == Modality::Code && is_non_code_path(path) {
            continue;
        }

        let path_lower = path.to_lowercase();
        let id_lower = id.to_lowercase();
        let mut lex_bonus = 0.0;
        for term in &query_terms {
            if id_lower.contains(term) || path_lower.contains(term) {
                lex_bonus += 0.005;
            }
        }

        file_scores
            .entry(path.to_string())
            .and_modify(|(max_s, count, best_lex)| {
                if score > *max_s {
                    *max_s = score;
                }
                *count += 1;
                if lex_bonus > *best_lex {
                    *best_lex = lex_bonus;
                }
            })
            .or_insert((score, 1, lex_bonus));
    }

    let mut scored_files: Vec<(String, f64)> = file_scores
        .into_iter()
        .map(|(path, (max_s, count, lex_bonus))| {
            let chunk_bonus = 0.002 * (count as f64).min(10.0);
            let total = max_s + chunk_bonus + lex_bonus;
            (path, total)
        })
        .collect();

    scored_files.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored_files.truncate(limit);

    scored_files.into_iter().map(|(path, score)| SearchResult::new(path, score)).collect()
}
