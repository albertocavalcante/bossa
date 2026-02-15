//! brctl backend implementation.
//!
//! Uses Apple's `brctl` CLI tool for iCloud Drive operations.
//! This is the safest and most reliable approach as it uses Apple's
//! official tooling.
//!
//! ## Safety
//!
//! `brctl evict` only removes the LOCAL copy of a file. The file remains
//! safely stored in iCloud and can be re-downloaded at any time.
//! This is NOT a delete operation.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};
use crate::types::{DownloadState, FileStatus};

use super::Backend;

/// Backend implementation using Apple's brctl CLI.
///
/// brctl is macOS's built-in tool for managing CloudDocs (iCloud Drive).
/// This backend shells out to brctl for operations.
///
/// ## Supported operations
///
/// - `brctl evict <path>` - Remove local copy, keep cloud copy
/// - `brctl download <path>` - Download cloud copy to local
///
/// ## Safety
///
/// brctl does NOT have any delete functionality. It can only:
/// - Evict (remove local, keep cloud)
/// - Download (fetch cloud to local)
///
/// There is no way to delete files from iCloud using brctl.
pub struct BrctlBackend {
    icloud_root: PathBuf,
}

impl BrctlBackend {
    /// Create a new BrctlBackend.
    ///
    /// Returns an error if iCloud Drive is not available.
    pub fn new() -> Result<Self> {
        // Verify brctl is available
        if !Self::is_available() {
            return Err(Error::BrctlNotFound);
        }

        let home = std::env::var("HOME").map_err(|_| {
            Error::ICloudNotAvailable("HOME environment variable not set".to_string())
        })?;

        let icloud_root = PathBuf::from(&home)
            .join("Library")
            .join("Mobile Documents")
            .join("com~apple~CloudDocs");

        // Verify iCloud Drive exists
        if !icloud_root.exists() {
            return Err(Error::ICloudNotAvailable(format!(
                "iCloud Drive not found at {}",
                icloud_root.display()
            )));
        }

        Ok(Self { icloud_root })
    }

    /// Check if brctl is available on this system.
    pub fn is_available() -> bool {
        Command::new("which")
            .arg("brctl")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Run a brctl command and return the output.
    fn run_brctl(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("brctl").args(args).output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::BrctlNotFound
            } else {
                Error::Io(e)
            }
        })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            // Parse common error messages for better error types
            Err(self.parse_brctl_error(&stderr))
        }
    }

    /// Parse brctl error messages into specific error types.
    fn parse_brctl_error(&self, stderr: &str) -> Error {
        if stderr.contains("cannot be evicted") {
            // File is still uploading or not ready
            Error::NotSynced(PathBuf::from("file not ready for eviction"))
        } else if stderr.contains("No such file") || stderr.contains("does not exist") {
            Error::NotFound(PathBuf::from("file not found"))
        } else if stderr.contains("Permission denied") {
            Error::PermissionDenied(PathBuf::from("permission denied"))
        } else {
            Error::BrctlFailed(stderr.to_string())
        }
    }

    /// iCloud extended attribute for download-in-progress state.
    const ICLOUD_DOWNLOAD_REQUESTED_ATTR: &'static str = "com.apple.icloud.itemDownloadRequested";

    /// Get the download state of a file using block allocation detection.
    ///
    /// This is the most reliable method for detecting iCloud file status:
    /// - Cloud-only files have size > 0 but blocks == 0 (no local data allocated)
    /// - Downloaded files have blocks > 0 (actual data on disk)
    ///
    /// This works regardless of Spotlight indexing status and doesn't require
    /// shelling out to mdls or xattr.
    ///
    /// Note: `std::fs::metadata()` succeeds on cloud-only files in iCloud Drive,
    /// correctly reporting the file's size while showing 0 allocated blocks.
    fn get_file_state(&self, path: &Path) -> Result<DownloadState> {
        use std::os::unix::fs::MetadataExt;

        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NotFound(path.to_path_buf()));
            }
            Err(e) => return Err(Error::Io(e)),
        };

        // For directories, assume local if accessible
        if metadata.is_dir() {
            return Ok(DownloadState::Local);
        }

        // For files, use block allocation to determine state
        // Cloud-only files report their size but have 0 blocks allocated
        // Downloaded files have actual blocks on disk
        if metadata.is_file() {
            let size = metadata.len();
            let blocks = metadata.blocks();

            if size > 0 && blocks == 0 {
                // File has size but no blocks allocated = cloud-only
                // Check if download is in progress
                if self.has_download_requested_attr(path) {
                    return Ok(DownloadState::Downloading { percent: 0 });
                }
                return Ok(DownloadState::Cloud);
            } else if blocks > 0 {
                // File has blocks allocated = downloaded locally
                return Ok(DownloadState::Local);
            }
            // Zero-size file - check if download was requested
            if self.has_download_requested_attr(path) {
                return Ok(DownloadState::Downloading { percent: 0 });
            }
            // Empty files are typically local
            return Ok(DownloadState::Local);
        }

        Ok(DownloadState::Unknown)
    }

    /// Check if a file has the iCloud download-requested extended attribute.
    fn has_download_requested_attr(&self, path: &Path) -> bool {
        Command::new("xattr")
            .args(["-p", Self::ICLOUD_DOWNLOAD_REQUESTED_ATTR])
            .arg(path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Canonicalize path for consistent handling.
    fn normalize_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
        }
    }
}

