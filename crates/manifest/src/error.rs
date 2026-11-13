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
    /// An unversioned store cannot establish its root or native path identity.
    #[error("legacy unqualified manifest; preserve it and perform a fresh scan")]
    Legacy,
    /// A manifest schema is not supported by this version.
    #[error("unsupported manifest schema {0}")]
    UnsupportedSchema(i64),
    /// The store belongs to a different declared or canonical root.
    #[error("manifest root differs from the requested root")]
    RootMismatch,
    /// Historical evidence must not be used for filesystem changes.
    #[error("manifest is stale or has no completed scan; rescan required")]
    Stale,
    /// Complete traversal is required for publishing a generation.
    #[error("scan failed: {0}")]
    Scan(String),
}

/// Result type for manifest operations
pub type Result<T> = std::result::Result<T, Error>;
