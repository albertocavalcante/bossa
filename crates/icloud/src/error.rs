use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during iCloud operations.
///
/// Note: This crate NEVER deletes files from iCloud. All errors relate to
/// non-destructive operations (evict, download, status).
#[derive(Debug, Error)]
pub enum Error {
    /// File or directory not found
    #[error("path not found: {0}")]
    NotFound(PathBuf),

    /// Path is not in iCloud Drive
    #[error("path is not in iCloud Drive: {0}")]
    NotInICloud(PathBuf),

    /// Invalid path provided
    #[error("invalid path: {0}")]
    InvalidPath(String),

    /// File is not downloaded (cannot evict what's not local)
    #[error("file is not downloaded locally: {0}")]
    NotDownloaded(PathBuf),

    /// File is already evicted (cloud-only)
    #[error("file is already evicted (cloud-only): {0}")]
    AlreadyEvicted(PathBuf),

    /// File hasn't finished syncing to iCloud yet
    #[error("file is not yet synced to iCloud (upload in progress): {0}")]
    NotSynced(PathBuf),

    /// File is currently syncing (downloading or uploading)
    #[error("file is currently syncing: {0}")]
    Syncing(PathBuf),

    /// iCloud Drive is not available or not configured
    #[error("iCloud Drive is not available: {0}")]
    ICloudNotAvailable(String),

    /// brctl command failed
    #[error("brctl command failed: {0}")]
    BrctlFailed(String),

    /// brctl not found (not on macOS?)
    #[error("brctl not found - this crate requires macOS")]
    BrctlNotFound,

    /// Permission denied
    #[error("permission denied: {0}")]
    PermissionDenied(PathBuf),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Other error
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Returns true if this error indicates the file isn't ready for eviction
    /// (still syncing, not in iCloud, etc.)
    pub fn is_not_ready(&self) -> bool {
        matches!(
            self,
            Error::NotSynced(_) | Error::Syncing(_) | Error::NotInICloud(_)
        )
    }

    /// Returns true if this error is recoverable by waiting/retrying
    pub fn is_transient(&self) -> bool {
        matches!(self, Error::Syncing(_) | Error::NotSynced(_))
    }

    /// Returns true if the operation was a no-op (file already in desired state)
    pub fn is_already_done(&self) -> bool {
        matches!(self, Error::AlreadyEvicted(_))
    }
}

/// Result type for iCloud operations
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_classification() {
        let syncing = Error::Syncing(PathBuf::from("/test"));
        assert!(syncing.is_transient());
        assert!(syncing.is_not_ready());

        let evicted = Error::AlreadyEvicted(PathBuf::from("/test"));
        assert!(evicted.is_already_done());
        assert!(!evicted.is_transient());

        let not_found = Error::NotFound(PathBuf::from("/test"));
        assert!(!not_found.is_transient());
        assert!(!not_found.is_already_done());
    }
}