impl Backend for BrctlBackend {
    fn status(&self, path: &Path) -> Result<FileStatus> {
        let path = self.normalize_path(path);
        let state = self.get_file_state(&path)?;
        let metadata = std::fs::metadata(&path).ok();

        let mut status = FileStatus::new(path, state);

        if let Some(m) = metadata {
            if m.is_dir() {
                status = status.as_dir();
            } else {
                status = status.with_size(m.len());
            }
        }

        Ok(status)
    }

    fn evict(&self, path: &Path) -> Result<()> {
        let path = self.normalize_path(path);

        // Pre-flight checks
        if !self.is_in_icloud(&path) {
            return Err(Error::NotInICloud(path));
        }

        // Check current state
        let state = self.get_file_state(&path)?;
        if state == DownloadState::Cloud {
            return Err(Error::AlreadyEvicted(path));
        }

        // Note: get_file_state() doesn't currently detect Uploading state,
        // but brctl will fail with "cannot be evicted" for uploading files.
        // This check provides defense-in-depth if upload detection is added later.
        if let DownloadState::Uploading { .. } = state {
            return Err(Error::NotSynced(path));
        }

        // Convert path to string, handling special characters
        let path_str = path.to_str().ok_or_else(|| {
            Error::InvalidPath(format!("path contains invalid UTF-8: {}", path.display()))
        })?;

        // Execute eviction
        // Note: brctl evict ONLY removes the local copy
        // The file remains safely in iCloud
        self.run_brctl(&["evict", path_str])?;

        Ok(())
    }

    fn download(&self, path: &Path) -> Result<()> {
        let path = self.normalize_path(path);

        // Pre-flight checks
        if !self.is_in_icloud(&path) {
            return Err(Error::NotInICloud(path));
        }

        let path_str = path.to_str().ok_or_else(|| {
            Error::InvalidPath(format!("path contains invalid UTF-8: {}", path.display()))
        })?;

        self.run_brctl(&["download", path_str])?;

        Ok(())
    }

    fn is_in_icloud(&self, path: &Path) -> bool {
        let path = self.normalize_path(path);

        // Check if path is under iCloud Drive root
        // Also check Mobile Documents for app-specific containers
        let mobile_docs = self
            .icloud_root
            .parent()
            .unwrap_or(&self.icloud_root)
            .to_path_buf();

        path.starts_with(&self.icloud_root) || path.starts_with(&mobile_docs)
    }

    fn icloud_root(&self) -> Result<PathBuf> {
        Ok(self.icloud_root.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_in_icloud() {
        if let Ok(backend) = BrctlBackend::new() {
            let home = std::env::var("HOME").unwrap();

            // Should be in iCloud
            let icloud_path =
                format!("{home}/Library/Mobile Documents/com~apple~CloudDocs/test.txt");
            assert!(backend.is_in_icloud(Path::new(&icloud_path)));

            // App-specific container should also be in iCloud
            let app_path =
                format!("{home}/Library/Mobile Documents/iCloud~com~example~app/test.txt");
            assert!(backend.is_in_icloud(Path::new(&app_path)));

            // Should not be in iCloud
            assert!(!backend.is_in_icloud(Path::new("/tmp/test.txt")));
            assert!(!backend.is_in_icloud(Path::new(&format!("{home}/Documents/test.txt"))));
        }
    }

    #[test]
    fn test_brctl_available() {
        // This test will pass on macOS, fail on other platforms
        if cfg!(target_os = "macos") {
            assert!(BrctlBackend::is_available());
        }
    }

    #[test]
    fn test_new_returns_error_if_no_icloud() {
        // This test verifies that new() properly validates iCloud availability
        // On a system with iCloud, it should succeed
        // On a system without iCloud, it should return an error
        let result = BrctlBackend::new();
        if cfg!(target_os = "macos") {
            // On macOS, it might succeed or fail depending on iCloud setup
            // Just verify it doesn't panic
            let _ = result;
        } else {
            // On non-macOS, should fail
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_block_based_detection_logic() {
        // Test the logic of block-based detection (without actual iCloud files)
        // This verifies our understanding of how blocks relate to file state

        use std::os::unix::fs::MetadataExt;

        // RAII guard for cleanup even on panic
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }

        // Create a temporary file with actual content
        let test_file = std::env::temp_dir().join("icloud_test_block_detection.txt");
        let _cleanup = Cleanup(test_file.clone());

        std::fs::write(&test_file, "test content with actual data").unwrap();

        let metadata = std::fs::metadata(&test_file).unwrap();

        // A real file with content should have blocks > 0
        assert!(metadata.len() > 0, "file should have size");
        assert!(
            metadata.blocks() > 0,
            "real file should have blocks allocated"
        );
    }
}
