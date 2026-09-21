//! Unified error types for the groundcontrol engine.

use thiserror::Error;

/// Top-level error type for the engine.
#[derive(Debug, Error)]
pub enum Error {
    /// Configuration file could not be parsed or is invalid.
    #[error("configuration error: {0}")]
    Config(String),

    /// File system operation failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Markdown parsing failed.
    #[error("parse error in {path}: {message}")]
    Parse {
        /// File path where parsing failed.
        path: String,
        /// Description of the parse failure.
        message: String,
    },

    /// Template validation failure.
    #[error("validation error in {path}: {message}")]
    Validation {
        /// File path that failed validation.
        path: String,
        /// Description of the validation failure.
        message: String,
    },

    /// Graph operation failed.
    #[error("graph error: {0}")]
    Graph(String),

    /// Search index error.
    #[error("index error: {0}")]
    Index(String),

    /// Database error.
    #[error("database error: {0}")]
    Database(String),

    /// Note not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// Operation not permitted (e.g., write on read-only corpus).
    #[error("not permitted: {0}")]
    NotPermitted(String),
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;
