//! Error types for the `groundcontrol-graphview` sidecar.

use thiserror::Error;

/// Result alias for GraphView operations.
pub type Result<T> = std::result::Result<T, GraphViewError>;

/// Errors occurring within the GraphView visualization sidecar.
#[derive(Debug, Error)]
pub enum GraphViewError {
    /// Graph file read or postcard deserialization failure.
    #[error("Graph load error: {0}")]
    GraphLoad(String),

    /// SQLite database query error.
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// I/O error reading directories or assets.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Network or HTTP request error (e.g. telemetry relay).
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    /// JSON serialization or parsing error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Layout calculation error.
    #[error("Layout error: {0}")]
    Layout(String),

    /// Requested corpus or entity not found.
    #[error("Not found: {0}")]
    NotFound(String),
}
