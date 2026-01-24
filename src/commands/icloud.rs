//! iCloud Drive storage management commands
//!
//! Commands:
//! - status: Show status of iCloud files
//! - list: List files with their status
//! - find-evictable: Find large local files that could be evicted
//! - evict: Remove local copy, keep cloud copy
//! - download: Fetch cloud copy to local

use anyhow::{Context, Result};
use colored::Colorize;
use icloud::{Client, DownloadState, FileStatus};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};

use crate::ui;

/// iCloud command variants (matches cli::ICloudCommand)
pub enum ICloudCommand {
    Status {
        path: Option<String>,
    },
    List {
        path: Option<String>,
        local: bool,
        cloud: bool,
    },
    FindEvictable {
        path: Option<String>,
        min_size: String,
    },
    Evict {
        path: String,
        recursive: bool,
        min_size: Option<String>,
        dry_run: bool,
    },
    Download {
        path: String,
        recursive: bool,
    },
}

/// Run an iCloud command
pub fn run(cmd: ICloudCommand) -> Result<()> {
    match cmd {
        ICloudCommand::Status { path } => status(path),
        ICloudCommand::List { path, local, cloud } => list(path, local, cloud),
        ICloudCommand::FindEvictable { path, min_size } => find_evictable(path, &min_size),
        ICloudCommand::Evict {
            path,
            recursive,
            min_size,
            dry_run,
        } => evict(&path, recursive, min_size.as_deref(), dry_run),
        ICloudCommand::Download { path, recursive } => download(&path, recursive),
    }
}

// ============================================================================
// Status Command
// ============================================================================

fn status(path: Option<String>) -> Result<()> {
    let client = Client::new().context("Failed to initialize iCloud client")?;
    let icloud_root = client.icloud_root()?;

    let target_path = path
        .map(|p| expand_path(&p))
        .unwrap_or_else(|| icloud_root.clone());

    ui::header("iCloud Drive Status");
    println!();

    if !client.is_in_icloud(&target_path) {
        ui::warn(&format!(
            "Path is not in iCloud Drive: {}",
            target_path.display()
        ));
        ui::dim(&format!("iCloud root: {}", icloud_root.display()));
        return Ok(());
    }

    if target_path.is_file() {
        let file_status = client.status(&target_path)?;
        print_file_status(&file_status);
    } else if target_path.is_dir() {
        let files = client.list(&target_path)?;
        print_directory_summary(&files, &target_path);
    } else {
        ui::warn(&format!("Path not found: {}", target_path.display()));
    }

    Ok(())
}

// ============================================================================
// List Command
// ============================================================================

fn list(path: Option<String>, local_only: bool, cloud_only: bool) -> Result<()> {
    let client = Client::new().context("Failed to initialize iCloud client")?;
    let icloud_root = client.icloud_root()?;

    let target_path = path
        .map(|p| expand_path(&p))
        .unwrap_or_else(|| icloud_root.clone());

    if !client.is_in_icloud(&target_path) {
        anyhow::bail!("Path is not in iCloud Drive: {}", target_path.display());
    }

    let files = client.list(&target_path)?;

    let filtered: Vec<_> = files
        .into_iter()
        .filter(|f| {
            if local_only {
                f.state.is_local()
            } else if cloud_only {
                f.state.is_cloud_only()
            } else {
                true
            }
        })
        .collect();

    ui::header(&format!("iCloud Files: {}", target_path.display()));
    println!();

    if filtered.is_empty() {
        ui::dim("No files found matching criteria");
        return Ok(());
    }

    for file in &filtered {
        print_file_line(file, &target_path);
    }

    println!();
    ui::dim(&format!("{} files", filtered.len()));

    Ok(())
}

// ============================================================================
// Find Evictable Command
// ============================================================================

