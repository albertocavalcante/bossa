//! Error types for the manifest crate

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during manifest operations
#[derive(Error, Debug)]
pub enum Error {
    /// Database error
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Path does not exist
    #[error("path does not exist: {}", .0.display())]
    PathNotFound(PathBuf),

    /// Failed to hash file
    #[error("failed to hash file {}: {source}", .path.display())]
    HashFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Invalid path (e.g., cannot determine manifest name)
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

/// Result type for manifest operations
pub type Result<T> = std::result::Result<T, Error>;
