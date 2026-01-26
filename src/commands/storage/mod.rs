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
use display::{show_hints, show_icloud, show_local_ssd, show_manifests, show_t9};

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
/// - Optimization hints
pub fn status() -> Result<()> {
    ui::header("Storage Overview");

    // Local SSD
    show_local_ssd()?;

    // iCloud Drive - collect all stats in one pass
    let icloud_stats = collect_icloud_stats();
    show_icloud(&icloud_stats);

    // T9 External
    show_t9()?;

    // Scanned manifests
    let manifests = collect_manifest_stats()?;
    show_manifests(&manifests);

    // Optimization hints
    show_hints(&icloud_stats, &manifests);

    Ok(())
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
