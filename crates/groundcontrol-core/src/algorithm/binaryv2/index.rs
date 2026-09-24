//! In-memory and persisted storage for 256-bit binaryv2 fingerprints.
//!
//! Provides sub-millisecond linear Hamming scans across multi-channel fingerprints
//! with single-cycle POPCOUNT matching, powered by unsupervised RRI co-occurrence statistics.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use groundcontrol_common::types::{BinaryFingerprint, FingerprintRecord, Modality};
use groundcontrol_common::{Error, Result};
use serde::{Deserialize, Serialize};

use super::projector::BinaryV2Projector;
use super::rri::RriEngine;

/// Schema version for binaryv2 fingerprints persistence file (`fingerprints_v2.bin`).
pub const FINGERPRINTS_V2_SCHEMA_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct FingerprintsV2Data {
    version: u32,
    records: Vec<FingerprintRecord>,
    #[serde(default)]
    rri: RriEngine,
}

/// Concrete high-throughput in-memory binaryv2 search index.
#[derive(Clone)]
pub struct BinaryV2SearchIndex {
    records: Vec<FingerprintRecord>,
    projector: Arc<BinaryV2Projector>,
    rri: RriEngine,
}

impl std::fmt::Debug for BinaryV2SearchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BinaryV2SearchIndex")
            .field("records_count", &self.records.len())
            .field("rri_vocab_size", &self.rri.doc_freqs.len())
            .finish()
    }
}

impl Default for BinaryV2SearchIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl BinaryV2SearchIndex {
    /// Create an empty binaryv2 search index.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            projector: Arc::new(BinaryV2Projector::new()),
            rri: RriEngine::new(),
        }
    }

    /// Load the binaryv2 index from a disk file.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).map_err(Error::Io)?;
        let data: FingerprintsV2Data = postcard::from_bytes(&bytes)
            .map_err(|e| Error::Index(format!("failed to deserialize fingerprints_v2.bin: {e}")))?;

        if data.version != FINGERPRINTS_V2_SCHEMA_VERSION {
            return Err(Error::Index(format!(
                "mismatched fingerprints_v2 schema version: expected {}, got {}",
                FINGERPRINTS_V2_SCHEMA_VERSION, data.version
            )));
        }

        Ok(Self {
            records: data.records,
            projector: Arc::new(BinaryV2Projector::new()),
            rri: data.rri,
        })
    }

    /// Persist the binaryv2 index to disk via postcard.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = FingerprintsV2Data {
            version: FINGERPRINTS_V2_SCHEMA_VERSION,
            records: self.records.clone(),
            rri: self.rri.clone(),
        };
        let bytes = postcard::to_allocvec(&data)
            .map_err(|e| Error::Index(format!("failed to serialize fingerprints_v2.bin: {e}")))?;
        fs::write(path, bytes)?;
        Ok(())
    }

    /// Access the active binaryv2 projector handle.
    pub fn projector(&self) -> &BinaryV2Projector {
        &self.projector
    }

    /// Access the learned RRI co-occurrence engine.
    pub fn rri(&self) -> &RriEngine {
        &self.rri
    }

    /// Mutable access to the RRI engine for training.
    pub fn rri_mut(&mut self) -> &mut RriEngine {
        &mut self.rri
    }

    /// Project a text query into a 256-bit fingerprint.
    pub fn project_query(&self, query: &str) -> BinaryFingerprint {
        self.projector.project_query(query, &self.rri)
    }

    /// Get all indexed fingerprint records.
    pub fn records(&self) -> &[FingerprintRecord] {
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

    /// Clear all fingerprint records and learned RRI statistics.
    pub fn clear(&mut self) {
        self.records.clear();
        self.rri.clear();
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
    pub fn index_fingerprints(&mut self, new_records: &[FingerprintRecord]) -> Result<()> {
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

    /// Search for nearest fingerprints by full 256-bit Hamming distance.
    pub fn search_hamming(
        &self,
        query_bits: &BinaryFingerprint,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<(String, u32)>> {
        if self.records.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let mut matches: Vec<(&FingerprintRecord, u32)> = self
            .records
            .iter()
            .filter(|r| match modality {
                Modality::Both => true,
                Modality::Docs => r.modality == Modality::Docs,
                Modality::Code => r.modality == Modality::Code,
            })
            .map(|r| (r, r.fingerprint.hamming_distance(query_bits)))
            .collect();

        if matches.is_empty() {
            return Ok(Vec::new());
        }

        let k = limit.min(matches.len());
        matches.select_nth_unstable_by(k - 1, |a, b| a.1.cmp(&b.1));
        matches.truncate(k);
        matches.sort_by_key(|m| m.1);

        Ok(matches.into_iter().map(|(r, dist)| (r.id.clone(), dist)).collect())
    }
}
