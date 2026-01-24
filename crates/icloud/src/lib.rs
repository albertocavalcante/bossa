//! # icloud
//!
//! A Rust library for managing iCloud Drive files on macOS.
//!
//! ## Safety Guarantees
//!
//! **This crate NEVER deletes files from iCloud.** All operations are non-destructive:
//!
//! - **Evict**: Removes only the LOCAL copy. The file remains safely stored in iCloud
//!   and can be re-downloaded at any time (shows download arrow in Finder).
//! - **Download**: Fetches a cloud-only file to local storage. No data is modified.
//! - **Status**: Read-only operation to check file state.
//!
//! There are intentionally NO delete, remove, or destructive operations in this crate.
//!
//! ## What is "eviction"?
//!
//! When you evict a file:
//! 1. The local copy is removed from your Mac's disk
//! 2. The file remains stored in iCloud (cloud copy is untouched)
//! 3. A placeholder appears in Finder with a download icon
//! 4. The file will be re-downloaded automatically when you open it
//! 5. You free up local disk space while keeping the file accessible
//!
//! ## Requirements
//!
//! - macOS 10.15 or later
//! - "Optimize Mac Storage" should be enabled in iCloud settings for best results
//! - File must be fully synced to iCloud before it can be evicted
//!
//! ## Example
//!
//! ```no_run
//! use icloud::Client;
//!
//! let client = Client::new().expect("Failed to create client");
//!
//! // Check if a file is downloaded locally
//! let status = client.status("~/Library/Mobile Documents/com~apple~CloudDocs/myfile.txt")
//!     .expect("Failed to get status");
//!
//! if status.state.is_local() {
//!     // Evict to free local space (file stays in iCloud!)
//!     client.evict(&status.path).expect("Failed to evict");
//! }
//! ```
//!
//! ## Backends
//!
//! The crate supports multiple backends:
//! - `brctl` (default): Uses Apple's brctl CLI tool (safe, well-tested)
//! - `native` (future): Direct FFI to NSFileManager
//!
//! ## Platform Support
//!
//! This crate only works on macOS, as iCloud Drive is a macOS/iOS feature.
//!
//! ## References
//!
//! - [brctl man page](https://keith.github.io/xcode-man-pages/brctl.1.html)
//! - [Apple FileManager.evictUbiquitousItem](https://developer.apple.com/documentation/foundation/filemanager/1409696-evictubiquitousitem)

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all)]

/// Backend implementations for iCloud operations.
pub mod backend;
/// Error types for iCloud operations.
pub mod error;
/// Common types for iCloud file status and operations.
pub mod types;

pub use error::{Error, Result};
pub use types::{BulkResult, DownloadOptions, DownloadState, EvictOptions, FileStatus};

use backend::Backend;
use std::path::{Path, PathBuf};

/// High-level client for iCloud Drive operations.
///
/// # Safety
///
/// This client provides ONLY non-destructive operations:
/// - `evict()` removes local copy, keeps cloud copy
/// - `download()` fetches cloud copy to local
/// - `status()` reads file state (read-only)
///
/// There are NO delete operations. Files are never removed from iCloud.
pub struct Client {
    backend: Box<dyn Backend>,
}

impl Client {
    /// Create a new Client with the default backend.
    ///
    /// Returns an error if not running on macOS or if iCloud Drive is not available.
    #[cfg(feature = "brctl")]
    pub fn new() -> Result<Self> {
        let backend = backend::brctl::BrctlBackend::new()?;
        Ok(Self {
            backend: Box::new(backend),
        })
    }

    /// Create a client with a custom backend (useful for testing).
    pub fn with_backend(backend: Box<dyn Backend>) -> Self {
        Self { backend }
    }

    /// Get the iCloud Drive root path.
    ///
    /// Typically `~/Library/Mobile Documents/com~apple~CloudDocs/`
    pub fn icloud_root(&self) -> Result<PathBuf> {
        self.backend.icloud_root()
    }

    /// Check if a path is within iCloud Drive.
    ///
    /// Returns `true` if the path is under the iCloud Drive root directory.
    pub fn is_in_icloud(&self, path: impl AsRef<Path>) -> bool {
        let path = expand_path(path.as_ref());
        self.backend.is_in_icloud(&path)
    }

    /// Get the status of a file.
    ///
    /// This is a read-only operation that checks whether a file is:
    /// - Downloaded locally (`DownloadState::Local`)
    /// - Cloud-only/evicted (`DownloadState::Cloud`)
    /// - Currently syncing (`DownloadState::Downloading` or `Uploading`)
    pub fn status(&self, path: impl AsRef<Path>) -> Result<FileStatus> {
        let path = expand_and_validate_path(path.as_ref())?;
        self.backend.status(&path)
    }

    /// Evict a file - remove LOCAL copy, keep CLOUD copy.
    ///
    /// # What this does
    ///
    /// - Removes the file from your local disk to free space
    /// - The file REMAINS in iCloud (this is NOT a delete!)
    /// - A placeholder appears in Finder with a download icon
    /// - Opening the file will automatically re-download it
    ///
    /// # What this does NOT do
    ///
    /// - Does NOT delete the file from iCloud
    /// - Does NOT make the file inaccessible
    /// - Does NOT affect other devices synced to the same iCloud account
    ///
    /// # Requirements
    ///
    /// - File must be in iCloud Drive
    /// - File must be fully synced (upload complete)
    /// - "Optimize Mac Storage" should be enabled
    ///
    /// # Errors
    ///
    /// - `NotInICloud`: Path is not in iCloud Drive
    /// - `NotSynced`: File hasn't finished uploading to iCloud yet
    /// - `AlreadyEvicted`: File is already cloud-only
    pub fn evict(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = expand_and_validate_path(path.as_ref())?;

        // Safety check: must be in iCloud
        if !self.backend.is_in_icloud(&path) {
            return Err(Error::NotInICloud(path));
        }

        self.backend.evict(&path)
    }