fn find_evictable(path: Option<String>, min_size_str: &str) -> Result<()> {
    let client = Client::new().context("Failed to initialize iCloud client")?;
    let icloud_root = client.icloud_root()?;

    let target_path = path
        .map(|p| expand_path(&p))
        .unwrap_or_else(|| icloud_root.clone());

    let min_size = ui::parse_size(min_size_str)
        .map_err(|e| anyhow::anyhow!("Invalid size '{}': {}", min_size_str, e))?;

    if !client.is_in_icloud(&target_path) {
        anyhow::bail!("Path is not in iCloud Drive: {}", target_path.display());
    }

    ui::header("Evictable Files");
    ui::dim(&format!(
        "Local files >= {} that could be evicted to free space",
        ui::format_size(min_size)
    ));
    println!();

    let evictable = client.find_evictable(&target_path, min_size)?;

    if evictable.is_empty() {
        ui::success(&format!(
            "No local files >= {} found",
            ui::format_size(min_size)
        ));
        return Ok(());
    }

    let mut sorted = evictable;
    sorted.sort_by(|a, b| b.size.cmp(&a.size));

    let total_size: u64 = sorted.iter().filter_map(|f| f.size).sum();

    for file in &sorted {
        let size_str = file
            .size
            .map(ui::format_size)
            .unwrap_or_else(|| "?".to_string());
        let rel_path = file.path.strip_prefix(&target_path).unwrap_or(&file.path);
        println!("  {} {}", size_str.yellow().bold(), rel_path.display());
    }

    println!();
    ui::kv("Files found", &sorted.len().to_string());
    ui::kv("Total size", &ui::format_size(total_size));
    println!();
    ui::dim("Run 'bossa icloud evict <path>' to free space");

    Ok(())
}

// ============================================================================
// Evict Command
// ============================================================================

fn evict(path: &str, recursive: bool, min_size: Option<&str>, dry_run: bool) -> Result<()> {
    let client = Client::new().context("Failed to initialize iCloud client")?;
    let target_path = expand_path(path);

    if !client.is_in_icloud(&target_path) {
        anyhow::bail!("Path is not in iCloud Drive: {}", target_path.display());
    }

    let min_bytes = min_size
        .map(|s| {
            ui::parse_size(s).map_err(|e| anyhow::anyhow!("Invalid size '{}': {}", s, e))
        })
        .transpose()?;

    if dry_run {
        ui::header("Evict (Dry Run)");
    } else {
        ui::header("Evict");
    }
    ui::dim("Removing local copies, files stay in iCloud");
    println!();

    if target_path.is_file() {
        evict_single_file(&client, &target_path, min_bytes, dry_run)
    } else if target_path.is_dir() {
        if !recursive {
            anyhow::bail!(
                "Cannot evict directory without --recursive flag: {}",
                target_path.display()
            );
        }
        evict_directory(&client, &target_path, min_bytes, dry_run)
    } else {
        anyhow::bail!("Path not found: {}", target_path.display());
    }
}

fn evict_single_file(
    client: &Client,
    path: &Path,
    min_bytes: Option<u64>,
    dry_run: bool,
) -> Result<()> {
    let status = client.status(path)?;

    if status.state.is_cloud_only() {
        ui::info("Already evicted (cloud-only)");
        return Ok(());
    }

    if let Some(min) = min_bytes {
        if status.size.map(|s| s < min).unwrap_or(true) {
            ui::info(&format!("Skipping: file smaller than {}", ui::format_size(min)));
            return Ok(());
        }
    }

    let size_str = status.size.map(ui::format_size).unwrap_or_else(|| "?".to_string());

    if dry_run {
        println!("  Would evict: {} ({})", path.display(), size_str);
    } else {
        client.evict(path)?;
        ui::success(&format!("Evicted: {} ({})", path.display(), size_str));
    }

    Ok(())
}

