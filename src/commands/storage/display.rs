//! Display functions for storage status

use anyhow::Result;
use colored::Colorize;
use std::path::Path;

use crate::ui;

use super::collectors::ICLOUD_MIN_EVICTABLE_SIZE;
use super::disk::{format_disk_usage, get_disk_space};
use super::types::{ICloudStats, ManifestEntry, ManifestInfo};

// ============================================================================
// Constants
// ============================================================================

/// T9 external drive mount point
pub const T9_MOUNT: &str = "/Volumes/T9";

/// Minimum size to show in optimization hints (100 MB)
const HINT_MIN_SIZE: u64 = 100 * 1024 * 1024;

// ============================================================================
// Local SSD Display
// ============================================================================

/// Display local SSD space usage
pub fn show_local_ssd() -> Result<()> {
    ui::section("Local SSD");

    let stats = get_disk_space("/")?;

    ui::kv("Used", &format_disk_usage(stats.used(), stats.total, ui::format_size));
    ui::kv("Available", &ui::format_size(stats.available));

    Ok(())
}

// ============================================================================
// iCloud Display
// ============================================================================

/// Display iCloud Drive statistics
pub fn show_icloud(stats: &Option<ICloudStats>) {
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
// T9 External Drive Display
// ============================================================================

/// Display T9 external drive status
pub fn show_t9() -> Result<()> {
    ui::section("T9 External");

    let t9_path = Path::new(T9_MOUNT);

    if !t9_path.exists() {
        ui::kv("Status", &"Not mounted".dimmed().to_string());
        return Ok(());
    }

    ui::kv("Status", &"Mounted".green().to_string());

    match get_disk_space(T9_MOUNT) {
        Ok(stats) => {
            ui::kv("Used", &format_disk_usage(stats.used(), stats.total, ui::format_size));
        }
        Err(e) => {
            ui::dim(&format!("Could not read space: {}", e));
        }
    }

    Ok(())
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
                    format!("{:>12}", entry.name).cyan(),
                    stats.file_count,
                    ui::format_size(stats.total_size)
                );
            } else {
                println!("  {}", entry.name.cyan());
            }
        } else {
            println!("  {} {}", entry.name.cyan(), "(error opening)".red());
        }
    }
}

// ============================================================================
// Optimization Hints Display
// ============================================================================

/// Display optimization hints based on collected stats
pub fn show_hints(icloud_stats: &Option<ICloudStats>, manifests: &[ManifestInfo]) {
    let mut hints: Vec<String> = Vec::new();

    // iCloud evictable hint
    if let Some(stats) = icloud_stats {
        if stats.evictable_bytes >= HINT_MIN_SIZE {
            hints.push(format!(
                "Evict {} from iCloud: {}",
                ui::format_size(stats.evictable_bytes),
                "bossa icloud find-evictable".cyan()
            ));
        }
    }

    // Manifest duplicates hint
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
// Help Messages
// ============================================================================

/// Display help for scanning manifests
pub fn show_scan_help() {
    println!("  Scan storage first:");
    println!("    {} {}", "$".dimmed(), "bossa manifest scan /Volumes/T9".cyan());
    println!(
        "    {} {}",
        "$".dimmed(),
        "bossa manifest scan ~/Library/Mobile\\ Documents/com~apple~CloudDocs".cyan()
    );
}

/// Display help for adding more manifests
pub fn show_add_manifest_help() {
    println!("  Scan more storage:");
    println!("    {} {}", "$".dimmed(), "bossa manifest scan <path>".cyan());
}
