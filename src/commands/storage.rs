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
// Cross-Storage Duplicates
// ============================================================================

/// Find duplicates across scanned manifests
///
/// # Arguments
/// * `filter` - Manifest names to compare. If empty, compares all.
/// * `list_only` - If true, just list available manifests and exit.
/// * `min_size` - Minimum file size to consider.
/// * `display_limit` - Maximum duplicates to show per comparison (0 = unlimited).
pub fn duplicates(filter: &[String], list_only: bool, min_size: u64, display_limit: usize) -> Result<()> {
    let manifest_dir = config::config_dir()?.join("manifests");

    // Handle missing manifest directory
    if !manifest_dir.exists() {
        ui::header("Cross-Storage Duplicates");
        ui::warn("No manifests found.");
        println!();
        println!("  Scan storage first:");
        println!("    {} {}", "$".dimmed(), "bossa manifest scan /Volumes/T9".cyan());
        println!("    {} {}", "$".dimmed(), "bossa manifest scan ~/Library/Mobile\\ Documents/com~apple~CloudDocs".cyan());
        return Ok(());
    }

    // Collect all available manifests
    let all_manifests = collect_manifests(&manifest_dir)?;

    // Handle --list flag
    if list_only {
        ui::header("Available Manifests");
        if all_manifests.is_empty() {
            ui::dim("  No manifests found.");
        } else {
            for (name, path) in &all_manifests {
                // Get basic stats
                if let Ok(m) = manifest::Manifest::open(path) {
                    if let Ok(stats) = m.stats() {
                        println!(
                            "  {} {} files ({})",
                            format!("{:>12}", name).cyan(),
                            stats.file_count,
                            ui::format_size(stats.total_size)
                        );
                    } else {
                        println!("  {}", name.cyan());
                    }
                } else {
                    println!("  {} {}", name.cyan(), "(error opening)".red());
                }
            }
        }
        println!();
        println!("  Scan more storage:");
        println!("    {} {}", "$".dimmed(), "bossa manifest scan <path>".cyan());
        return Ok(());
    }

    ui::header("Cross-Storage Duplicates");

    // Filter manifests if names specified
    let (manifests, not_found) = filter_manifests(all_manifests, filter);

    // Report any manifests that weren't found (show original input)
    for (original, _) in &not_found {
        ui::warn(&format!("Manifest '{}' not found (case-insensitive search)", original));
    }

    // Need at least 2 manifests to compare
    if manifests.len() < 2 {
        ui::warn("Need at least 2 manifests to compare.");
        println!();
        if !filter.is_empty() {
            println!("  Requested: {}", filter.join(", "));
        }
        println!("  Available: {}", "bossa storage duplicates --list".cyan());
        return Ok(());
    }

    // Show what we're comparing
    let manifest_names: Vec<&str> = manifests.iter().map(|(n, _)| n.as_str()).collect();
    println!(
        "  Comparing: {} (min size: {})\n",
        manifest_names.join(", ").cyan(),
        ui::format_size(min_size)
    );

    let mut total_duplicates = 0u64;
    let mut total_size = 0u64;
    let mut comparison_errors = Vec::new();

    // Compare all pairs of manifests
    for i in 0..manifests.len() {
        for j in (i + 1)..manifests.len() {
            let (name_a, path_a) = &manifests[i];
            let (name_b, path_b) = &manifests[j];

            match compare_manifests(path_a, path_b, name_a, name_b, min_size, display_limit) {
                Ok((count, size)) => {
                    total_duplicates += count;
                    total_size += size;
                }
                Err(e) => {
                    comparison_errors.push(format!("{} vs {}: {}", name_a, name_b, e));
                }
            }
        }
    }

    // Report any comparison errors
    if !comparison_errors.is_empty() {
        ui::section("Errors");
        for err in &comparison_errors {
            println!("  {} {}", "✗".red(), err);
        }
        println!();
    }

    // Summary
    if total_duplicates > 0 {
        ui::section("Summary");
        ui::kv(
            "Total",
            &format!(
                "{} duplicate files across storages ({})",
                total_duplicates,
                ui::format_size(total_size)
            ),
        );
        println!();
        println!("  {} Files backed up on T9 can be safely evicted from iCloud:", "Tip:".bold());
        println!("    {} {}", "$".dimmed(), "bossa icloud evict <path> --dry-run".cyan());
    } else if comparison_errors.is_empty() {
        ui::dim("  No cross-storage duplicates found.");
    }

    Ok(())
}

