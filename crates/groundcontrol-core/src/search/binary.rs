//! 256-Bit Matryoshka Binary Index & SIMD Hamming Scan.
//!
//! Provides sub-millisecond linear Hamming scans across 500,000+ binary fingerprints
//! using 1-bit sign quantization and hardware POPCOUNT (`count_ones()`).

use std::fs;
use std::path::Path;
use std::sync::Arc;

use groundcontrol_common::ports::AlgorithmicSearchIndex;
use groundcontrol_common::types::{BinaryFingerprint, BinaryProjectionKind, FingerprintRecord, Modality};
use groundcontrol_common::{Error, Result};
use serde::{Deserialize, Serialize};

use super::hyperplanes::PartitionedHyperplaneProjector;
use super::sif::SifEngine;

/// Schema version for binary fingerprints persistence file (`fingerprints.bin`).
pub const FINGERPRINTS_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct FingerprintsData {
    version: u32,
    records: Vec<FingerprintRecord>,
}

/// Concrete high-throughput in-memory binary search index satisfying `AlgorithmicSearchIndex`.
#[derive(Debug, Clone)]
pub struct BinarySearchIndex {
    records: Vec<FingerprintRecord>,
    sif: Arc<SifEngine>,
    hyperplanes: Arc<PartitionedHyperplaneProjector>,
    projection_kind: BinaryProjectionKind,
}

impl Default for BinarySearchIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl BinarySearchIndex {
    /// Create an empty binary search index.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            sif: Arc::new(SifEngine::default()),
            hyperplanes: Arc::new(PartitionedHyperplaneProjector::default()),
            projection_kind: BinaryProjectionKind::default(),
        }
    }

    /// Load the binary index from a disk file.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).map_err(|e| Error::Io(e))?;
        let data: FingerprintsData = postcard::from_bytes(&bytes)
            .map_err(|e| Error::Index(format!("failed to deserialize fingerprints.bin: {e}")))?;

        if data.version != FINGERPRINTS_SCHEMA_VERSION {
            return Err(Error::Index(format!(
                "mismatched fingerprints schema version: expected {}, got {}",
                FINGERPRINTS_SCHEMA_VERSION, data.version
            )));
        }

        Ok(Self {
            records: data.records,
            sif: Arc::new(SifEngine::default()),
            hyperplanes: Arc::new(PartitionedHyperplaneProjector::default()),
            projection_kind: BinaryProjectionKind::default(),
        })
    }

    /// Persist the binary index to disk via postcard.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = FingerprintsData {
            version: FINGERPRINTS_SCHEMA_VERSION,
            records: self.records.clone(),
        };
        let bytes = postcard::to_allocvec(&data)
            .map_err(|e| Error::Index(format!("failed to serialize fingerprints.bin: {e}")))?;
        fs::write(path, bytes)?;
        Ok(())
    }

    /// Access the underlying shared SIF engine handle.
    pub fn sif(&self) -> Arc<SifEngine> {
        Arc::clone(&self.sif)
    }

    /// Access the underlying SIF engine reference for observation or direct projection.
    pub fn sif_engine(&self) -> &SifEngine {
        &self.sif
    }

    /// Access the underlying SIF engine for observation or fine-tuning.
    pub fn sif_mut(&mut self) -> &mut SifEngine {
        Arc::make_mut(&mut self.sif)
    }

    /// Get the current binary projection kind.
    pub fn projection_kind(&self) -> BinaryProjectionKind {
        self.projection_kind
    }

    /// Set the binary projection kind (e.g. for ablation between FlatSif and PartitionedHyperplane).
    pub fn set_projection_kind(&mut self, kind: BinaryProjectionKind) {
        self.projection_kind = kind;
    }

    /// Builder method to set the binary projection kind.
    pub fn with_projection_kind(mut self, kind: BinaryProjectionKind) -> Self {
        self.projection_kind = kind;
        self
    }

    /// Project a text query using a specific projection kind.
    pub fn project_query_with_kind(&self, query: &str, kind: BinaryProjectionKind) -> Result<BinaryFingerprint> {
        match kind {
            BinaryProjectionKind::FlatSif => Ok(self.sif.project_to_fingerprint(query)),
            BinaryProjectionKind::PartitionedHyperplane => Ok(self.hyperplanes.project_query(query)),
        }
    }

    /// Get all indexed fingerprint records.
    pub fn records(&self) -> &[FingerprintRecord] {
        &self.records
    }

    /// Check whether the index contains no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Add or update binary fingerprint records.
    pub fn index_fingerprints(&mut self, new_records: &[FingerprintRecord]) -> Result<()> {
        <Self as AlgorithmicSearchIndex>::index_fingerprints(self, new_records)
    }

    /// Search for nearest fingerprints by Hamming distance.
    pub fn search_hamming(
        &self,
        query_bits: &BinaryFingerprint,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<(String, u32)>> {
        <Self as AlgorithmicSearchIndex>::search_hamming(self, query_bits, limit, modality)
    }

    /// Access the underlying partitioned hyperplane projector.
    pub fn hyperplanes(&self) -> Arc<PartitionedHyperplaneProjector> {
        Arc::clone(&self.hyperplanes)
    }

    /// Project extracted AST grammar semantics into a 256-bit partitioned binary fingerprint.
    pub fn project_semantics(
        &self,
        sem: &crate::parser::code::grammar::ExtractedGrammarSemantics,
    ) -> BinaryFingerprint {
        self.hyperplanes.project_semantics(sem)
    }

    /// Project a text query into a 256-bit binary fingerprint.
    pub fn project_query(&self, text: &str) -> Result<BinaryFingerprint> {
        <Self as AlgorithmicSearchIndex>::project_query(self, text)
    }
}

