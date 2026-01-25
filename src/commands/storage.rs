//! Unified storage overview command
//!
//! Provides a single view of all storage: Local SSD, iCloud Drive, external drives,
//! and scanned manifests with duplicate statistics.

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::Path;

use crate::config;
use crate::ui;

// ============================================================================
// Constants
// ============================================================================

const T9_MOUNT: &str = "/Volumes/T9";
const ICLOUD_MIN_EVICTABLE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
const HINT_MIN_SIZE: u64 = 100 * 1024 * 1024; // 100 MB - minimum to show in hints
const ICLOUD_MAX_DEPTH: usize = 3; // Limit iCloud walk depth for performance

// ============================================================================
// Collected Data Types
// ============================================================================

/// iCloud statistics collected in a single pass
struct ICloudStats {
    local_bytes: u64,
    cloud_bytes: u64,
    local_count: usize,
    cloud_count: usize,
    evictable_bytes: u64,
    evictable_count: usize,
}

/// Manifest statistics
struct ManifestInfo {
    name: String,
    file_count: u64,
    total_size: u64,
    duplicate_groups: u64,
    wasted_space: u64,
}

// ============================================================================
// Public API
// ============================================================================

/// Run the storage status command
pub fn status() -> Result<()> {
    ui::header("Storage Overview");

    // Local SSD
    show_local_ssd()?;

    // iCloud Drive - collect all stats in one pass
    let icloud_stats = collect_icloud_stats();
    show_icloud(&icloud_stats);

    // T9 External
    show_t9()?;

    // Scanned manifests - collect once, use for display and hints
    let manifests = collect_manifest_stats()?;
    show_manifests(&manifests);

    // Optimization hints - use already collected data
    show_hints(&icloud_stats, &manifests);

    Ok(())
}

// ============================================================================
// Local SSD
// ============================================================================

fn show_local_ssd() -> Result<()> {
    ui::section("Local SSD");

    let stats = get_disk_space("/")?;
    let used = stats.total.saturating_sub(stats.available);
    let percent = calc_percent(used, stats.total);

    ui::kv(
        "Used",
        &format!(
            "{} / {} ({}%)",
            ui::format_size(used),
            ui::format_size(stats.total),
            percent
        ),
    );
    ui::kv("Available", &ui::format_size(stats.available));

    Ok(())
}

// ============================================================================
// iCloud Drive
// ============================================================================

/// Collect all iCloud stats in a single directory walk
fn collect_icloud_stats() -> Option<ICloudStats> {
    use walkdir::WalkDir;

    let client = icloud::Client::new().ok()?;
    let root = client.icloud_root().ok()?;

    let mut stats = ICloudStats {
        local_bytes: 0,
        cloud_bytes: 0,
        local_count: 0,
        cloud_count: 0,
        evictable_bytes: 0,
        evictable_count: 0,
    };

    for entry in WalkDir::new(&root)
        .max_depth(ICLOUD_MAX_DEPTH)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        // Skip hidden files
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with('.') {
                continue;
            }
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

fn show_icloud(stats: &Option<ICloudStats>) {
    ui::section("iCloud Drive");

    let stats = match stats {
        Some(s) => s,
        None => {
            ui::dim("Not available");
            return;
        }
    };

    ui::kv(
        "Local",
        &format!(
            "{} ({} files downloaded)",
            ui::format_size(stats.local_bytes),
            stats.local_count
        ),
    );
    ui::kv(
        "Cloud-only",
        &format!(
            "{} ({} files evicted)",
            ui::format_size(stats.cloud_bytes),
            stats.cloud_count
        ),
    );

    if stats.evictable_bytes > 0 {
        ui::kv(
            "Evictable",
            &format!(
                "{} ({} files >= {})",
                ui::format_size(stats.evictable_bytes),
                stats.evictable_count,
                ui::format_size(ICLOUD_MIN_EVICTABLE_SIZE)
            ),
        );
    }
}

// ============================================================================
// T9 External Drive
// ============================================================================

fn show_t9() -> Result<()> {
    ui::section("T9 External");

    let t9_path = Path::new(T9_MOUNT);

    if !t9_path.exists() {
        ui::kv("Status", &"Not mounted".dimmed().to_string());
        return Ok(());
    }

    ui::kv("Status", &"Mounted".green().to_string());

    match get_disk_space(T9_MOUNT) {
        Ok(stats) => {
            let used = stats.total.saturating_sub(stats.available);
            let percent = calc_percent(used, stats.total);

            ui::kv(
                "Used",
                &format!(
                    "{} / {} ({}%)",
                    ui::format_size(used),
                    ui::format_size(stats.total),
                    percent
                ),
            );
        }
        Err(e) => {
            ui::dim(&format!("Could not read space: {}", e));
        }
    }

    Ok(())
}

// ============================================================================
// Manifests
// ============================================================================

fn collect_manifest_stats() -> Result<Vec<ManifestInfo>> {
    let manifest_dir = config::config_dir()?.join("manifests");

    if !manifest_dir.exists() {
        return Ok(Vec::new());
    }

    let mut manifests = Vec::new();

    if let Ok(entries) = fs::read_dir(&manifest_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "db").unwrap_or(false) {
                if let Some(info) = load_manifest_info(&path) {
                    manifests.push(info);
                }
            }
        }
    }

    Ok(manifests)
}

