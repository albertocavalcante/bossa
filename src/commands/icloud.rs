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
use std::path::PathBuf;

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

    let target_path = match path {
        Some(p) => expand_path(&p),
        None => icloud_root.clone(),
    };

    ui::header("iCloud Drive Status");
    println!();

    if !client.is_in_icloud(&target_path) {
        ui::warn(&format!("Path is not in iCloud Drive: {}", target_path.display()));
        ui::dim(&format!("iCloud root: {}", icloud_root.display()));
        return Ok(());
    }

    if target_path.is_file() {
        // Single file status
        let file_status = client.status(&target_path)?;
        print_file_status(&file_status, &icloud_root);
    } else if target_path.is_dir() {
        // Directory summary
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

    let target_path = match path {
        Some(p) => expand_path(&p),
        None => icloud_root.clone(),
    };

    if !client.is_in_icloud(&target_path) {
        anyhow::bail!("Path is not in iCloud Drive: {}", target_path.display());
    }

    let files = client.list(&target_path)?;

    // Filter based on flags
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

    let target_path = match path {
        Some(p) => expand_path(&p),
        None => icloud_root.clone(),
    };

    let min_size = parse_size(min_size_str)?;

    if !client.is_in_icloud(&target_path) {
        anyhow::bail!("Path is not in iCloud Drive: {}", target_path.display());
    }

    ui::header("Evictable Files");
    ui::dim(&format!(
        "Local files >= {} that could be evicted to free space",
        format_size(min_size)
    ));
    println!();

    let evictable = client.find_evictable(&target_path, min_size)?;

    if evictable.is_empty() {
        ui::success(&format!(
            "No local files >= {} found",
            format_size(min_size)
        ));
        return Ok(());
    }

    // Sort by size descending
    let mut sorted = evictable;
    sorted.sort_by(|a, b| b.size.cmp(&a.size));

    let total_size: u64 = sorted.iter().filter_map(|f| f.size).sum();

    for file in &sorted {
        let size_str = file
            .size
            .map(|s| format_size(s))
            .unwrap_or_else(|| "?".to_string());
        let rel_path = file
            .path
            .strip_prefix(&target_path)
            .unwrap_or(&file.path);
        println!(
            "  {} {}",
            size_str.yellow().bold(),
            rel_path.display()
        );
    }

    println!();
    ui::kv("Files found", &sorted.len().to_string());
    ui::kv("Total size", &format_size(total_size));
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

    let min_bytes = min_size.map(parse_size).transpose()?;

    if dry_run {
        ui::header("Evict (Dry Run)");
    } else {
        ui::header("Evict");
    }
    ui::dim("Removing local copies, files stay in iCloud");
    println!();

    if target_path.is_file() {
        // Single file eviction
        let status = client.status(&target_path)?;

        if status.state.is_cloud_only() {
            ui::info("Already evicted (cloud-only)");
            return Ok(());
        }

        if let Some(min) = min_bytes {
            if status.size.map(|s| s < min).unwrap_or(true) {
                ui::info(&format!(
                    "Skipping: file smaller than {}",
                    format_size(min)
                ));
                return Ok(());
            }
        }

        let size_str = status
            .size
            .map(|s| format_size(s))
            .unwrap_or_else(|| "?".to_string());

        if dry_run {
            println!("  Would evict: {} ({})", target_path.display(), size_str);
        } else {
            client.evict(&target_path)?;
            ui::success(&format!("Evicted: {} ({})", target_path.display(), size_str));
        }
    } else if target_path.is_dir() {
        if !recursive {
            anyhow::bail!(
                "Cannot evict directory without --recursive flag: {}",
                target_path.display()
            );
        }

        // Recursive eviction
        let files = collect_evictable_files(&client, &target_path, min_bytes)?;

        if files.is_empty() {
            ui::info("No evictable files found");
            return Ok(());
        }

        let total_size: u64 = files.iter().filter_map(|f| f.size).sum();

        println!(
            "  Found {} files ({}) to evict",
            files.len(),
            format_size(total_size)
        );
        println!();

        if dry_run {
            for file in &files {
                let size_str = file
                    .size
                    .map(|s| format_size(s))
                    .unwrap_or_else(|| "?".to_string());
                let rel_path = file
                    .path
                    .strip_prefix(&target_path)
                    .unwrap_or(&file.path);
                println!("  Would evict: {} ({})", rel_path.display(), size_str);
            }
            println!();
            ui::dim("(dry run - no files evicted)");
        } else {
            let mut succeeded = 0;
            let mut failed = 0;

            for file in &files {
                match client.evict(&file.path) {
                    Ok(()) => {
                        succeeded += 1;
                        let rel_path = file
                            .path
                            .strip_prefix(&target_path)
                            .unwrap_or(&file.path);
                        println!("  {} {}", "✓".green(), rel_path.display());
                    }
                    Err(e) => {
                        failed += 1;
                        let rel_path = file
                            .path
                            .strip_prefix(&target_path)
                            .unwrap_or(&file.path);
                        println!("  {} {} ({})", "✗".red(), rel_path.display(), e);
                    }
                }
            }

            println!();
            if failed == 0 {
                ui::success(&format!(
                    "Evicted {} files, freed {}",
                    succeeded,
                    format_size(total_size)
                ));
            } else {
                ui::warn(&format!(
                    "Evicted {}/{} files ({} failed)",
                    succeeded,
                    succeeded + failed,
                    failed
                ));
            }
        }
    } else {
        anyhow::bail!("Path not found: {}", target_path.display());
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
        // Single file download
        let status = client.status(&target_path)?;

        if status.state.is_local() {
            ui::info("Already downloaded (local)");
            return Ok(());
        }

        client.download(&target_path)?;
        ui::success(&format!("Downloaded: {}", target_path.display()));
    } else if target_path.is_dir() {
        if !recursive {
            anyhow::bail!(
                "Cannot download directory without --recursive flag: {}",
                target_path.display()
            );
        }

        // Recursive download
        let files = collect_cloud_files(&client, &target_path)?;

        if files.is_empty() {
            ui::info("No cloud-only files found (all already downloaded)");
            return Ok(());
        }

        println!("  Found {} cloud-only files to download", files.len());
        println!();

        let mut succeeded = 0;
        let mut failed = 0;

        for file in &files {
            match client.download(&file.path) {
                Ok(()) => {
                    succeeded += 1;
                    let rel_path = file
                        .path
                        .strip_prefix(&target_path)
                        .unwrap_or(&file.path);
                    println!("  {} {}", "✓".green(), rel_path.display());
                }
                Err(e) => {
                    failed += 1;
                    let rel_path = file
                        .path
                        .strip_prefix(&target_path)
                        .unwrap_or(&file.path);
                    println!("  {} {} ({})", "✗".red(), rel_path.display(), e);
                }
            }
        }

        println!();
        if failed == 0 {
            ui::success(&format!("Downloaded {} files", succeeded));
        } else {
            ui::warn(&format!(
                "Downloaded {}/{} files ({} failed)",
                succeeded,
                succeeded + failed,
                failed
            ));
        }
    } else {
        anyhow::bail!("Path not found: {}", target_path.display());
    }

    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

/// Expand ~ in paths
fn expand_path(path: &str) -> PathBuf {
    let expanded = shellexpand::tilde(path);
    PathBuf::from(expanded.as_ref())
}

/// Parse human-readable size string (e.g., "100MB", "1GB")
fn parse_size(size_str: &str) -> Result<u64> {
    let size_str = size_str.trim().to_uppercase();

    let (num_str, multiplier) = if let Some(num) = size_str.strip_suffix("TB") {
        (num, 1024u64 * 1024 * 1024 * 1024)
    } else if let Some(num) = size_str.strip_suffix("GB") {
        (num, 1024u64 * 1024 * 1024)
    } else if let Some(num) = size_str.strip_suffix("MB") {
        (num, 1024u64 * 1024)
    } else if let Some(num) = size_str.strip_suffix("KB") {
        (num, 1024u64)
    } else if let Some(num) = size_str.strip_suffix('B') {
        (num, 1u64)
    } else {
        // Assume bytes if no suffix
        (size_str.as_str(), 1u64)
    };

    let num: f64 = num_str
        .trim()
        .parse()
        .with_context(|| format!("Invalid size: {}", size_str))?;

    Ok((num * multiplier as f64) as u64)
}

/// Format bytes as human-readable size
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Print file status in detail
fn print_file_status(file: &FileStatus, _icloud_root: &std::path::Path) {
    let state_str = format_state(&file.state);
    let size_str = file
        .size
        .map(|s| format_size(s))
        .unwrap_or_else(|| "-".to_string());

    ui::kv("Path", &file.path.display().to_string());
    ui::kv("Status", &state_str);
    ui::kv("Size", &size_str);
    ui::kv("Type", if file.is_dir { "Directory" } else { "File" });
}

/// Print directory summary
fn print_directory_summary(files: &[FileStatus], path: &std::path::Path) {
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
    ui::kv("Local", &format!("{} ({})", local_count, format_size(local_size)));
    ui::kv("Cloud-only", &cloud_count.to_string());
    if syncing_count > 0 {
        ui::kv("Syncing", &syncing_count.to_string());
    }
}

/// Print single file line
fn print_file_line(file: &FileStatus, base_path: &std::path::Path) {
    let state_icon = match file.state {
        DownloadState::Local => "●".green(),
        DownloadState::Cloud => "○".blue(),
        DownloadState::Downloading { percent } => format!("↓{}%", percent).cyan().bold(),
        DownloadState::Uploading { percent } => format!("↑{}%", percent).yellow().bold(),
        DownloadState::Unknown => "?".dimmed(),
    };

    let size_str = file
        .size
        .map(|s| format!("{:>8}", format_size(s)))
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

/// Collect all evictable files recursively
fn collect_evictable_files(
    client: &Client,
    path: &std::path::Path,
    min_size: Option<u64>,
) -> Result<Vec<FileStatus>> {
    use walkdir::WalkDir;

    let mut results = Vec::new();

    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        // Skip hidden files
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with('.') {
                continue;
            }
        }

        let entry_path = entry.path();

        if let Ok(status) = client.status(entry_path) {
            // Only include local files
            if !status.state.is_local() {
                continue;
            }

            // Apply min_size filter
            if let Some(min) = min_size {
                if status.size.map(|s| s < min).unwrap_or(true) {
                    continue;
                }
            }

            results.push(status);
        }
    }

    Ok(results)
}

/// Collect all cloud-only files recursively
fn collect_cloud_files(client: &Client, path: &std::path::Path) -> Result<Vec<FileStatus>> {
    use walkdir::WalkDir;

    let mut results = Vec::new();

    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        // Skip hidden files
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with('.') {
                continue;
            }
        }

        let entry_path = entry.path();

        if let Ok(status) = client.status(entry_path) {
            if status.state.is_cloud_only() {
                results.push(status);
            }
        }
    }

    Ok(results)
}