impl AlgorithmicSearchIndex for BinarySearchIndex {
    fn index_fingerprints(&mut self, new_records: &[FingerprintRecord]) -> Result<()> {
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

    fn search_hamming(
        &self,
        query_bits: &BinaryFingerprint,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<(String, u32)>> {
        if self.records.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        // Detect text-query fingerprints: project_query() zeros Ch2 and Ch3 (dataflow and
        // grammar channels) because those feature vocabularies are structurally incompatible
        // with plain text. When Ch2/Ch3 are both zero we apply channel masking so that
        // document bits in those channels don't contribute noise to the ranking distance.
        let is_text_query = query_bits.0[2] == 0 && query_bits.0[3] == 0;
        let channel_mask = if is_text_query {
            [true, true, false, false]
        } else {
            [true, true, true, true]
        };

        // Candidates matching the modality filter
        let mut matches: Vec<(&FingerprintRecord, u32)> = self
            .records
            .iter()
            .filter(|r| match modality {
                Modality::Both => true,
                Modality::Docs => r.modality == Modality::Docs,
                Modality::Code => r.modality == Modality::Code,
            })
            .map(|r| (r, r.fingerprint.masked_hamming_distance(query_bits, channel_mask)))
            .collect();

        if matches.is_empty() {
            return Ok(Vec::new());
        }

        // Partial sort for top-K with smallest Hamming distance
        let k = limit.min(matches.len());
        matches.select_nth_unstable_by(k - 1, |a, b| a.1.cmp(&b.1));
        matches.truncate(k);
        matches.sort_by_key(|m| m.1);

        Ok(matches.into_iter().map(|(r, dist)| (r.id.clone(), dist)).collect())
    }

    fn project_query(&self, query: &str) -> Result<BinaryFingerprint> {
        self.project_query_with_kind(query, self.projection_kind)
    }

    fn clear(&mut self) {
        self.records.clear();
    }

    fn len(&self) -> usize {
        self.records.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_index_search() {
        let mut index = BinarySearchIndex::new();
        let fp1 = index.project_query("authentication token jwt validator").unwrap();
        let fp2 = index.project_query("http router endpoint controller").unwrap();
        let fp3 = index.project_query("database postgres sql storage flush").unwrap();

        index
            .index_fingerprints(&[
                FingerprintRecord {
                    id: "src/auth.rs".into(),
                    fingerprint: fp1,
                    modality: Modality::Code,
                },
                FingerprintRecord {
                    id: "src/api.rs".into(),
                    fingerprint: fp2,
                    modality: Modality::Code,
                },
                FingerprintRecord {
                    id: "src/db.rs".into(),
                    fingerprint: fp3,
                    modality: Modality::Code,
                },
            ])
            .unwrap();

        let query_fp = index.project_query("jwt auth login").unwrap();
        let hits = index.search_hamming(&query_fp, 2, Modality::Code).unwrap();

        assert!(!hits.is_empty());
        assert_eq!(hits[0].0, "src/auth.rs");
    }
}
