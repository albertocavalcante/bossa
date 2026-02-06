//! Cross-storage duplicate detection

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;

use crate::config;
use crate::ui;

use super::collectors::collect_manifest_entries;
use super::display::{show_add_manifest_help, show_manifest_list, show_scan_help};
use super::types::ManifestEntry;

// ============================================================================
// Constants
// ============================================================================

/// Column width for size display
const SIZE_COLUMN_WIDTH: usize = 10;

// ============================================================================
// Public API
// ============================================================================

/// Find duplicates across scanned manifests
///
/// # Arguments
/// * `filter` - Manifest names to compare. If empty, compares all.
/// * `list_only` - If true, just list available manifests and exit.
/// * `min_size` - Minimum file size to consider.
/// * `display_limit` - Maximum duplicates to show per comparison (0 = unlimited).
pub fn run(filter: &[String], list_only: bool, min_size: u64, display_limit: usize) -> Result<()> {
    let manifest_dir = config::config_dir()?.join("manifests");

    // Handle missing manifest directory
    if !manifest_dir.exists() {
        show_no_manifests_error();
        return Ok(());
    }

    // Collect all available manifests
    let all_manifests = collect_manifest_entries(&manifest_dir)?;

    // Handle --list flag
    if list_only {
        show_manifests_list(&all_manifests);
        return Ok(());
    }

    ui::header("Cross-Storage Duplicates");

    // Filter manifests if names specified
    let (manifests, not_found) = filter_manifests(all_manifests, filter);

    // Report any manifests that weren't found
    report_missing_manifests(&not_found);

    // Validate we have enough manifests
    if manifests.len() < 2 {
        show_insufficient_manifests_error(filter);
        return Ok(());
    }

    // Run comparisons
    let results = run_comparisons(&manifests, min_size, display_limit);

    // Display results
    display_results(&results);

    Ok(())
}

// ============================================================================
// Manifest Filtering
// ============================================================================

/// Filter manifests by name (case-insensitive)
fn filter_manifests(
    all_manifests: Vec<ManifestEntry>,
    filter: &[String],
) -> (Vec<ManifestEntry>, Vec<String>) {
    if filter.is_empty() {
        return (all_manifests, Vec::new());
    }

    let mut matched = Vec::new();
    let mut not_found = Vec::new();

    for requested in filter {
        let requested_lower = requested.to_lowercase();
        if let Some(m) = all_manifests
            .iter()
            .find(|m| m.name.to_lowercase() == requested_lower)
        {
            // Avoid duplicates in matched list
            if !matched
                .iter()
                .any(|existing: &ManifestEntry| existing.name == m.name)
            {
                matched.push(m.clone());
            }
        } else {
            not_found.push(requested.clone());
        }
    }

    (matched, not_found)
}

fn report_missing_manifests(not_found: &[String]) {
    for name in not_found {
        ui::warn(&format!(
            "Manifest '{name}' not found (case-insensitive search)"
        ));
    }
}

// ============================================================================
// Comparison Logic
// ============================================================================

/// Results from running all comparisons
struct ComparisonResults {
    total_duplicates: u64,
    total_size: u64,
    errors: Vec<String>,
}

/// Run pairwise comparisons between all manifests
fn run_comparisons(
    manifests: &[ManifestEntry],
    min_size: u64,
    display_limit: usize,
) -> ComparisonResults {
    // Show what we're comparing
    let names: Vec<&str> = manifests.iter().map(|m| m.name.as_str()).collect();
    println!(
        "  Comparing: {} (min size: {})\n",
        names.join(", ").cyan(),
        ui::format_size(min_size)
    );

    let mut results = ComparisonResults {
        total_duplicates: 0,
        total_size: 0,
        errors: Vec::new(),
    };

    // Compare all pairs
    for i in 0..manifests.len() {
        for j in (i + 1)..manifests.len() {
            let a = &manifests[i];
            let b = &manifests[j];

            match compare_pair(&a.path, &b.path, &a.name, &b.name, min_size, display_limit) {
                Ok((count, size)) => {
                    results.total_duplicates += count;
                    results.total_size += size;
                }
                Err(e) => {
                    results
                        .errors
                        .push(format!("{} vs {}: {}", a.name, b.name, e));
                }
            }
        }
    }

    results
}