fn evict_directory(
    client: &Client,
    path: &Path,
    min_bytes: Option<u64>,
    dry_run: bool,
) -> Result<()> {
    let files = collect_files(client, path, |status| {
        if !status.state.is_local() {
            return false;
        }
        if let Some(min) = min_bytes {
            status.size.map(|s| s >= min).unwrap_or(false)
        } else {
            true
        }
    })?;

    if files.is_empty() {
        ui::info("No evictable files found");
        return Ok(());
    }

    let total_size: u64 = files.iter().filter_map(|f| f.size).sum();

    println!(
        "  Found {} files ({}) to evict",
        files.len(),
        ui::format_size(total_size)
    );
    println!();

    if dry_run {
        for file in &files {
            let size_str = file.size.map(ui::format_size).unwrap_or_else(|| "?".to_string());
            let rel_path = file.path.strip_prefix(path).unwrap_or(&file.path);
            println!("  Would evict: {} ({})", rel_path.display(), size_str);
        }
        println!();
        ui::dim("(dry run - no files evicted)");
    } else {
        let result = process_files_with_progress(&files, path, |file| client.evict(&file.path));

        println!();
        if result.failed == 0 {
            ui::success(&format!(
                "Evicted {} files, freed {}",
                result.succeeded,
                ui::format_size(total_size)
            ));
        } else {
            ui::warn(&format!(
                "Evicted {}/{} files ({} failed)",
                result.succeeded,
                result.succeeded + result.failed,
                result.failed
            ));
        }
    }

    Ok(())
}

// ============================================================================
// Download Command
// ============================================================================

fn download(path: &str, recursive: bool) -> Result<()> {
    let client = Client::new().context("Failed to initialize iCloud client")?;
    let target_path = expand_path(path);

    if !client.is_in_icloud(&target_path) {
        anyhow::bail!("Path is not in iCloud Drive: {}", target_path.display());
    }

    ui::header("Download from iCloud");
    println!();

    if target_path.is_file() {
        download_single_file(&client, &target_path)
    } else if target_path.is_dir() {
        if !recursive {
            anyhow::bail!(
                "Cannot download directory without --recursive flag: {}",
                target_path.display()
            );
        }
        download_directory(&client, &target_path)
    } else {
        anyhow::bail!("Path not found: {}", target_path.display());
    }
}

fn download_single_file(client: &Client, path: &Path) -> Result<()> {
    let status = client.status(path)?;

    if status.state.is_local() {
        ui::info("Already downloaded (local)");
        return Ok(());
    }

    client.download(path)?;
    ui::success(&format!("Downloaded: {}", path.display()));

    Ok(())
}

