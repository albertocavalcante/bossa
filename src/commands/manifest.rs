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
use std::fs;
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
        self.pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
                .unwrap()
                .progress_chars("=>-"),
        );
    }

    fn on_file(&mut self, path: &Path, _size: u64) {
        let path_str = path.to_string_lossy();
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
    let path = PathBuf::from(shellexpand::tilde(path_str).as_ref());
    let name = manifest::path_to_name(&path);

    ui::header(&format!("Scanning: {}", path.display()));

    if !path.exists() {
        anyhow::bail!("Path does not exist: {}", path.display());
    }

    let db_path = manifest_db_path(&name)?;
    let manifest_db = Manifest::open(&db_path)?;

    // Count files first
    ui::info("Counting files...");
    let (file_count, total_size) = count_files(&path);

    ui::kv("Files found", &file_count.to_string());
    ui::kv("Total size", &manifest::format_size(total_size));
    println!();

    if file_count == 0 {
        ui::info("No files to scan.");
        return Ok(());
    }

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
    let path = PathBuf::from(shellexpand::tilde(path_str).as_ref());
    let name = manifest::path_to_name(&path);

    ui::header(&format!("Manifest Stats: {}", path.display()));

    let db_path = manifest_db_path(&name)?;
    let manifest_db = Manifest::open(&db_path)?;

    let stats = manifest_db.stats()?;

    println!();
    ui::kv("Total files", &stats.file_count.to_string());
    ui::kv("Total size", &manifest::format_size(stats.total_size));
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
    let path = PathBuf::from(shellexpand::tilde(path_str).as_ref());
    let name = manifest::path_to_name(&path);

    ui::header(&format!("Duplicates: {}", path.display()));

    let db_path = manifest_db_path(&name)?;
    let manifest_db = Manifest::open(&db_path)?;

    let groups = manifest_db.find_duplicates(min_size)?;

    if groups.is_empty() {
        ui::success("No duplicates found!");
        return Ok(());
    }

    let mut total_wasted: u64 = 0;

    println!();
    for (i, group) in groups.iter().enumerate() {
        let wasted = group.wasted_space();
        total_wasted += wasted;

        print_duplicate_group(i + 1, group);

        // Limit output
        if i >= 19 && !delete {
            let remaining = groups.len() - 20;
            if remaining > 0 {
                ui::dim(&format!("... and {} more duplicate groups", remaining));
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
            println!("  {} {}", prefix.red(), file_path.dimmed());
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
    let mut deleted_count = 0u64;
    let mut deleted_size = 0u64;

    for group in groups {
        // Keep first, delete rest
        for file_path in group.paths.iter().skip(1) {
            let full_path = base_path.join(file_path);
            match fs::remove_file(&full_path) {
                Ok(()) => {
                    manifest_db.delete_entry(file_path)?;
                    deleted_count += 1;
                    deleted_size += group.size_each;
                    println!("  {} Deleted: {}", "✓".green(), file_path);
                }
                Err(e) => {
                    println!("  {} Failed: {} ({})", "✗".red(), file_path, e);
                }
            }
        }
    }

    println!();
    ui::success(&format!(
        "Deleted {} files, freed {}",
        deleted_count,
        manifest::format_size(deleted_size)
    ));

    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

/// Get the path to the manifest database for a given name
fn manifest_db_path(name: &str) -> Result<PathBuf> {
    let manifest_dir = config::config_dir()?.join("manifests");
    fs::create_dir_all(&manifest_dir)?;
    Ok(manifest_dir.join(format!("{}.db", name)))
}

/// Count files in a directory
fn count_files(path: &Path) -> (u64, u64) {
    use walkdir::WalkDir;

    let mut file_count = 0u64;
    let mut total_size = 0u64;

    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            file_count += 1;
            if let Ok(meta) = entry.metadata() {
                total_size += meta.len();
            }
        }
    }

    (file_count, total_size)
}
