//! Disk backup command - copy directory with progress
//!
//! Copies files from source to destination while:
//! - Showing progress with a progress bar
//! - Skipping system files (.DS_Store, .Spotlight-V100, .fseventsd, .Trashes)
//! - Validating destination has enough space
//! - Supporting dry-run mode

use anyhow::{Context, Result};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::ui;

/// Files and directories to skip during backup
const SKIP_PATTERNS: &[&str] = &[
    ".DS_Store",
    ".Spotlight-V100",
    ".fseventsd",
    ".Trashes",
    ".TemporaryItems",
    ".VolumeIcon.icns",
    ".com.apple.timemachine.donotpresent",
    ".DocumentRevisions-V100",
    ".PKInstallSandboxManager-SystemSoftware",
];

/// Entry to be copied
struct CopyEntry {
    source: PathBuf,
    dest: PathBuf,
    size: u64,
    is_dir: bool,
}

/// Run the backup command
pub fn run(source: &str, destination: &str, dry_run: bool) -> Result<()> {
    let source_path = expand_path(source);
    let dest_path = expand_path(destination);

    // Validate source exists
    if !source_path.exists() {
        anyhow::bail!("Source path does not exist: {}", source_path.display());
    }

    if dry_run {
        ui::header("Backup (Dry Run)");
    } else {
        ui::header("Backup");
    }
    println!();

    ui::kv("Source", &source_path.display().to_string());
    ui::kv("Destination", &dest_path.display().to_string());
    println!();

    // Collect files to copy
    ui::dim("Scanning source...");
    let entries = collect_entries(&source_path, &dest_path)?;

    if entries.is_empty() {
        ui::info("No files to copy");
        return Ok(());
    }

    let total_size: u64 = entries.iter().map(|e| e.size).sum();
    let file_count = entries.iter().filter(|e| !e.is_dir).count();
    let dir_count = entries.iter().filter(|e| e.is_dir).count();

    println!(
        "  {} files, {} directories ({})",
        file_count,
        dir_count,
        ui::format_size(total_size)
    );
    println!();

    // Check destination space
    if !dry_run {
        check_destination_space(&dest_path, total_size)?;
    }

    // Show what would be skipped
    let skip_info = count_skipped(&source_path);
    if skip_info.count > 0 {
        ui::dim(&format!(
            "Skipping {} system files ({})",
            skip_info.count,
            ui::format_size(skip_info.size)
        ));
        println!();
    }

    if dry_run {
        // Show what would be copied
        let max_show = 20;
        let file_entries: Vec<_> = entries.iter().filter(|e| !e.is_dir).collect();

        for entry in file_entries.iter().take(max_show) {
            let rel_path = entry
                .source
                .strip_prefix(&source_path)
                .unwrap_or(&entry.source);
            println!(
                "  {} {} ({})",
                "Would copy:".dimmed(),
                rel_path.display(),
                ui::format_size(entry.size)
            );
        }

        if file_entries.len() > max_show {
            ui::dim(&format!(
                "  ... and {} more files",
                file_entries.len() - max_show
            ));
        }

        println!();
        ui::dim("(dry run - no files copied)");
    } else {
        // Perform the copy
        perform_backup(&entries, &source_path, total_size)?;
    }

    Ok(())
}

/// Expand ~ and environment variables in paths
fn expand_path(path: &str) -> PathBuf {
    crate::paths::expand(path)
}

/// Check if a path should be skipped
/// Only skips specific macOS system files, NOT all dotfiles
fn should_skip(path: &Path) -> bool {
    // Check the file/directory name against the explicit skip list
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if SKIP_PATTERNS.contains(&name) {
            return true;
        }

        // Also skip AppleDouble files (._filename) which are macOS metadata
        if name.starts_with("._") {
            return true;
        }
    }

    false
}

/// Collect all entries to be copied
fn collect_entries(source: &Path, dest_base: &Path) -> Result<Vec<CopyEntry>> {
    let mut entries = Vec::new();

    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        // Check if this or any ancestor should be skipped
        if path.ancestors().any(should_skip) {
            continue;
        }

        let rel_path = path.strip_prefix(source).unwrap_or(path);
        let dest_path = dest_base.join(rel_path);

        let metadata = entry.metadata().context("Failed to read metadata")?;
        let size = if metadata.is_file() {
            metadata.len()
        } else {
            0
        };

        entries.push(CopyEntry {
            source: path.to_path_buf(),
            dest: dest_path,
            size,
            is_dir: metadata.is_dir(),
        });
    }

    Ok(entries)
}

/// Count skipped files
struct SkipInfo {
    count: usize,
    size: u64,
}

fn count_skipped(source: &Path) -> SkipInfo {
    let mut count = 0;
    let mut size = 0u64;

    for entry in WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if should_skip(entry.path()) {
            count += 1;
            if let Ok(meta) = entry.metadata() {
                size += meta.len();
            }
        }
    }

    SkipInfo { count, size }
}

