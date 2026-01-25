//! Data collection for storage statistics

use anyhow::Result;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

use crate::config;

use super::types::{ICloudStats, ManifestEntry, ManifestInfo};

// ============================================================================
// Constants
// ============================================================================

/// Minimum file size to consider for eviction (10 MB)
pub const ICLOUD_MIN_EVICTABLE_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum depth for iCloud directory walk (performance limit)
const ICLOUD_MAX_DEPTH: usize = 3;

// ============================================================================
// iCloud Collection
// ============================================================================

/// Collect all iCloud stats in a single directory walk
///
/// Returns `None` if iCloud is not available or cannot be accessed.
/// Errors during file iteration are silently skipped.
pub fn collect_icloud_stats() -> Option<ICloudStats> {
    let client = icloud::Client::new().ok()?;
    let root = client.icloud_root().ok()?;

    let mut stats = ICloudStats::default();

    for entry in WalkDir::new(&root)
        .max_depth(ICLOUD_MAX_DEPTH)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        // Skip hidden files
        if is_hidden_file(&entry) {
            continue;
        }

        if let Ok(status) = client.status(entry.path()) {
            let size = status.size.unwrap_or(0);
            if status.state.is_local() {
                stats.local_bytes += size;
                stats.local_count += 1;
                // Track evictable (local files above threshold)
                if size >= ICLOUD_MIN_EVICTABLE_SIZE {
                    stats.evictable_bytes += size;
                    stats.evictable_count += 1;
                }
            } else if status.state.is_cloud_only() {
                stats.cloud_bytes += size;
                stats.cloud_count += 1;
            }
        }
    }

    Some(stats)
}

/// Check if a walkdir entry is a hidden file (starts with '.')
fn is_hidden_file(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}

// ============================================================================
// Manifest Collection
// ============================================================================

/// Collect manifest statistics for all manifests in the config directory
pub fn collect_manifest_stats() -> Result<Vec<ManifestInfo>> {
    let manifest_dir = config::config_dir()?.join("manifests");

    if !manifest_dir.exists() {
        return Ok(Vec::new());
    }

    let mut manifests = Vec::new();

    if let Ok(entries) = fs::read_dir(&manifest_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if is_manifest_db(&path)
                && let Some(info) = load_manifest_info(&path)
            {
                manifests.push(info);
            }
        }
    }

    Ok(manifests)
}

/// Collect manifest entries (name + path) from a directory
pub fn collect_manifest_entries(manifest_dir: &Path) -> Result<Vec<ManifestEntry>> {
    let mut manifests: Vec<ManifestEntry> = fs::read_dir(manifest_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if is_manifest_db(&path) {
                let name = get_manifest_name(&path);
                Some(ManifestEntry { name, path })
            } else {
                None
            }
        })
        .collect();

    manifests.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(manifests)
}

/// Load detailed manifest info from a database file
fn load_manifest_info(db_path: &Path) -> Option<ManifestInfo> {
    let manifest = manifest::Manifest::open(db_path).ok()?;
    let stats = manifest.stats().ok()?;

    Some(ManifestInfo {
        name: get_manifest_name(db_path),
        file_count: stats.file_count,
        total_size: stats.total_size,
        duplicate_groups: stats.duplicates.duplicate_groups,
        wasted_space: stats.duplicates.wasted_space,
    })
}

/// Check if a path is a manifest database file (.db extension)
fn is_manifest_db(path: &Path) -> bool {
    path.extension().map(|e| e == "db").unwrap_or(false)
}

/// Extract manifest name from database path
fn get_manifest_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}
