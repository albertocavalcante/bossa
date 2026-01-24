use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The download/sync state of an iCloud file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    /// File is fully downloaded and available locally.
    Local,
    /// File is in the cloud only (evicted/placeholder).
    Cloud,
    /// File is currently being downloaded.
    Downloading {
        /// Download progress percentage (0-100).
        percent: u8,
    },
    /// File is currently being uploaded.
    Uploading {
        /// Upload progress percentage (0-100).
        percent: u8,
    },
    /// State is unknown or file is not in iCloud.
    Unknown,
}

impl DownloadState {
    /// Returns true if the file is available locally
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    /// Returns true if the file is cloud-only (evicted)
    pub fn is_cloud_only(&self) -> bool {
        matches!(self, Self::Cloud)
    }

    /// Returns true if the file is currently syncing
    pub fn is_syncing(&self) -> bool {
        matches!(self, Self::Downloading { .. } | Self::Uploading { .. })
    }
}

/// Complete status information for an iCloud file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStatus {
    /// Path to the file
    pub path: PathBuf,
    /// Current download/sync state
    pub state: DownloadState,
    /// File size in bytes (if known)
    pub size: Option<u64>,
    /// Whether this is a directory
    pub is_dir: bool,
}

impl FileStatus {
    /// Create a new FileStatus
    pub fn new(path: PathBuf, state: DownloadState) -> Self {
        Self {
            path,
            state,
            size: None,
            is_dir: false,
        }
    }

    /// Set the file size
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// Mark as directory
    pub fn as_dir(mut self) -> Self {
        self.is_dir = true;
        self
    }
}

/// Options for eviction operations
#[derive(Debug, Clone, Default)]
pub struct EvictOptions {
    /// Recursively evict directory contents
    pub recursive: bool,
    /// Only evict files larger than this size (bytes)
    pub min_size: Option<u64>,
    /// Dry run - don't actually evict
    pub dry_run: bool,
}

/// Options for download operations
#[derive(Debug, Clone, Default)]
pub struct DownloadOptions {
    /// Recursively download directory contents
    pub recursive: bool,
    /// Dry run - don't actually download
    pub dry_run: bool,
}

/// Result of a bulk operation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BulkResult {
    /// Number of files successfully processed
    pub succeeded: usize,
    /// Number of files that failed
    pub failed: usize,
    /// Number of files skipped (already in desired state)
    pub skipped: usize,
    /// Total bytes affected
    pub bytes: u64,
    /// Paths that failed with error messages
    pub errors: Vec<(PathBuf, String)>,
}

impl BulkResult {
    /// Add a success
    pub fn add_success(&mut self, bytes: u64) {
        self.succeeded += 1;
        self.bytes += bytes;
    }

    /// Add a failure
    pub fn add_failure(&mut self, path: PathBuf, error: String) {
        self.failed += 1;
        self.errors.push((path, error));
    }

    /// Add a skip
    pub fn add_skip(&mut self) {
        self.skipped += 1;
    }

    /// Check if all operations succeeded
    pub fn is_success(&self) -> bool {
        self.failed == 0
    }

    /// Total files processed
    pub fn total(&self) -> usize {
        self.succeeded + self.failed + self.skipped
    }
}
