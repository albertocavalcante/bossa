//! Data types for storage module

use std::path::PathBuf;

/// Scope and completeness of the metadata-only iCloud observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Debug, serde::Serialize)]
pub struct ICloudStats {
    pub root: Option<PathBuf>,
    pub max_depth: usize,
    pub max_entries: u64,
    pub entries_seen: u64,
    pub entry_limit_reached: bool,
    pub hidden_policy: &'static str,
    pub filesystem_coverage: Coverage,
    pub icloud_state_coverage: Coverage,
    pub observed_files: u64,
    pub known_logical_bytes: Option<u64>,
    pub unknown_sizes: u64,
    pub unknown_states: u64,
    pub walk_errors: u64,
    pub metadata_errors: u64,
    pub skipped_hidden: u64,
    pub depth_limited_directories: u64,
    pub skipped_other: u64,
    pub arithmetic_overflow: bool,
}

impl ICloudStats {
    pub fn unavailable(root: Option<PathBuf>, max_depth: usize, max_entries: u64) -> Self {
        Self {
            root,
            max_depth,
            max_entries,
            entries_seen: 0,
            entry_limit_reached: false,
            hidden_policy: "excluded",
            filesystem_coverage: Coverage::Unavailable,
            icloud_state_coverage: Coverage::Unavailable,
            observed_files: 0,
            known_logical_bytes: None,
            unknown_sizes: 0,
            unknown_states: 0,
            walk_errors: 0,
            metadata_errors: 0,
            skipped_hidden: 0,
            depth_limited_directories: 0,
            skipped_other: 0,
            arithmetic_overflow: false,
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct StorageReport {
    pub schema: &'static str,
    pub local: Option<DiskSpace>,
    pub external: Option<DiskSpace>,
    pub icloud: ICloudStats,
    pub manifests: Option<Vec<ManifestInfo>>,
}

/// Manifest statistics for display
#[derive(Debug, serde::Serialize)]
pub struct ManifestInfo {
    pub name: String,
    pub file_count: u64,
    pub total_size: u64,
    pub duplicate_groups: u64,
    #[serde(rename = "duplicate_logical_bytes")]
    pub wasted_space: u64,
}

/// A manifest entry with name and path
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    pub name: String,
    pub path: PathBuf,
}

/// Disk space information
#[derive(Debug, serde::Serialize)]
pub struct DiskSpace {
    #[serde(rename = "total_bytes")]
    pub total: u64,
    #[serde(rename = "available_bytes")]
    pub available: u64,
}

impl DiskSpace {
    /// Calculate used space
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}
