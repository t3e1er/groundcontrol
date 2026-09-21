//! Disk footprint profiler for measuring source corpus size and index directory breakdown.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Detailed breakdown of on-disk storage for a corpus and its `.index/` directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskBreakdown {
    /// Number of source files in corpus.
    pub source_file_count: usize,
    /// Total source bytes on disk.
    pub source_bytes: u64,
    /// Size of SQLite metadata catalog (`meta.db`) in bytes.
    pub meta_db_bytes: u64,
    /// Total size of Tantivy inverted index directory in bytes.
    pub tantivy_bytes: u64,
    /// Size of 256-bit MRL binary fingerprints file (`fingerprints.bin`) in bytes.
    pub fingerprints_bytes: u64,
    /// Size of serialized knowledge graph (`graph.bin`) in bytes.
    pub graph_bytes: u64,
    /// Size of dense vectors file (`vectors.bin` / `vectors.json`) in bytes.
    pub vectors_bytes: u64,
    /// Size of derived text projections directory (`projections/`) in bytes.
    pub projections_bytes: u64,
    /// Total bytes occupied by `.index/` directory.
    pub total_index_bytes: u64,
    /// Index expansion ratio: `total_index_bytes / source_bytes`.
    pub expansion_ratio: f64,
}

impl DiskBreakdown {
    /// Compute human-readable megabytes for source.
    pub fn source_mb(&self) -> f64 {
        self.source_bytes as f64 / (1024.0 * 1024.0)
    }

    /// Compute human-readable megabytes for index.
    pub fn index_mb(&self) -> f64 {
        self.total_index_bytes as f64 / (1024.0 * 1024.0)
    }
}

/// Disk profiler utility.
pub struct DiskProfiler;

impl DiskProfiler {
    /// Profile the disk footprint of a corpus directory and its `.index/` directory.
    pub fn profile(corpus_dir: &Path) -> std::io::Result<DiskBreakdown> {
        let index_dir = corpus_dir.join(".index");

        let mut source_file_count = 0;
        let mut source_bytes = 0;

        // Traverse corpus excluding .index and .git
        if corpus_dir.is_dir() {
            for entry in fs::read_dir(corpus_dir)? {
                let entry = entry?;
                let path = entry.path();
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();
                if name_str == ".index" || name_str == ".git" {
                    continue;
                }
                let (files, bytes) = Self::dir_stats(&path)?;
                source_file_count += files;
                source_bytes += bytes;
            }
        }

        let meta_db_bytes = Self::file_size(&index_dir.join("meta.db"));
        let tantivy_bytes = Self::dir_size(&index_dir.join("tantivy"));
        let fingerprints_bytes = Self::file_size(&index_dir.join("fingerprints.bin"));
        let graph_bytes = Self::file_size(&index_dir.join("graph.bin"));
        let vectors_bytes = Self::file_size(&index_dir.join("vectors.bin"))
            + Self::file_size(&index_dir.join("vectors.json"));
        let projections_bytes = Self::dir_size(&index_dir.join("projections"));
        let total_index_bytes = Self::dir_size(&index_dir);

        let expansion_ratio =
            if source_bytes > 0 { total_index_bytes as f64 / source_bytes as f64 } else { 0.0 };

        Ok(DiskBreakdown {
            source_file_count,
            source_bytes,
            meta_db_bytes,
            tantivy_bytes,
            fingerprints_bytes,
            graph_bytes,
            vectors_bytes,
            projections_bytes,
            total_index_bytes,
            expansion_ratio,
        })
    }

    fn file_size(path: &Path) -> u64 {
        fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    fn dir_size(path: &Path) -> u64 {
        if !path.exists() {
            return 0;
        }
        if path.is_file() {
            return Self::file_size(path);
        }
        let mut total = 0;
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    total += Self::dir_size(&p);
                } else if let Ok(m) = entry.metadata() {
                    total += m.len();
                }
            }
        }
        total
    }

    fn dir_stats(path: &Path) -> std::io::Result<(usize, u64)> {
        if !path.exists() {
            return Ok((0, 0));
        }
        if path.is_file() {
            let len = fs::metadata(path)?.len();
            return Ok((1, len));
        }
        let mut count = 0;
        let mut bytes = 0;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == ".index" || name_str == ".git" {
                continue;
            }
            if p.is_dir() {
                let (sub_count, sub_bytes) = Self::dir_stats(&p)?;
                count += sub_count;
                bytes += sub_bytes;
            } else if let Ok(m) = entry.metadata() {
                count += 1;
                bytes += m.len();
            }
        }
        Ok((count, bytes))
    }
}