/// Compare two manifests and display their duplicates
fn compare_pair(
    path_a: &Path,
    path_b: &Path,
    name_a: &str,
    name_b: &str,
    min_size: u64,
    display_limit: usize,
) -> Result<(u64, u64)> {
    let manifest_a =
        manifest::Manifest::open(path_a).context(format!("Failed to open manifest: {name_a}"))?;

    let cross_dups = manifest_a
        .compare_with(path_b, min_size)
        .context(format!("Failed to compare {name_a} with {name_b}"))?;

    if cross_dups.is_empty() {
        return Ok((0, 0));
    }

    let count = cross_dups.len() as u64;
    let total_size: u64 = cross_dups.iter().map(|d| d.size).sum();

    // Display header
    println!(
        "  {} {} {}\n",
        format!("{name_a} (source)").green(),
        "↔".dimmed(),
        format!("{name_b} (also exists)").blue()
    );

    // Display duplicates (respect limit, 0 = unlimited)
    let effective_limit = if display_limit == 0 {
        usize::MAX
    } else {
        display_limit
    };

    for dup in cross_dups.iter().take(effective_limit) {
        println!(
            "    {} {}",
            format!(
                "{:>width$}",
                ui::format_size(dup.size),
                width = SIZE_COLUMN_WIDTH
            )
            .dimmed(),
            dup.source_path
        );
        println!(
            "    {:>width$} {} {}",
            "",
            "└─".dimmed(),
            dup.other_path.dimmed(),
            width = SIZE_COLUMN_WIDTH
        );
    }

    // Show "and X more" if truncated
    if count > effective_limit as u64 {
        println!(
            "    {:>width$}  ... and {} more (use {} to see all)",
            "",
            count - effective_limit as u64,
            "--limit 0".cyan(),
            width = SIZE_COLUMN_WIDTH
        );
    }

    // Subtotal for this pair
    println!(
        "\n    {} {} files ({})\n",
        "Subtotal:".bold(),
        count,
        ui::format_size(total_size)
    );

    Ok((count, total_size))
}

// ============================================================================
// Results Display
// ============================================================================

fn display_results(results: &ComparisonResults) {
    // Report any comparison errors
    if !results.errors.is_empty() {
        ui::section("Errors");
        for err in &results.errors {
            println!("  {} {}", "✗".red(), err);
        }
        println!();
    }

    // Summary
    if results.total_duplicates > 0 {
        ui::section("Summary");
        ui::kv(
            "Total",
            &format!(
                "{} duplicate files across storages ({})",
                results.total_duplicates,
                ui::format_size(results.total_size)
            ),
        );
        println!();
        println!(
            "  {} Files backed up on T9 can be safely evicted from iCloud:",
            "Tip:".bold()
        );
        println!(
            "    {} {}",
            "$".dimmed(),
            "bossa icloud evict <path> --dry-run".cyan()
        );
    } else if results.errors.is_empty() {
        ui::dim("  No cross-storage duplicates found.");
    }
}

// ============================================================================
// Error/Help Display
// ============================================================================

fn show_no_manifests_error() {
    ui::header("Cross-Storage Duplicates");
    ui::warn("No manifests found.");
    println!();
    show_scan_help();
}

fn show_manifests_list(manifests: &[ManifestEntry]) {
    ui::header("Available Manifests");
    show_manifest_list(manifests);
    println!();
    show_add_manifest_help();
}

fn show_insufficient_manifests_error(filter: &[String]) {
    ui::warn("Need at least 2 manifests to compare.");
    println!();
    if !filter.is_empty() {
        println!("  Requested: {}", filter.join(", "));
    }
    println!("  Available: {}", "bossa storage duplicates --list".cyan());
}
