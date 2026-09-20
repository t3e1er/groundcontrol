//! Indexing domain types.

use serde::{Deserialize, Serialize};

/// Status of an indexing operation for a corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexingStatus {
    /// No indexing job in progress.
    Idle,
    /// Actively indexing files in batches.
    Indexing,
    /// Indexing is paused.
    Paused,
    /// Indexing encountered an error.
    Error,
    /// All discovered files successfully indexed.
    Completed,
}

impl std::fmt::Display for IndexingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Indexing => write!(f, "indexing"),
            Self::Paused => write!(f, "paused"),
            Self::Error => write!(f, "error"),
            Self::Completed => write!(f, "completed"),
        }
    }
}

impl std::str::FromStr for IndexingStatus {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "idle" => Ok(Self::Idle),
            "indexing" => Ok(Self::Indexing),
            "paused" => Ok(Self::Paused),
            "error" => Ok(Self::Error),
            "completed" => Ok(Self::Completed),
            _ => Ok(Self::Idle),
        }
    }
}

/// State tracking record for resumable paginated indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingState {
    /// Corpus identifier/name.
    pub corpus_id: String,
    /// Current indexing status.
    pub status: IndexingStatus,
    /// Total markdown files discovered in corpus.
    pub total_files: usize,
    /// Count of markdown files successfully committed.
    pub indexed_files: usize,
    /// Relative path of the last committed file.
    pub last_processed_path: Option<String>,
    /// When indexing started (Unix timestamp seconds).
    pub started_at: i64,
    /// When state was last updated (Unix timestamp seconds).
    pub updated_at: i64,
    /// Error message if status is Error.
    pub error_message: Option<String>,
}