/// Check if destination has enough space
#[cfg(unix)]
fn check_destination_space(dest: &Path, required: u64) -> Result<()> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    // Get the mount point for the destination (or its parent if it doesn't exist)
    let check_path = if dest.exists() {
        dest.to_path_buf()
    } else {
        dest.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/"))
    };

    let path_str = check_path.to_string_lossy();
    let c_path = CString::new(path_str.as_ref()).context("Invalid path")?;

    // SAFETY: statvfs is a standard POSIX call
    let available = unsafe {
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        let result = libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr());

        if result != 0 {
            anyhow::bail!("Failed to check disk space for destination");
        }

        let stat = stat.assume_init();

        // Cast needed on macOS, not on Linux
        #[allow(clippy::unnecessary_cast)]
        let avail = stat.f_bavail as u64 * stat.f_frsize;
        avail
    };

    if available < required {
        anyhow::bail!(
            "Not enough space on destination. Need {}, only {} available",
            ui::format_size(required),
            ui::format_size(available)
        );
    }

    ui::kv(
        "Destination space",
        &format!(
            "{} available (need {})",
            ui::format_size(available).green(),
            ui::format_size(required)
        ),
    );

    Ok(())
}

#[cfg(not(unix))]
fn check_destination_space(_dest: &Path, _required: u64) -> Result<()> {
    ui::warn("Cannot check destination space on this platform");
    Ok(())
}

/// Perform the actual backup
fn perform_backup(entries: &[CopyEntry], source_base: &Path, total_size: u64) -> Result<()> {
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    let mut copied_count = 0;
    let mut error_count = 0;
    let mut copied_bytes = 0u64;

    // First pass: create directories
    for entry in entries.iter().filter(|e| e.is_dir) {
        if let Err(e) = fs::create_dir_all(&entry.dest) {
            pb.suspend(|| {
                let rel_path = entry
                    .source
                    .strip_prefix(source_base)
                    .unwrap_or(&entry.source);
                println!(
                    "  {} Failed to create directory {}: {}",
                    "!".red(),
                    rel_path.display(),
                    e
                );
            });
            error_count += 1;
        }
    }

    // Second pass: copy files
    for entry in entries.iter().filter(|e| !e.is_dir) {
        let rel_path = entry
            .source
            .strip_prefix(source_base)
            .unwrap_or(&entry.source);

        pb.set_message(ui::truncate_path(&rel_path.display().to_string(), 40));

        // Ensure parent directory exists
        if let Some(parent) = entry.dest.parent()
            && !parent.exists()
            && let Err(e) = fs::create_dir_all(parent)
        {
            pb.suspend(|| {
                println!("  {} Failed to create parent directory: {}", "!".red(), e);
            });
            error_count += 1;
            continue;
        }

        match fs::copy(&entry.source, &entry.dest) {
            Ok(bytes) => {
                copied_count += 1;
                copied_bytes += bytes;
                pb.inc(entry.size);
            }
            Err(e) => {
                pb.suspend(|| {
                    println!(
                        "  {} Failed to copy {}: {}",
                        "!".red(),
                        rel_path.display(),
                        e
                    );
                });
                error_count += 1;
            }
        }
    }

    pb.finish_and_clear();

    // Summary
    println!();
    if error_count == 0 {
        ui::success(&format!(
            "Copied {} files ({})",
            copied_count,
            ui::format_size(copied_bytes)
        ));
    } else {
        ui::warn(&format!(
            "Copied {} files ({}) with {} errors",
            copied_count,
            ui::format_size(copied_bytes),
            error_count
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_skip_ds_store() {
        assert!(should_skip(Path::new(".DS_Store")));
        assert!(should_skip(Path::new("/some/path/.DS_Store")));
    }

    #[test]
    fn test_should_skip_spotlight() {
        assert!(should_skip(Path::new(".Spotlight-V100")));
    }

    #[test]
    fn test_should_not_skip_regular_files() {
        assert!(!should_skip(Path::new("myfile.txt")));
        assert!(!should_skip(Path::new("/some/path/document.pdf")));
    }

    #[test]
    fn test_should_allow_dotfiles() {
        // Regular dotfiles should NOT be skipped
        assert!(!should_skip(Path::new(".gitignore")));
        assert!(!should_skip(Path::new(".github")));
        assert!(!should_skip(Path::new(".hidden")));
        assert!(!should_skip(Path::new(".iso_manifest.txt")));
        assert!(!should_skip(Path::new(".env")));
        assert!(!should_skip(Path::new(".bashrc")));
    }

    #[test]
    fn test_should_skip_apple_double_files() {
        // AppleDouble metadata files (._filename) should be skipped
        assert!(should_skip(Path::new("._myfile.txt")));
        assert!(should_skip(Path::new("._Document.pdf")));
    }

    #[test]
    fn test_expand_path() {
        let path = expand_path("/absolute/path");
        assert_eq!(path, PathBuf::from("/absolute/path"));
    }
}
