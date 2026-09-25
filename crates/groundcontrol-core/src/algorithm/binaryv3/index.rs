//! In-memory and persisted storage for 256-bit binaryv3 fingerprints.
//!
//! Implements hardware POPCOUNT linear Hamming scan (Stage 1) followed by
//! O(1) post-Hamming Bayesian structural prior rescoring (Stage 2) in sub-millisecond latency.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use groundcontrol_common::types::{BinaryFingerprint, Modality};
use groundcontrol_common::{Error, Result};
use serde::{Deserialize, Serialize};

use super::projector::BinaryV3Projector;
use super::types::{BinaryV3Config, FingerprintV3Record};

/// Schema version for binaryv3 fingerprints persistence file (`fingerprints_v3.bin`).
pub const FINGERPRINTS_V3_SCHEMA_VERSION: u32 = 3;

#[derive(Serialize, Deserialize)]
struct FingerprintsV3Data {
    version: u32,
    records: Vec<FingerprintV3Record>,
}

/// Concrete high-throughput in-memory binaryv3 search index.
#[derive(Clone)]
pub struct BinaryV3SearchIndex {
    records: Vec<FingerprintV3Record>,
    projector: Arc<BinaryV3Projector>,
    config: BinaryV3Config,
}

impl std::fmt::Debug for BinaryV3SearchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BinaryV3SearchIndex")
            .field("records_count", &self.records.len())
            .field("config", &self.config)
            .finish()
    }
}

