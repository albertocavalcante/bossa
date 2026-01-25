//! Data types for storage module

use std::path::PathBuf;

/// iCloud statistics collected in a single pass
#[derive(Debug, Default)]
pub struct ICloudStats {
    pub local_bytes: u64,
    pub cloud_bytes: u64,
    pub local_count: usize,
    pub cloud_count: usize,
    pub evictable_bytes: u64,
    pub evictable_count: usize,
}

/// Manifest statistics for display
#[derive(Debug)]
pub struct ManifestInfo {
    pub name: String,
    pub file_count: u64,
    pub total_size: u64,
    pub duplicate_groups: u64,
    pub wasted_space: u64,
}

/// A manifest entry with name and path
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    pub name: String,
    pub path: PathBuf,
}

/// Disk space information
#[derive(Debug)]
pub struct DiskSpace {
    pub total: u64,
    pub available: u64,
}

impl DiskSpace {
    /// Calculate used space
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}