/// Collect all manifest databases from the manifest directory
fn collect_manifests(manifest_dir: &std::path::Path) -> Result<Vec<(String, std::path::PathBuf)>> {
    let mut manifests: Vec<(String, std::path::PathBuf)> = fs::read_dir(manifest_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension().map(|e| e == "db").unwrap_or(false) {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                Some((name, path))
            } else {
                None
            }
        })
        .collect();

    manifests.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(manifests)
}

/// A manifest entry: (name, path)
type ManifestEntry = (String, std::path::PathBuf);

/// Filter manifests by name (case-insensitive)
///
/// Returns (matched_manifests, not_found_with_original_input)
#[allow(clippy::type_complexity)]
fn filter_manifests(
    all_manifests: Vec<ManifestEntry>,
    filter: &[String],
) -> (Vec<ManifestEntry>, Vec<(String, String)>) {
    if filter.is_empty() {
        return (all_manifests, Vec::new());
    }

    let mut matched = Vec::new();
    let mut not_found = Vec::new();

    for requested in filter {
        let requested_lower = requested.to_lowercase();
        if let Some(m) = all_manifests
            .iter()
            .find(|(name, _)| name.to_lowercase() == requested_lower)
        {
            // Avoid duplicates in matched list
            if !matched.iter().any(|(n, _): &(String, _)| n == &m.0) {
                matched.push(m.clone());
            }
        } else {
            not_found.push((requested.clone(), requested_lower));
        }
    }

    (matched, not_found)
}

/// Compare two manifests and print duplicates
fn compare_manifests(
    path_a: &std::path::Path,
    path_b: &std::path::Path,
    name_a: &str,
    name_b: &str,
    min_size: u64,
    display_limit: usize,
) -> Result<(u64, u64)> {
    let manifest_a = manifest::Manifest::open(path_a)
        .context(format!("Failed to open manifest: {}", name_a))?;

    let cross_dups = manifest_a
        .compare_with(path_b, min_size)
        .context(format!("Failed to compare {} with {}", name_a, name_b))?;

    if cross_dups.is_empty() {
        return Ok((0, 0));
    }

    let count = cross_dups.len() as u64;
    let total_size: u64 = cross_dups.iter().map(|d| d.size).sum();

    // Header showing which manifests are being compared with clear labeling
    println!(
        "  {} {} {}\n",
        format!("{} (source)", name_a).green(),
        "↔".dimmed(),
        format!("{} (also exists)", name_b).blue()
    );

    // Show duplicates (respect display_limit, 0 = unlimited)
    let limit = if display_limit == 0 { usize::MAX } else { display_limit };
    for dup in cross_dups.iter().take(limit) {
        println!(
            "    {} {}",
            format!("{:>10}", ui::format_size(dup.size)).dimmed(),
            dup.source_path
        );
        println!(
            "             {} {}",
            "└─".dimmed(),
            dup.other_path.dimmed()
        );
    }

    if count > limit as u64 {
        println!(
            "    {}  ... and {} more (use {} to see all)",
            " ".repeat(10),
            count - limit as u64,
            "--limit 0".cyan()
        );
    }

    println!(
        "\n    {} {} files ({})\n",
        "Subtotal:".bold(),
        count,
        ui::format_size(total_size)
    );

    Ok((count, total_size))
}

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
