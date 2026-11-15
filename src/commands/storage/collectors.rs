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
    // Allocation and xattr heuristics cannot establish iCloud residency. Do not
    // call Client::status or xattr here; report only regular-file logical sizes.
    scan_icloud(&root, |path| {
        let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
        if !metadata.file_type().is_file() {
            return Err(());
        }
        Ok(Some(metadata.len()))
    })
}

fn scan_icloud(
    root: &Path,
    mut metadata: impl FnMut(&Path) -> std::result::Result<Option<u64>, ()>,
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
    stats.filesystem_coverage = Coverage::Complete;
    stats.known_logical_bytes = Some(0);
    let mut entries = WalkDir::new(root).max_depth(ICLOUD_MAX_DEPTH).into_iter();
    while let Some(entry) = entries.next() {
        if stats.entries_seen == stats.max_entries {
            stats.entry_limit_reached = true;
            stats.filesystem_coverage = Coverage::Partial;
            break;
        }
        stats.entries_seen += 1;
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                stats.walk_errors += 1;
                stats.filesystem_coverage = Coverage::Partial;
                continue;
            }
        };
        if entry.depth() > 0 && entry.file_name().to_string_lossy().starts_with('.') {
            stats.skipped_hidden += 1;
            stats.filesystem_coverage = Coverage::Partial;
            if entry.file_type().is_dir() {
                entries.skip_current_dir();
            }
            continue;
        }
        if entry.file_type().is_dir() {
            if entry.depth() == ICLOUD_MAX_DEPTH {
                stats.depth_limited_directories += 1;
                stats.filesystem_coverage = Coverage::Partial;
            }
            continue;
        }
        if !entry.file_type().is_file() {
            stats.skipped_other += 1;
            stats.filesystem_coverage = Coverage::Partial;
            continue;
        }
        stats.observed_files += 1;
        // Every iCloud state remains unknown, regardless of allocation or xattrs.
        stats.unknown_states += 1;
        let size = match metadata(entry.path()) {
            Ok(size) => size,
            Err(()) => {
                stats.metadata_errors += 1;
                stats.filesystem_coverage = Coverage::Partial;
                continue;
            }
        };
        let total = stats
            .known_logical_bytes
            .as_mut()
            .expect("initialized after root validation");
        match size {
            Some(size) => match total.checked_add(size) {
                Some(sum) => *total = sum,
                None => {
                    *total = u64::MAX;
                    stats.arithmetic_overflow = true;
                    stats.filesystem_coverage = Coverage::Partial;
                }
            },
            None => {
                stats.unknown_sizes += 1;
                stats.filesystem_coverage = Coverage::Partial;
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

    #[test]
    fn omissions_and_metadata_failures_are_visible() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("known"), b"hello").unwrap();
        fs::write(root.join("unknown-size"), b"x").unwrap();
        fs::write(root.join("error"), b"x").unwrap();
        fs::write(root.join(".hidden"), b"x").unwrap();
        fs::create_dir(root.join(".private")).unwrap();
        fs::write(root.join(".private/omitted"), b"x").unwrap();
        fs::create_dir_all(root.join("a/b/c/deeper")).unwrap();
        fs::write(root.join("a/b/c/deeper/omitted"), b"x").unwrap();
        let stats = scan_icloud(root, |path| {
            match path.file_name().unwrap().to_str().unwrap() {
                "known" => Ok(Some(5)),
                "unknown-size" => Ok(None),
                "error" => Err(()),
                _ => panic!("unexpected file"),
            }
        });
        assert_eq!(stats.filesystem_coverage, Coverage::Partial);
        assert_eq!(stats.icloud_state_coverage, Coverage::Unavailable);
        assert_eq!(stats.known_logical_bytes, Some(5));
        assert_eq!(
            (
                stats.unknown_sizes,
                stats.unknown_states,
                stats.metadata_errors
            ),
            (1, 3, 1)
        );
        assert_eq!(
            (stats.skipped_hidden, stats.depth_limited_directories),
            (2, 1)
        );
        let json = serde_json::to_value(&stats).unwrap();
        assert_eq!(json["filesystem_coverage"], "partial");
        assert!(json.get("known_cloud_logical_bytes").is_none());
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
        let can_read = fs::read_dir(&denied).is_ok();
        let stats = scan_icloud(dir.path(), |_| panic!("no files expected"));
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o700)).unwrap();
        if !can_read {
            assert_eq!(stats.filesystem_coverage, Coverage::Partial);
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
            Ok(Some(1))
        });
        assert_eq!(stats.filesystem_coverage, Coverage::Partial);
        assert!(stats.entry_limit_reached);
        assert_eq!(stats.entries_seen, ICLOUD_MAX_ENTRIES);
        assert_eq!(calls, ICLOUD_MAX_ENTRIES - 1);
    }

    #[test]
    fn overflowing_subtotal_is_partial() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), b"x").unwrap();
        fs::write(dir.path().join("b"), b"x").unwrap();
        let stats = scan_icloud(dir.path(), |_| Ok(Some(u64::MAX)));
        assert!(stats.arithmetic_overflow);
        assert_eq!(stats.filesystem_coverage, Coverage::Partial);
        assert_eq!(stats.known_logical_bytes, Some(u64::MAX));
    }

    #[test]
    fn empty_observed_directory_differs_from_unavailable_root() {
        let dir = tempfile::tempdir().unwrap();
        let empty = scan_icloud(dir.path(), |_| panic!("empty"));
        assert_eq!(empty.filesystem_coverage, Coverage::Complete);
        assert_eq!(empty.icloud_state_coverage, Coverage::Unavailable);
        assert_eq!(empty.known_logical_bytes, Some(0));
        let absent = scan_icloud(&dir.path().join("absent"), |_| panic!("absent"));
        assert_eq!(absent.filesystem_coverage, Coverage::Unavailable);
        assert_eq!(absent.known_logical_bytes, None);
        assert!(serde_json::to_value(absent).unwrap()["known_logical_bytes"].is_null());
    }

    // Run in a child process so the simulated PATH and HOME do not race other tests.
    #[cfg(unix)]
    #[test]
    fn unavailable_xattr_cannot_imply_cloud_or_local() {
        use std::os::unix::fs::PermissionsExt;
        if std::env::var_os("BOSSA_STORAGE_XATTR_TEST_CHILD").is_some() {
            let stats = collect_icloud_stats();
            assert_eq!(stats.filesystem_coverage, Coverage::Complete);
            assert_eq!(stats.icloud_state_coverage, Coverage::Unavailable);
            assert_eq!(stats.unknown_states, 1);
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let root = dir
            .path()
            .join("home/Library/Mobile Documents/com~apple~CloudDocs");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&root).unwrap();
        let sparse = fs::File::create(root.join("sparse")).unwrap();
        sparse.set_len(1024 * 1024).unwrap();
        fs::write(bin.join("which"), b"#!/bin/sh\nexit 0\n").unwrap();
        // If invoked, this simulates failed xattr lookup and leaves a marker.
        fs::write(
            bin.join("xattr"),
            b"#!/bin/sh\n: > \"$HOME/xattr-was-called\"\nexit 1\n",
        )
        .unwrap();
        for name in ["which", "xattr"] {
            fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let marker = dir.path().join("home/xattr-was-called");
        let control = std::process::Command::new(bin.join("xattr"))
            .env("HOME", dir.path().join("home"))
            .status()
            .unwrap();
        assert!(!control.success());
        assert!(marker.exists());
        fs::remove_file(&marker).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "commands::storage::collectors::coverage_tests::unavailable_xattr_cannot_imply_cloud_or_local"])
            .env_clear().env("HOME", dir.path().join("home")).env("PATH", &bin)
            .env("BOSSA_STORAGE_XATTR_TEST_CHILD", "1").status().unwrap();
        assert!(status.success());
        assert!(!dir.path().join("home/xattr-was-called").exists());
    }

    #[test]
    fn sparse_file_is_never_classified_as_cloud_or_local() {
        let dir = tempfile::tempdir().unwrap();
        let file = fs::File::create(dir.path().join("sparse")).unwrap();
        file.set_len(1024 * 1024).unwrap();
        let stats = scan_icloud(dir.path(), |path| {
            fs::symlink_metadata(path)
                .map(|m| Some(m.len()))
                .map_err(|_| ())
        });
        assert_eq!(stats.filesystem_coverage, Coverage::Complete);
        assert_eq!(stats.icloud_state_coverage, Coverage::Unavailable);
        assert_eq!(stats.unknown_states, 1);
        assert_eq!(stats.known_logical_bytes, Some(1024 * 1024));
        let json = serde_json::to_value(&stats).unwrap();
        assert!(json.get("observed_cloud_files").is_none());
        assert!(json.get("observed_local_files").is_none());
    }
}
