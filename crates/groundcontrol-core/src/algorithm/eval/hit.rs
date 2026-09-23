//! Evaluation hit types, path normalization, deduplication, and candidate scoring.

use groundcontrol_common::types::Modality;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A ranked candidate result from an algorithmic retrieval query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlgoHit {
    /// 1-based rank position.
    pub rank: usize,
    /// Path to candidate file.
    pub path: String,
    /// Relevance score.
    pub score: f64,
    /// Optional symbol identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

/// Strip chunk identifiers and anchors from candidate IDs.
pub fn clean_path(raw: &str) -> &str {
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

/// Check if an ID represents a file path rather than a pure symbol or entity.
pub fn is_file_path(p: &str) -> bool {
    p.contains('/') || p.contains('\\') || p.contains('.')
}

/// Check if a path points to documentation or metadata rather than source code.
pub fn is_non_code_path(path: &str) -> bool {
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

/// Deduplicate raw hits into ranked file-level hits with frequency and lexical bonuses.
pub fn deduplicate_hits(
    raw_hits: impl IntoIterator<Item = (String, f64, Option<String>)>,
    limit: usize,
    modality: Modality,
    query_text: &str,
) -> Vec<AlgoHit> {
    let query_terms: Vec<String> = query_text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect();

    let mut file_scores: HashMap<String, (f64, usize, f64, Option<String>)> = HashMap::new();

    for (id, score, symbol) in raw_hits {
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
            .and_modify(|(max_s, count, best_lex, best_sym)| {
                if score > *max_s {
                    *max_s = score;
                    if symbol.is_some() {
                        *best_sym = symbol.clone();
                    }
                }
                *count += 1;
                if lex_bonus > *best_lex {
                    *best_lex = lex_bonus;
                }
            })
            .or_insert((score, 1, lex_bonus, symbol));
    }

    let mut scored_files: Vec<(String, f64, Option<String>)> = file_scores
        .into_iter()
        .map(|(path, (max_s, count, lex_bonus, symbol))| {
            let chunk_bonus = 0.002 * (count as f64).min(10.0);
            let total = max_s + chunk_bonus + lex_bonus;
            (path, total, symbol)
        })
        .collect();

    scored_files.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored_files.truncate(limit);

    scored_files
        .into_iter()
        .enumerate()
        .map(|(i, (path, score, symbol))| AlgoHit { rank: i + 1, path, score, symbol })
        .collect()
}
