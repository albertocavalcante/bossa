//! Manifest commands - CLI wrapper around the manifest crate
//!
//! Commands:
//! - scan: Walk filesystem, hash files, store in SQLite manifest
//! - stats: Show size, file count, duplicates summary
//! - duplicates: List duplicate file sets

use anyhow::Result;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use manifest::{DuplicateGroup, Manifest, ProgressCallback, ScanResult};
use std::path::{Path, PathBuf};

use crate::config;
use crate::ui;

// ============================================================================
// Command Enum
// ============================================================================

#[derive(Debug)]
pub enum ManifestCommand {
    Scan {
        path: String,
        force: bool,
    },
    Stats {
        path: String,
    },
    Duplicates {
        path: String,
        min_size: u64,
        delete: bool,
    },
}

impl From<crate::cli::ManifestCommand> for ManifestCommand {
    fn from(cmd: crate::cli::ManifestCommand) -> Self {
        match cmd {
            crate::cli::ManifestCommand::Scan { path, force } => Self::Scan { path, force },
            crate::cli::ManifestCommand::Stats { path } => Self::Stats { path },
            crate::cli::ManifestCommand::Duplicates {
                path,
                min_size,
                delete,
            } => Self::Duplicates {
                path,
                min_size,
                delete,
            },
        }
    }
}

pub fn run(cmd: ManifestCommand) -> Result<()> {
    match cmd {
        ManifestCommand::Scan { path, force } => scan(&path, force),
        ManifestCommand::Stats { path } => stats(&path),
        ManifestCommand::Duplicates {
            path,
            min_size,
            delete,
        } => duplicates(&path, min_size, delete),
    }
}

// ============================================================================
// Progress Adapter
// ============================================================================

struct IndicatifProgress {
    pb: ProgressBar,
}

impl IndicatifProgress {
    fn new() -> Self {
        Self {
            pb: ProgressBar::hidden(),
        }
    }
}

impl ProgressCallback for IndicatifProgress {
    fn on_start(&mut self, total_files: u64, _total_size: u64) {
        self.pb = ProgressBar::new(total_files);
        let style = ProgressStyle::default_bar();
        let style = match style
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
        {
            Ok(style) => style.progress_chars("=>-"),
            Err(_) => ProgressStyle::default_bar(),
        };
        self.pb.set_style(style);
    }

    fn on_file(&mut self, path: &Path, _size: u64) {
        let path_str: String = path
            .to_string_lossy()
            .chars()
            .flat_map(char::escape_default)
            .collect();
        self.pb.set_message(ui::truncate_path(&path_str, 30));
    }

    fn on_file_complete(&mut self, _success: bool) {
        self.pb.inc(1);
    }

    fn on_complete(&mut self, _result: &ScanResult) {
        self.pb.finish_and_clear();
    }
}

// ============================================================================
// Scan Command
// ============================================================================

fn scan(path_str: &str, force: bool) -> Result<()> {
    let path = crate::paths::expand(path_str);

    ui::header(&format!(
        "Scanning: {}",
        manifest::NativePath::from_path(&path)
    ));

    if !path.exists() {
        anyhow::bail!(
            "Path does not exist: {}",
            manifest::NativePath::from_path(&path)
        );
    }

    let db_path = manifest_db_path(&path)?;
    let manifest_db = Manifest::open_for_root(&db_path, &path)?;

    // Scan with progress
    let mut progress = IndicatifProgress::new();
    let result = manifest_db.scan(&path, force, &mut progress)?;

    println!();
    ui::success(&format!("Scan complete: {} files hashed", result.hashed));
    if result.errors > 0 {
        ui::warn(&format!("  Errors: {} (could not read)", result.errors));
    }
    if result.pruned > 0 {
        ui::dim(&format!("  Pruned: {} (no longer exist)", result.pruned));
    }

    // Show duplicate summary
    if result.duplicates.duplicate_groups > 0 {
        println!();
        ui::warn(&format!(
            "Found {} duplicate groups ({} files, {} wasted)",
            result.duplicates.duplicate_groups,
            result.duplicates.duplicate_files,
            manifest::format_size(result.duplicates.wasted_space)
        ));
        ui::dim("Run 'bossa manifest duplicates <path>' to see details");
    }

    Ok(())
}

// ============================================================================
// Stats Command
// ============================================================================

fn stats(path_str: &str) -> Result<()> {
    let path = crate::paths::expand(path_str);

    ui::header(&format!(
        "Manifest Stats: {}",
        manifest::NativePath::from_path(&path)
    ));

    let db_path = existing_manifest_path(&path)?;
    let manifest_db = Manifest::open_existing(&db_path, &path)?;
    if manifest_db.is_legacy() {
        ui::warn(
            "Legacy unqualified historical inventory; root and native paths are not verified.",
        );
    }

    let stats = manifest_db.stats()?;

    println!();
    ui::kv("Total files", &stats.file_count.to_string());
    ui::kv("Total size", &manifest::format_size(stats.total_size));
    if let Some(timestamp) = stats.last_scanned_at {
        ui::kv("Last scan (Unix seconds)", &timestamp.to_string());
    }
    println!();
    ui::kv(
        "Duplicate groups",
        &stats.duplicates.duplicate_groups.to_string(),
    );
    ui::kv(
        "Duplicate files",
        &stats.duplicates.duplicate_files.to_string(),
    );
    ui::kv(
        "Wasted space",
        &manifest::format_size(stats.duplicates.wasted_space),
    );

    if stats.duplicates.wasted_space > 0 {
        ui::kv(
            "Potential savings",
            &format!("{:.1}%", stats.savings_percentage()),
        );
    }

    Ok(())
}

