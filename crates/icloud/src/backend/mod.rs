use std::path::Path;

use crate::error::Result;
use crate::types::{BulkResult, DownloadOptions, DownloadState, EvictOptions, FileStatus};

#[cfg(feature = "brctl")]
pub mod brctl;

/// Backend trait for iCloud operations
///
/// This trait abstracts the underlying implementation, allowing us to:
/// - Start with brctl (shell out to Apple's CLI)
/// - Later add native FFI via objc crate
/// - Mock for testing
pub trait Backend: Send + Sync {
    /// Get the download/sync status of a file
    fn status(&self, path: &Path) -> Result<FileStatus>;

    /// Evict a file (remove local copy, keep in cloud)
    fn evict(&self, path: &Path) -> Result<()>;

    /// Download a file (fetch from cloud to local)
    fn download(&self, path: &Path) -> Result<()>;

    /// Check if a path is within iCloud Drive
    fn is_in_icloud(&self, path: &Path) -> bool;

    /// Get the iCloud Drive root path
    fn icloud_root(&self) -> Result<std::path::PathBuf>;

    /// Evict multiple files with options
    fn evict_bulk(&self, paths: &[&Path], options: &EvictOptions) -> Result<BulkResult> {
        let mut result = BulkResult::default();

        for path in paths {
            if options.dry_run {
                result.add_success(0);
                continue;
            }

            match self.status(path) {
                Ok(status) => {
                    if status.state == DownloadState::Cloud {
                        result.add_skip();
                        continue;
                    }

                    if let Some(min_size) = options.min_size {
                        if let Some(size) = status.size {
                            if size < min_size {
                                result.add_skip();
                                continue;
                            }
                        }
                    }

                    match self.evict(path) {
                        Ok(()) => result.add_success(status.size.unwrap_or(0)),
                        Err(e) => result.add_failure(path.to_path_buf(), e.to_string()),
                    }
                }
                Err(e) => result.add_failure(path.to_path_buf(), e.to_string()),
            }
        }

        Ok(result)
    }

    /// Download multiple files with options
    fn download_bulk(&self, paths: &[&Path], options: &DownloadOptions) -> Result<BulkResult> {
        let mut result = BulkResult::default();

        for path in paths {
            if options.dry_run {
                result.add_success(0);
                continue;
            }

            match self.status(path) {
                Ok(status) => {
                    if status.state == DownloadState::Local {
                        result.add_skip();
                        continue;
                    }

                    match self.download(path) {
                        Ok(()) => result.add_success(status.size.unwrap_or(0)),
                        Err(e) => result.add_failure(path.to_path_buf(), e.to_string()),
                    }
                }
                Err(e) => result.add_failure(path.to_path_buf(), e.to_string()),
            }
        }

        Ok(result)
    }
}

/// Get the default backend based on enabled features.
///
/// Returns an error if iCloud Drive is not available.
#[cfg(feature = "brctl")]
pub fn default_backend() -> Result<brctl::BrctlBackend> {
    brctl::BrctlBackend::new()
}
