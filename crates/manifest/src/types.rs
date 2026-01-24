//! Data types for the manifest crate

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A group of duplicate files sharing the same content hash
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    /// BLAKE3 hash of the file content
    pub hash: String,
    /// Paths to all files with this hash (relative to scan root)
    pub paths: Vec<String>,
    /// Size of each file in bytes
    pub size_each: u64,
    /// Number of copies
    pub count: usize,
}

impl DuplicateGroup {
    /// Calculate total wasted space (all copies except one)
    pub fn wasted_space(&self) -> u64 {
        self.size_each * (self.count as u64 - 1)
    }
}

/// Statistics about duplicate files in a manifest
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DuplicateStats {
    /// Total number of files that have duplicates
    pub duplicate_files: u64,
    /// Number of unique content hashes with multiple files
    pub duplicate_groups: u64,
    /// Total wasted space from duplicates (sum of all copies except one per hash)
    pub wasted_space: u64,
}

/// Statistics about a manifest
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestStats {
    /// Total number of files in the manifest
    pub file_count: u64,
    /// Total size of all files in bytes
    pub total_size: u64,
    /// Duplicate statistics
    pub duplicates: DuplicateStats,
}

impl ManifestStats {
    /// Calculate potential savings percentage
    pub fn savings_percentage(&self) -> f64 {
        if self.total_size == 0 {
            0.0
        } else {
            (self.duplicates.wasted_space as f64 / self.total_size as f64) * 100.0
        }
    }
}

/// Result of a scan operation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanResult {
    /// Number of files successfully hashed
    pub hashed: u64,
    /// Number of files that failed to hash
    pub errors: u64,
    /// Number of stale entries pruned from the manifest
    pub pruned: u64,
    /// Duplicate statistics after scanning
    pub duplicates: DuplicateStats,
}

/// Progress information during a scan
#[derive(Debug, Clone)]
pub struct ScanProgress {
    /// Total files to scan
    pub total_files: u64,
    /// Total size to scan in bytes
    pub total_size: u64,
    /// Files scanned so far
    pub files_scanned: u64,
    /// Current file being scanned (relative path)
    pub current_file: Option<PathBuf>,
}

/// Callback trait for scan progress updates
pub trait ProgressCallback: Send {
    /// Called when starting the scan with totals
    fn on_start(&mut self, total_files: u64, total_size: u64);

    /// Called when starting to scan a file
    fn on_file(&mut self, path: &std::path::Path, size: u64);

    /// Called when a file is complete (success or failure)
    fn on_file_complete(&mut self, success: bool);

    /// Called when the scan is complete
    fn on_complete(&mut self, result: &ScanResult);
}

/// A no-op progress callback for when progress isn't needed
pub struct NoProgress;

impl ProgressCallback for NoProgress {
    fn on_start(&mut self, _total_files: u64, _total_size: u64) {}
    fn on_file(&mut self, _path: &std::path::Path, _size: u64) {}
    fn on_file_complete(&mut self, _success: bool) {}
    fn on_complete(&mut self, _result: &ScanResult) {}
}
