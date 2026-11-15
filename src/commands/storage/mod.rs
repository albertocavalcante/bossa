//! Unified storage overview and cross-storage duplicate detection
//!
//! This module provides:
//! - `status()` - Unified view of all storage (SSD, iCloud, T9, manifests)
//! - `duplicates()` - Find files that exist across multiple storage locations
//!
//! # Architecture
//!
//! The module is split into focused submodules:
//! - `types` - Data structures (ICloudStats, ManifestInfo, etc.)
//! - `disk` - Platform-specific disk space utilities
//! - `collectors` - Data collection functions
//! - `display` - UI/presentation functions
//! - `duplicates` - Cross-storage duplicate detection

mod collectors;
mod disk;
mod display;
mod duplicates;
mod types;

use anyhow::Result;

use crate::ui;

use collectors::{collect_icloud_stats, collect_manifest_stats};
use display::{show_icloud, show_local_ssd, show_manifests, show_t9};

// ============================================================================
// Public API
// ============================================================================

/// Display unified storage overview
///
/// Shows:
/// - Local SSD space usage
/// - iCloud Drive statistics (local/cloud/evictable)
/// - T9 external drive status
/// - Scanned manifest statistics
pub fn status(json: bool) -> Result<()> {
    let report = types::StorageReport {
        schema: "bossa.storage-status/v1",
        local: disk::get_disk_space("/").ok(),
        external: external_disk(display::T9_MOUNT),
        icloud: collect_icloud_stats(),
        manifests: collect_manifest_stats().ok(),
    };
    if json {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }
    ui::header("Storage Overview");
    show_local_ssd(&report.local);
    show_icloud(&report.icloud);
    show_t9(&report.external);
    if let Some(manifests) = &report.manifests {
        show_manifests(manifests);
    } else {
        ui::dim("Manifest statistics unavailable");
    }
    Ok(())
}

// A stale ordinary directory under /Volumes must not be reported as a mounted disk.
#[cfg(unix)]
fn external_disk(path: &str) -> Option<types::DiskSpace> {
    use std::os::unix::fs::MetadataExt;
    let mount = std::path::Path::new(path);
    let parent = mount.parent()?;
    let metadata = std::fs::symlink_metadata(mount).ok()?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.dev() == std::fs::metadata(parent).ok()?.dev()
    {
        return None;
    }
    disk::get_disk_space(path).ok()
}

#[cfg(not(unix))]
fn external_disk(_path: &str) -> Option<types::DiskSpace> {
    None
}

/// Find duplicates across scanned manifests
///
/// # Arguments
/// * `filter` - Manifest names to compare. If empty, compares all.
/// * `list_only` - If true, just list available manifests and exit.
/// * `min_size` - Minimum file size to consider.
/// * `display_limit` - Maximum duplicates to show per comparison (0 = unlimited).
pub fn duplicates(
    filter: &[String],
    list_only: bool,
    min_size: u64,
    display_limit: usize,
) -> Result<()> {
    duplicates::run(filter, list_only, min_size, display_limit)
}

#[cfg(test)]
mod status_tests {
    use crate::cli::{Cli, Command, StorageCommand};
    use clap::Parser;

    #[cfg(unix)]
    #[test]
    fn ordinary_directory_is_not_a_mounted_external_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-mount");
        std::fs::create_dir(&path).unwrap();
        assert!(super::external_disk(path.to_str().unwrap()).is_none());
    }

    #[test]
    fn status_json_is_a_distinct_cli_option() {
        let parsed = Cli::try_parse_from(["bossa", "storage", "status", "--json"]).unwrap();
        assert!(matches!(
            parsed.command,
            Command::Storage(StorageCommand::Status { json: true })
        ));
    }
}
