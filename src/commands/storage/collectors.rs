//! Data collection for storage statistics

use anyhow::Result;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

use crate::config;

use super::types::{Coverage, ICloudStats, ManifestEntry, ManifestInfo};

// ============================================================================
// Constants
// ============================================================================

/// Maximum depth for iCloud metadata walk; depth-limited directories are partial.
const ICLOUD_MAX_DEPTH: usize = 3;
/// Count root, files, directories and traversal errors against the same budget.
const ICLOUD_MAX_ENTRIES: u64 = 1000;

/// Collect only metadata. A missing root or failed traversal is never a known zero.
pub fn collect_icloud_stats() -> ICloudStats {
    let Ok(client) = icloud::Client::new() else {
        return ICloudStats::unavailable(None, ICLOUD_MAX_DEPTH, ICLOUD_MAX_ENTRIES);
    };
    let Ok(root) = client.icloud_root() else {
        return ICloudStats::unavailable(None, ICLOUD_MAX_DEPTH, ICLOUD_MAX_ENTRIES);
    };
    scan_icloud(&root, |path| {
        client
            .status(path)
            .map(|s| (s.state, s.size))
            .map_err(|_| ())
    })
}

fn scan_icloud(
    root: &Path,
    mut status: impl FnMut(&Path) -> std::result::Result<(icloud::DownloadState, Option<u64>), ()>,
) -> ICloudStats {
    let mut stats = ICloudStats::unavailable(
        Some(root.to_path_buf()),
        ICLOUD_MAX_DEPTH,
        ICLOUD_MAX_ENTRIES,
    );
    if !fs::symlink_metadata(root).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return stats;
    }
    if fs::read_dir(root).is_err() {
        return stats;
    }
    stats.coverage = Coverage::Complete;
    stats.known_local_logical_bytes = Some(0);
    stats.known_cloud_logical_bytes = Some(0);
    let mut entries = WalkDir::new(root).max_depth(ICLOUD_MAX_DEPTH).into_iter();
    while let Some(entry) = entries.next() {
        if stats.entries_seen == stats.max_entries {
            stats.entry_limit_reached = true;
            stats.coverage = Coverage::Partial;
            break;
        }
        stats.entries_seen += 1;
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                stats.walk_errors += 1;
                stats.coverage = Coverage::Partial;
                continue;
            }
        };
        if entry.depth() > 0 && entry.file_name().to_string_lossy().starts_with('.') {
            stats.skipped_hidden += 1;
            stats.coverage = Coverage::Partial;
            if entry.file_type().is_dir() {
                entries.skip_current_dir();
            }
            continue;
        }
        if entry.file_type().is_dir() {
            if entry.depth() == ICLOUD_MAX_DEPTH {
                stats.depth_limited_directories += 1;
                stats.coverage = Coverage::Partial;
            }
            continue;
        }
        if !entry.file_type().is_file() {
            stats.skipped_other += 1;
            stats.coverage = Coverage::Partial;
            continue;
        }
        let (state, size) = match status(entry.path()) {
            Ok(status) => status,
            Err(()) => {
                stats.status_errors += 1;
                stats.coverage = Coverage::Partial;
                continue;
            }
        };
        let total = if state.is_local() {
            stats.observed_local_files += 1;
            stats
                .known_local_logical_bytes
                .as_mut()
                .expect("initialized after root validation")
        } else if state.is_cloud_only() {
            stats.observed_cloud_files += 1;
            stats
                .known_cloud_logical_bytes
                .as_mut()
                .expect("initialized after root validation")
        } else {
            stats.unknown_states += 1;
            stats.coverage = Coverage::Partial;
            if size.is_none() {
                stats.unknown_sizes += 1;
            }
            continue;
        };
        match size {
            Some(size) => match total.checked_add(size) {
                Some(sum) => *total = sum,
                None => {
                    *total = u64::MAX;
                    stats.arithmetic_overflow = true;
                    stats.coverage = Coverage::Partial;
                }
            },
            None => {
                stats.unknown_sizes += 1;
                stats.coverage = Coverage::Partial;
            }
        }
    }
    stats
}

// ============================================================================
// Manifest Collection
// ============================================================================

/// Collect manifest statistics for all manifests in the config directory
pub fn collect_manifest_stats() -> Result<Vec<ManifestInfo>> {
    let manifest_dir = config::config_dir()?.join("manifests");

    let entries = match fs::read_dir(&manifest_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut manifests = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if is_manifest_db(&path) {
            manifests.push(load_manifest_info(&path)?);
        }
    }

    Ok(manifests)
}

/// Collect manifest entries (name + path) from a directory
pub fn collect_manifest_entries(manifest_dir: &Path) -> Result<Vec<ManifestEntry>> {
    let mut manifests = Vec::new();
    for entry in fs::read_dir(manifest_dir)? {
        let path = entry?.path();
        if is_manifest_db(&path) {
            manifests.push(ManifestEntry {
                name: get_manifest_name(&path),
                path,
            });
        }
    }

    manifests.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(manifests)
}

