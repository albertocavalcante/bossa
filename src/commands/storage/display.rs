//! Display functions for storage status

use colored::Colorize;

use crate::ui;

use super::disk::format_disk_usage;
use super::types::{Coverage, DiskSpace, ICloudStats, ManifestEntry, ManifestInfo};

// ============================================================================
// Constants
// ============================================================================

/// T9 external drive mount point
pub const T9_MOUNT: &str = "/Volumes/T9";

// ============================================================================
// Local SSD Display
// ============================================================================

/// Display local SSD space usage
pub fn show_local_ssd(stats: &Option<DiskSpace>) {
    ui::section("Local SSD");
    if let Some(stats) = stats {
        ui::kv(
            "Used",
            &format_disk_usage(stats.used(), stats.total, ui::format_size),
        );
        ui::kv("Available", &ui::format_size(stats.available));
    } else {
        ui::dim("Unavailable");
    }
}

pub fn show_icloud(stats: &ICloudStats) {
    ui::section("iCloud Drive metadata");
    if let Some(root) = &stats.root {
        ui::kv("Root", &root.display().to_string());
    }
    ui::kv(
        "Scope",
        &format!(
            "depth <= {}; max {} entries; hidden excluded",
            stats.max_depth, stats.max_entries
        ),
    );
    if stats.filesystem_coverage == Coverage::Unavailable {
        ui::dim("Filesystem observation unavailable; totals unknown");
        return;
    }
    ui::kv(
        "Filesystem coverage",
        if stats.filesystem_coverage == Coverage::Complete {
            "complete for stated scope"
        } else {
            "partial; totals unknown"
        },
    );
    if let Some(bytes) = stats.known_logical_bytes {
        ui::kv(
            "Known logical bytes",
            &format!(
                "at least {} ({} observed files)",
                ui::format_size(bytes),
                stats.observed_files
            ),
        );
    } else {
        ui::dim("Known-size subtotal unavailable");
    }
    ui::kv(
        "iCloud residency",
        &format!(
            "unknown ({} observed files); no state check",
            stats.unknown_states
        ),
    );
    ui::kv(
        "Omissions",
        &format!(
            "walk errors {}, metadata errors {}, unknown sizes {}, hidden {}, depth-limited directories {}, other {}",
            stats.walk_errors,
            stats.metadata_errors,
            stats.unknown_sizes,
            stats.skipped_hidden,
            stats.depth_limited_directories,
            stats.skipped_other
        ),
    );
    if stats.entry_limit_reached {
        ui::dim("Entry limit reached; remaining coverage is unknown.");
    }
    ui::dim("Logical sizes are not allocated or reclaimable bytes.");
}

pub fn show_t9(stats: &Option<DiskSpace>) {
    ui::section("T9 External");
    if let Some(stats) = stats {
        ui::kv(
            "Used",
            &format_disk_usage(stats.used(), stats.total, ui::format_size),
        );
    } else {
        ui::dim("Unavailable or not mounted");
    }
}

// ============================================================================
// Manifest Display
// ============================================================================

/// Display scanned manifest statistics
pub fn show_manifests(manifests: &[ManifestInfo]) {
    if manifests.is_empty() {
        return;
    }

    ui::section("Scanned Manifests");

    for info in manifests {
        let dup_str = if info.duplicate_groups > 0 {
            format!(
                " | {} duplicate groups ({} logical bytes; not reclaimable space)",
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

/// Display list of available manifests with stats
pub fn show_manifest_list(manifests: &[ManifestEntry]) {
    if manifests.is_empty() {
        ui::dim("  No manifests found.");
        return;
    }

    for entry in manifests {
        if let Ok(m) = manifest::Manifest::open(&entry.path) {
            if let Ok(stats) = m.stats() {
                println!(
                    "  {} {} files ({})",
                    format!(
                        "{:>12}{}",
                        entry.name,
                        if m.is_legacy() {
                            " (legacy unqualified)"
                        } else {
                            ""
                        }
                    )
                    .cyan(),
                    stats.file_count,
                    ui::format_size(stats.total_size)
                );
            } else {
                println!(
                    "  {} {}",
                    entry.name.cyan(),
                    "(manifest unavailable or stale)".red()
                );
            }
        } else {
            println!(
                "  {} {}",
                entry.name.cyan(),
                "(error opening manifest)".red()
            );
        }
    }
}

// ============================================================================
// Help Messages
// ============================================================================

/// Display help for scanning manifests
pub fn show_scan_help() {
    println!("  Scan storage first:");
    println!(
        "    {} {}",
        "$".dimmed(),
        "bossa manifest scan /Volumes/T9".cyan()
    );
    println!(
        "    {} {}",
        "$".dimmed(),
        "bossa manifest scan ~/Library/Mobile\\ Documents/com~apple~CloudDocs".cyan()
    );
}

/// Display help for adding more manifests
pub fn show_add_manifest_help() {
    println!("  Scan more storage:");
    println!(
        "    {} {}",
        "$".dimmed(),
        "bossa manifest scan <path>".cyan()
    );
}