fn load_manifest_info(db_path: &Path) -> Option<ManifestInfo> {
    let manifest = manifest::Manifest::open(db_path).ok()?;
    let stats = manifest.stats().ok()?;

    let name = db_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    Some(ManifestInfo {
        name,
        file_count: stats.file_count,
        total_size: stats.total_size,
        duplicate_groups: stats.duplicates.duplicate_groups,
        wasted_space: stats.duplicates.wasted_space,
    })
}

fn show_manifests(manifests: &[ManifestInfo]) {
    if manifests.is_empty() {
        return;
    }

    ui::section("Scanned Manifests");

    for info in manifests {
        let dup_str = if info.duplicate_groups > 0 {
            format!(
                " | {} dups ({})",
                info.duplicate_groups,
                ui::format_size(info.wasted_space)
            )
            .yellow()
            .to_string()
        } else {
            String::new()
        };

        println!(
            "  {} {} | {}{}",
            format!("{:>12}", ui::format_size(info.total_size)).dimmed(),
            format!("{:>8} files", info.file_count).dimmed(),
            info.name,
            dup_str
        );
    }
}

// ============================================================================
// Optimization Hints
// ============================================================================

fn show_hints(icloud_stats: &Option<ICloudStats>, manifests: &[ManifestInfo]) {
    let mut hints: Vec<String> = Vec::new();

    // iCloud evictable hint (using already-collected data)
    if let Some(stats) = icloud_stats {
        if stats.evictable_bytes >= HINT_MIN_SIZE {
            hints.push(format!(
                "Evict {} from iCloud: {}",
                ui::format_size(stats.evictable_bytes),
                "bossa icloud find-evictable".cyan()
            ));
        }
    }

    // Manifest duplicates hint (using already-collected data)
    let total_wasted: u64 = manifests.iter().map(|m| m.wasted_space).sum();
    if total_wasted >= HINT_MIN_SIZE {
        hints.push(format!(
            "Clean {} in duplicates: {}",
            ui::format_size(total_wasted),
            "bossa manifest duplicates <path>".cyan()
        ));
    }

    if !hints.is_empty() {
        ui::section("Hints");
        for hint in hints {
            println!("  {} {}", "->".dimmed(), hint);
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Calculate percentage safely, avoiding division by zero
fn calc_percent(part: u64, total: u64) -> u32 {
    if total == 0 {
        0
    } else {
        (part as f64 / total as f64 * 100.0) as u32
    }
}

struct DiskSpace {
    total: u64,
    available: u64,
}

#[cfg(unix)]
fn get_disk_space(path: &str) -> Result<DiskSpace> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = CString::new(path).context("Invalid path")?;

    unsafe {
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) != 0 {
            anyhow::bail!("statvfs failed for {}", path);
        }
        let stat = stat.assume_init();

        Ok(DiskSpace {
            total: u64::from(stat.f_blocks) * stat.f_frsize,
            available: u64::from(stat.f_bavail) * stat.f_frsize,
        })
    }
}

#[cfg(not(unix))]
fn get_disk_space(_path: &str) -> Result<DiskSpace> {
    anyhow::bail!("Disk space detection not supported on this platform")
}