// ============================================================================
// Duplicates Command
// ============================================================================

fn duplicates(path_str: &str, min_size: u64, delete: bool) -> Result<()> {
    let path = crate::paths::expand(path_str);

    ui::header(&format!(
        "Duplicates: {}",
        manifest::NativePath::from_path(&path)
    ));

    let db_path = existing_manifest_path(&path)?;
    let manifest_db = Manifest::open_existing(&db_path, &path)?;
    if manifest_db.is_legacy() {
        ui::warn(
            "Legacy unqualified historical inventory; root and native paths are not verified.",
        );
    }

    let groups = manifest_db.find_duplicates(min_size)?;

    if groups.is_empty() {
        ui::success("No duplicates found!");
        return Ok(());
    }

    let total_wasted: u64 = groups.iter().map(DuplicateGroup::wasted_space).sum();

    println!();
    for (i, group) in groups.iter().enumerate() {
        print_duplicate_group(i + 1, group);

        // Limit output
        if i >= 19 && !delete {
            let remaining = groups.len() - 20;
            if remaining > 0 {
                ui::dim(&format!("... and {remaining} more duplicate groups"));
            }
            break;
        }
    }

    println!();
    ui::kv("Total duplicate groups", &groups.len().to_string());
    ui::kv("Total wasted space", &manifest::format_size(total_wasted));

    if delete {
        delete_duplicates(&path, &manifest_db, &groups)?;
    } else {
        ui::dim("Run with --delete to interactively remove duplicates");
    }

    Ok(())
}

fn print_duplicate_group(index: usize, group: &DuplicateGroup) {
    let wasted = group.wasted_space();

    println!(
        "{}. {} ({} each, {} copies, {} wasted)",
        index.to_string().bold(),
        manifest::format_size(group.size_each).yellow(),
        manifest::format_size(group.size_each),
        group.count,
        manifest::format_size(wasted).red()
    );

    for (j, file_path) in group.paths.iter().enumerate() {
        let prefix = if j == 0 { "  ★" } else { "  ✗" };
        let color = if j == 0 { "green" } else { "red" };
        if color == "green" {
            println!("  {} {}", prefix.green(), file_path);
        } else {
            println!("  {} {}", prefix.red(), file_path.to_string().dimmed());
        }
    }
    println!();
}

fn delete_duplicates(
    base_path: &Path,
    manifest_db: &Manifest,
    groups: &[DuplicateGroup],
) -> Result<()> {
    println!();
    ui::warn("Interactive deletion mode:");
    println!("  For each group, the first file (★) is kept, others (✗) are deleted.");
    println!();

    print!("  Type 'delete duplicates' to confirm: ");
    std::io::Write::flush(&mut std::io::stdout())?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    if input.trim() != "delete duplicates" {
        ui::warn("Aborted. No files deleted.");
        return Ok(());
    }

    println!();
    let (deleted_count, deleted_size) = manifest_db.delete_duplicates(base_path, groups)?;
    println!();
    ui::success(&format!(
        "Deleted {} files, freed {}",
        deleted_count,
        manifest::format_size(deleted_size)
    ));
    ui::warn("Manifest is stale; run a complete scan before using it again.");

    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

/// Compute the v2 location without creating directories or reading the root.
fn manifest_db_path(root: &Path) -> Result<PathBuf> {
    Ok(config::config_dir()?
        .join("manifests")
        .join(format!("{}.db", manifest::manifest_name(root)?)))
}

/// Prefer v2; fall back to explicitly unqualified historical data.
fn existing_manifest_path(root: &Path) -> Result<PathBuf> {
    let path = manifest_db_path(root)?;
    if path.try_exists()? {
        return Ok(path);
    }
    let legacy = config::config_dir()?
        .join("manifests")
        .join(format!("{}.db", manifest::path_to_name(root)));
    if legacy.try_exists()? {
        Ok(legacy)
    } else {
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::ManifestCommand;
    use crate::cli::ManifestCommand as CliManifestCommand;

    #[test]
    fn cli_manifest_duplicates_maps_fields() {
        let cli_cmd = CliManifestCommand::Duplicates {
            path: "/tmp/data".to_string(),
            min_size: 2048,
            delete: true,
        };

        let mapped: ManifestCommand = cli_cmd.into();
        match mapped {
            ManifestCommand::Duplicates {
                path,
                min_size,
                delete,
            } => {
                assert_eq!(path, "/tmp/data");
                assert_eq!(min_size, 2048);
                assert!(delete);
            }
            _ => panic!("expected duplicates mapping"),
        }
    }
}