fn download_directory(client: &Client, path: &Path) -> Result<()> {
    let files = collect_files(client, path, |status| status.state.is_cloud_only())?;

    if files.is_empty() {
        ui::info("No cloud-only files found (all already downloaded)");
        return Ok(());
    }

    println!("  Found {} cloud-only files to download", files.len());
    println!();

    let result = process_files_with_progress(&files, path, |file| client.download(&file.path));

    println!();
    if result.failed == 0 {
        ui::success(&format!("Downloaded {} files", result.succeeded));
    } else {
        ui::warn(&format!(
            "Downloaded {}/{} files ({} failed)",
            result.succeeded,
            result.succeeded + result.failed,
            result.failed
        ));
    }

    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

/// Expand ~ in paths
fn expand_path(path: &str) -> PathBuf {
    PathBuf::from(shellexpand::tilde(path).as_ref())
}

/// Print file status details
fn print_file_status(file: &FileStatus) {
    let state_str = format_state(&file.state);
    let size_str = file.size.map(ui::format_size).unwrap_or_else(|| "-".to_string());

    ui::kv("Path", &file.path.display().to_string());
    ui::kv("Status", &state_str);
    ui::kv("Size", &size_str);
    ui::kv("Type", if file.is_dir { "Directory" } else { "File" });
}

/// Print directory summary
fn print_directory_summary(files: &[FileStatus], path: &Path) {
    let total = files.len();
    let local_count = files.iter().filter(|f| f.state.is_local()).count();
    let cloud_count = files.iter().filter(|f| f.state.is_cloud_only()).count();
    let syncing_count = files.iter().filter(|f| f.state.is_syncing()).count();

    let local_size: u64 = files
        .iter()
        .filter(|f| f.state.is_local())
        .filter_map(|f| f.size)
        .sum();

    ui::kv("Path", &path.display().to_string());
    ui::kv("Total files", &total.to_string());
    println!();
    ui::kv(
        "Local",
        &format!("{} ({})", local_count, ui::format_size(local_size)),
    );
    ui::kv("Cloud-only", &cloud_count.to_string());
    if syncing_count > 0 {
        ui::kv("Syncing", &syncing_count.to_string());
    }
}

/// Print single file line
fn print_file_line(file: &FileStatus, base_path: &Path) {
    let state_icon = match file.state {
        DownloadState::Local => "●".green(),
        DownloadState::Cloud => "○".blue(),
        DownloadState::Downloading { percent } => format!("↓{}%", percent).cyan().bold(),
        DownloadState::Uploading { percent } => format!("↑{}%", percent).yellow().bold(),
        DownloadState::Unknown => "?".dimmed(),
    };

    let size_str = file
        .size
        .map(|s| format!("{:>8}", ui::format_size(s)))
        .unwrap_or_else(|| "       -".to_string());

    let rel_path = file.path.strip_prefix(base_path).unwrap_or(&file.path);
    let name = if file.is_dir {
        format!("{}/", rel_path.display()).bold()
    } else {
        rel_path.display().to_string().normal()
    };

    println!("  {} {} {}", state_icon, size_str.dimmed(), name);
}

/// Format download state as string
fn format_state(state: &DownloadState) -> String {
    match state {
        DownloadState::Local => "Local (downloaded)".green().to_string(),
        DownloadState::Cloud => "Cloud-only (evicted)".blue().to_string(),
        DownloadState::Downloading { percent } => {
            format!("Downloading ({}%)", percent).cyan().to_string()
        }
        DownloadState::Uploading { percent } => {
            format!("Uploading ({}%)", percent).yellow().to_string()
        }
        DownloadState::Unknown => "Unknown".dimmed().to_string(),
    }
}

/// Collect files recursively with a filter predicate
fn collect_files<F>(client: &Client, path: &Path, filter: F) -> Result<Vec<FileStatus>>
where
    F: Fn(&FileStatus) -> bool,
{
    use walkdir::WalkDir;

    let mut results = Vec::new();
    let mut errors = Vec::new();

    for entry in WalkDir::new(path) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                errors.push(format!("{}", e));
                continue;
            }
        };

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
            if filter(&status) {
                results.push(status);
            }
        }
    }

    // Report errors but don't fail
    if !errors.is_empty() {
        ui::warn(&format!("Skipped {} paths due to errors", errors.len()));
    }

    Ok(results)
}

/// Result of processing files
struct ProcessResult {
    succeeded: usize,
    failed: usize,
}

/// Process files with progress bar
fn process_files_with_progress<F>(
    files: &[FileStatus],
    base_path: &Path,
    operation: F,
) -> ProcessResult
where
    F: Fn(&FileStatus) -> icloud::Result<()>,
{
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    let mut succeeded = 0;
    let mut failed = 0;

    for file in files {
        let rel_path = file.path.strip_prefix(base_path).unwrap_or(&file.path);
        pb.set_message(ui::truncate_path(&rel_path.display().to_string(), 30));

        match operation(file) {
            Ok(()) => {
                succeeded += 1;
            }
            Err(e) => {
                failed += 1;
                pb.suspend(|| {
                    println!("  {} {} ({})", "✗".red(), rel_path.display(), e);
                });
            }
        }

        pb.inc(1);
    }

    pb.finish_and_clear();

    // Print successes after progress bar is done
    if failed == 0 {
        for file in files {
            let rel_path = file.path.strip_prefix(base_path).unwrap_or(&file.path);
            println!("  {} {}", "✓".green(), rel_path.display());
        }
    }

    ProcessResult { succeeded, failed }
}