/// Load detailed manifest info from a database file
fn load_manifest_info(db_path: &Path) -> Result<ManifestInfo> {
    let manifest = manifest::Manifest::open(db_path)?;
    let stats = manifest.stats()?;
    let name = get_manifest_name(db_path);
    let name = if manifest.is_legacy() {
        format!("{name} (legacy unqualified)")
    } else {
        name
    };
    Ok(ManifestInfo {
        name,
        file_count: stats.file_count,
        total_size: stats.total_size,
        duplicate_groups: stats.duplicates.duplicate_groups,
        wasted_space: stats.duplicates.wasted_space,
    })
}

/// Check if a path is a manifest database file (.db extension)
fn is_manifest_db(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "db")
}

/// Extract manifest name from database path
fn get_manifest_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .chars()
        .flat_map(char::escape_default)
        .collect()
}

#[cfg(test)]
mod coverage_tests {
    use super::*;
    use icloud::DownloadState;

    #[test]
    fn incomplete_walk_is_visible_and_unknown_sizes_are_not_counted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("local"), b"local").unwrap();
        fs::write(root.join("cloud"), b"cloud").unwrap();
        fs::write(root.join("unknown-size"), b"x").unwrap();
        fs::write(root.join("error"), b"x").unwrap();
        fs::write(root.join("unknown-state"), b"x").unwrap();
        fs::write(root.join(".hidden"), b"x").unwrap();
        fs::create_dir(root.join(".private")).unwrap();
        fs::write(root.join(".private/omitted"), b"x").unwrap();
        fs::create_dir_all(root.join("a/b/c/deeper")).unwrap();
        fs::write(root.join("a/b/c/deeper/omitted"), b"x").unwrap();
        let stats = scan_icloud(root, |path| {
            match path.file_name().unwrap().to_str().unwrap() {
                "local" => Ok((DownloadState::Local, Some(5))),
                "cloud" => Ok((DownloadState::Cloud, Some(7))),
                "unknown-size" => Ok((DownloadState::Local, None)),
                "unknown-state" => Ok((DownloadState::Unknown, Some(10))),
                "error" => Err(()),
                _ => panic!("unexpected file"),
            }
        });
        assert_eq!(stats.coverage, Coverage::Partial);
        assert_eq!(
            (
                stats.known_local_logical_bytes,
                stats.known_cloud_logical_bytes
            ),
            (Some(5), Some(7))
        );
        assert_eq!(
            (
                stats.unknown_sizes,
                stats.unknown_states,
                stats.status_errors
            ),
            (1, 1, 1)
        );
        assert_eq!(
            (stats.skipped_hidden, stats.depth_limited_directories),
            (2, 1)
        );
        let json = serde_json::to_value(&stats).unwrap();
        assert_eq!(json["coverage"], "partial");
        assert!(json.get("evictable_bytes").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_subtree_reports_a_walk_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let denied = dir.path().join("denied");
        fs::create_dir(&denied).unwrap();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o0)).unwrap();
        // Privileged test environments may still read mode-000 directories.
        let can_read = fs::read_dir(&denied).is_ok();
        let stats = scan_icloud(dir.path(), |_| panic!("no files expected"));
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o700)).unwrap();
        if !can_read {
            assert_eq!(stats.coverage, Coverage::Partial);
            assert!(stats.walk_errors > 0);
        }
    }

    #[test]
    fn entry_budget_stops_and_marks_coverage_partial() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..ICLOUD_MAX_ENTRIES {
            fs::write(dir.path().join(format!("{i:04}")), b"x").unwrap();
        }
        let mut calls = 0;
        let stats = scan_icloud(dir.path(), |_| {
            calls += 1;
            Ok((DownloadState::Local, Some(1)))
        });
        assert_eq!(stats.coverage, Coverage::Partial);
        assert!(stats.entry_limit_reached);
        assert_eq!(stats.entries_seen, ICLOUD_MAX_ENTRIES);
        assert_eq!(calls, ICLOUD_MAX_ENTRIES - 1); // root consumes one entry
    }

    #[test]
    fn overflowing_subtotal_is_partial() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), b"x").unwrap();
        fs::write(dir.path().join("b"), b"x").unwrap();
        let stats = scan_icloud(dir.path(), |_| Ok((DownloadState::Local, Some(u64::MAX))));
        assert!(stats.arithmetic_overflow);
        assert_eq!(stats.coverage, Coverage::Partial);
        assert_eq!(stats.known_local_logical_bytes, Some(u64::MAX));
    }

    #[test]
    fn empty_observed_directory_differs_from_unavailable_root() {
        let dir = tempfile::tempdir().unwrap();
        let empty = scan_icloud(dir.path(), |_| panic!("empty"));
        assert_eq!(empty.coverage, Coverage::Complete);
        assert_eq!(empty.known_local_logical_bytes, Some(0));
        let absent = scan_icloud(&dir.path().join("absent"), |_| panic!("absent"));
        assert_eq!(absent.coverage, Coverage::Unavailable);
        assert_eq!(absent.known_local_logical_bytes, None);
        assert!(serde_json::to_value(absent).unwrap()["known_local_logical_bytes"].is_null());
    }
}