impl Default for BinaryV3SearchIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl BinaryV3SearchIndex {
    /// Create an empty binaryv3 search index.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            projector: Arc::new(BinaryV3Projector::default()),
            config: BinaryV3Config::default(),
        }
    }

    /// Load the binaryv3 index from a disk file.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).map_err(Error::Io)?;
        let data: FingerprintsV3Data = postcard::from_bytes(&bytes)
            .map_err(|e| Error::Index(format!("failed to deserialize fingerprints_v3.bin: {e}")))?;

        if data.version != FINGERPRINTS_V3_SCHEMA_VERSION {
            return Err(Error::Index(format!(
                "mismatched fingerprints_v3 schema version: expected {}, got {}",
                FINGERPRINTS_V3_SCHEMA_VERSION, data.version
            )));
        }

        Ok(Self {
            records: data.records,
            projector: Arc::new(BinaryV3Projector::default()),
            config: BinaryV3Config::default(),
        })
    }

    /// Persist the binaryv3 index to disk via postcard.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = FingerprintsV3Data {
            version: FINGERPRINTS_V3_SCHEMA_VERSION,
            records: self.records.clone(),
        };
        let bytes = postcard::to_allocvec(&data)
            .map_err(|e| Error::Index(format!("failed to serialize fingerprints_v3.bin: {e}")))?;
        fs::write(path, bytes)?;
        Ok(())
    }

    /// Access the active binaryv3 projector handle.
    pub fn projector(&self) -> &BinaryV3Projector {
        &self.projector
    }

    /// Access active configuration.
    pub fn config(&self) -> &BinaryV3Config {
        &self.config
    }

    /// Access mutable configuration.
    pub fn config_mut(&mut self) -> &mut BinaryV3Config {
        &mut self.config
    }

    /// Set runtime configuration.
    pub fn set_config(&mut self, config: BinaryV3Config) {
        self.config = config;
    }

    /// Project a text query into a 256-bit fingerprint.
    pub fn project_query(&self, query: &str) -> BinaryFingerprint {
        self.projector.project_query(query)
    }

    /// Get all indexed fingerprint records.
    pub fn records(&self) -> &[FingerprintV3Record] {
        &self.records
    }

    /// Check whether the index contains no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Get the count of indexed records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Clear all fingerprint records.
    pub fn clear(&mut self) {
        self.records.clear();
    }

    /// Remove all fingerprints associated with a document path.
    pub fn remove_document(&mut self, doc_path: &str) {
        let chunk_prefix = format!("{doc_path}:chunk:");
        let symbol_prefix = format!("{doc_path}#");
        self.records.retain(|r| {
            r.id != doc_path
                && !r.id.starts_with(&chunk_prefix)
                && !r.id.starts_with(&symbol_prefix)
        });
    }

    /// Add or update binary fingerprint records.
    pub fn index_fingerprints(&mut self, new_records: &[FingerprintV3Record]) -> Result<()> {
        let mut index_map: std::collections::HashMap<String, usize> =
            self.records.iter().enumerate().map(|(idx, r)| (r.id.clone(), idx)).collect();

        for rec in new_records {
            if let Some(&existing_idx) = index_map.get(&rec.id) {
                self.records[existing_idx] = rec.clone();
            } else {
                index_map.insert(rec.id.clone(), self.records.len());
                self.records.push(rec.clone());
            }
        }
        Ok(())
    }

    /// Search candidates using two-stage pipeline:
    /// 1. Stage 1: Hardware POPCOUNT Hamming scan + optional Word 0 early-exit.
    /// 2. Stage 2: Bayesian prior multiplier rescoring on candidate pool.
    ///
    /// Returns `(entity_id, hamming_distance, final_rescored_score)`.
    /// Search candidates using two-stage pipeline with default configuration.
    pub fn search_candidates(
        &self,
        query_bits: &BinaryFingerprint,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<(String, u32, f64)>> {
        self.search_candidates_with_opts(
            query_bits,
            limit,
            modality,
            self.config.apply_prior,
            self.config.early_exit,
        )
    }

    /// Search candidates using two-stage pipeline with explicit prior and early-exit options:
    /// 1. Stage 1: Hardware POPCOUNT Hamming scan + optional Word 0 early-exit.
    /// 2. Stage 2: Bayesian prior multiplier rescoring on candidate pool.
    ///
    /// Returns `(entity_id, hamming_distance, final_rescored_score)`.
    pub fn search_candidates_with_opts(
        &self,
        query_bits: &BinaryFingerprint,
        limit: usize,
        modality: Modality,
        apply_prior: bool,
        early_exit: bool,
    ) -> Result<Vec<(String, u32, f64)>> {
        if self.records.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        // Stage 1: Hardware Hamming scan
        let mut matches: Vec<(&FingerprintV3Record, u32)> = Vec::with_capacity(self.records.len());

        for r in &self.records {
            match modality {
                Modality::Both => {}
                Modality::Docs => {
                    if r.modality != Modality::Docs {
                        continue;
                    }
                }
                Modality::Code => {
                    if r.modality != Modality::Code {
                        continue;
                    }
                }
            }

            // Word 0 Matryoshka early-exit filter
            if early_exit {
                let w0_dist = (query_bits.0[0] ^ r.fingerprint.0[0]).count_ones();
                if w0_dist > self.config.early_exit_threshold {
                    continue;
                }
            }

            let dist = r.fingerprint.hamming_distance(query_bits);
            matches.push((r, dist));
        }

        // If early-exit was too aggressive, fallback to unpruned pool to safeguard recall
        if early_exit && matches.len() < limit {
            matches.clear();
            for r in &self.records {
                match modality {
                    Modality::Both => {}
                    Modality::Docs => {
                        if r.modality != Modality::Docs {
                            continue;
                        }
                    }
                    Modality::Code => {
                        if r.modality != Modality::Code {
                            continue;
                        }
                    }
                }
                let dist = r.fingerprint.hamming_distance(query_bits);
                matches.push((r, dist));
            }
        }

        if matches.is_empty() {
            return Ok(Vec::new());
        }

        // Partition candidate pool (Top M, default M=50)
        let candidate_pool_size = if apply_prior {
            (limit * 2).max(self.config.candidate_pool).min(matches.len())
        } else {
            limit.min(matches.len())
        };

        if candidate_pool_size < matches.len() {
            matches.select_nth_unstable_by(candidate_pool_size - 1, |a, b| a.1.cmp(&b.1));
            matches.truncate(candidate_pool_size);
        }

        // Stage 2: Bayesian prior rescoring
        let mut rescored: Vec<(&FingerprintV3Record, u32, f64)> = matches
            .into_iter()
            .map(|(r, dist)| {
                let sim = 1.0 - (dist as f32 / 256.0);
                let score = if apply_prior {
                    (sim * r.flags.compute_multiplier()) as f64
                } else {
                    sim as f64
                };
                (r, dist, score)
            })
            .collect();

        // Sort by final rescored score descending
        rescored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        rescored.truncate(limit);

        Ok(rescored.into_iter().map(|(r, dist, score)| (r.id.clone(), dist, score)).collect())
    }

    /// Search for nearest fingerprints by raw Hamming distance.
    pub fn search_hamming(
        &self,
        query_bits: &BinaryFingerprint,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<(String, u32)>> {
        let candidates = self.search_candidates(query_bits, limit, modality)?;
        Ok(candidates.into_iter().map(|(id, dist, _)| (id, dist)).collect())
    }
}