    /// Download a file from iCloud to local storage.
    ///
    /// # What this does
    ///
    /// - Fetches the cloud copy to your local disk
    /// - Makes the file available offline
    /// - If already downloaded, this is a no-op
    ///
    /// # Errors
    ///
    /// - `NotInICloud`: Path is not in iCloud Drive
    /// - `NotFound`: File doesn't exist
    pub fn download(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = expand_and_validate_path(path.as_ref())?;

        // Safety check: must be in iCloud
        if !self.backend.is_in_icloud(&path) {
            return Err(Error::NotInICloud(path));
        }

        self.backend.download(&path)
    }

    /// Evict multiple files with options.
    ///
    /// See [`evict`](Self::evict) for details on what eviction does.
    pub fn evict_bulk(
        &self,
        paths: &[impl AsRef<Path>],
        options: &EvictOptions,
    ) -> Result<BulkResult> {
        let paths: Vec<PathBuf> = paths
            .iter()
            .filter_map(|p| expand_and_validate_path(p.as_ref()).ok())
            .collect();
        let path_refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();
        self.backend.evict_bulk(&path_refs, options)
    }

    /// Download multiple files with options.
    ///
    /// See [`download`](Self::download) for details.
    pub fn download_bulk(
        &self,
        paths: &[impl AsRef<Path>],
        options: &DownloadOptions,
    ) -> Result<BulkResult> {
        let paths: Vec<PathBuf> = paths
            .iter()
            .filter_map(|p| expand_and_validate_path(p.as_ref()).ok())
            .collect();
        let path_refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();
        self.backend.download_bulk(&path_refs, options)
    }

    /// List files in an iCloud Drive directory with their status.
    ///
    /// Returns a list of files and their download states.
    /// Skips hidden AppleDouble files (._*).
    pub fn list(&self, path: impl AsRef<Path>) -> Result<Vec<FileStatus>> {
        let path = expand_and_validate_path(path.as_ref())?;

        if !self.backend.is_in_icloud(&path) {
            return Err(Error::NotInICloud(path));
        }

        let mut results = Vec::new();

        let entries = std::fs::read_dir(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::NotFound(path.clone())
            } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                Error::PermissionDenied(path.clone())
            } else {
                Error::Io(e)
            }
        })?;

        for entry in entries.flatten() {
            let entry_path = entry.path();

            // Skip hidden AppleDouble metadata files
            if let Some(name) = entry_path.file_name() {
                let name_str = name.to_string_lossy();
                if name_str.starts_with("._") || name_str.starts_with('.') {
                    continue;
                }
            }

            if let Ok(status) = self.status(&entry_path) {
                results.push(status);
            }
        }

        Ok(results)
    }

    /// Find large local files that could be evicted to free space.
    ///
    /// Returns files larger than `min_size` bytes that are currently downloaded locally.
    /// These files can be safely evicted to free disk space while keeping them in iCloud.
    pub fn find_evictable(&self, path: impl AsRef<Path>, min_size: u64) -> Result<Vec<FileStatus>> {
        let files = self.list(path)?;
        Ok(files
            .into_iter()
            .filter(|f| {
                f.state.is_local() && !f.is_dir && f.size.map(|s| s >= min_size).unwrap_or(false)
            })
            .collect())
    }

    /// Calculate total size of local files that could be evicted.
    ///
    /// Returns the total bytes that could be freed by evicting all local files
    /// larger than `min_size`.
    pub fn evictable_size(&self, path: impl AsRef<Path>, min_size: u64) -> Result<u64> {
        let evictable = self.find_evictable(path, min_size)?;
        Ok(evictable.iter().filter_map(|f| f.size).sum())
    }
}

/// Expand ~ in paths and return the expanded path.
fn expand_path(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(&path_str[2..]);
        }
    }
    path.to_path_buf()
}

/// Expand path and validate it's reasonable.
fn expand_and_validate_path(path: &Path) -> Result<PathBuf> {
    let path_str = path.to_string_lossy();

    // Check for empty path
    if path_str.is_empty() {
        return Err(Error::InvalidPath("empty path".to_string()));
    }

    // Expand ~ if present
    let expanded = expand_path(path);

    // Ensure path is absolute after expansion
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(Error::Io)?
            .join(&expanded)
    };

    Ok(absolute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_path() {
        let home = std::env::var("HOME").unwrap();

        let expanded = expand_path(Path::new("~/test.txt"));
        assert_eq!(expanded, PathBuf::from(format!("{}/test.txt", home)));

        let absolute = expand_path(Path::new("/tmp/test.txt"));
        assert_eq!(absolute, PathBuf::from("/tmp/test.txt"));

        let relative = expand_path(Path::new("relative/path.txt"));
        assert_eq!(relative, PathBuf::from("relative/path.txt"));
    }

    #[test]
    fn test_expand_and_validate_path() {
        // Empty path should fail
        assert!(expand_and_validate_path(Path::new("")).is_err());

        // Valid paths should succeed
        assert!(expand_and_validate_path(Path::new("/tmp/test.txt")).is_ok());
        assert!(expand_and_validate_path(Path::new("~/test.txt")).is_ok());
    }
}
