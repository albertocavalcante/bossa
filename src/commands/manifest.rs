//! Manifest - Content hashing and duplicate detection for storage volumes
//!
//! Commands:
//! - scan: Walk filesystem, hash files, store in SQLite manifest
//! - stats: Show size, file count, duplicates summary
//! - duplicates: List duplicate file sets

use anyhow::Result;
use blake3::Hasher;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::{params, Connection};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::config;
use crate::ui;

// ============================================================================
// Command Enum
// ============================================================================

#[derive(Debug)]
pub enum ManifestCommand {
    Scan { path: String, force: bool },
    Stats { path: String },
    Duplicates { path: String, min_size: u64, delete: bool },
}

pub fn run(cmd: ManifestCommand) -> Result<()> {
    match cmd {
        ManifestCommand::Scan { path, force } => scan(&path, force),
        ManifestCommand::Stats { path } => stats(&path),
        ManifestCommand::Duplicates { path, min_size, delete } => {
            duplicates(&path, min_size, delete)
        }
    }
}

// ============================================================================
// Manifest Database
// ============================================================================

struct Manifest {
    conn: Connection,
}

impl Manifest {
    /// Open or create manifest database
    fn open(name: &str) -> Result<Self> {
        let manifest_dir = config::config_dir()?.join("manifests");
        fs::create_dir_all(&manifest_dir)?;

        let db_path = manifest_dir.join(format!("{}.db", name));
        let conn = Connection::open(&db_path)?;

        // Create tables
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                hash TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                scanned_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_hash ON files(hash);
            CREATE INDEX IF NOT EXISTS idx_size ON files(size);
            "
        )?;

        Ok(Self { conn })
    }

    /// Insert or update a file entry
    fn upsert(&self, path: &str, hash: &str, size: u64, mtime: i64) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT INTO files (path, hash, size, mtime, scanned_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
                hash = excluded.hash,
                size = excluded.size,
                mtime = excluded.mtime,
                scanned_at = excluded.scanned_at",
            params![path, hash, size, mtime, now],
        )?;
        Ok(())
    }

    /// Get total file count
    fn file_count(&self) -> Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Get total size
    fn total_size(&self) -> Result<u64> {
        let size: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(size), 0) FROM files",
            [],
            |row| row.get(0),
        )?;
        Ok(size as u64)
    }

    /// Get duplicate groups (hash -> paths)
    fn find_duplicates(&self, min_size: u64) -> Result<Vec<DuplicateGroup>> {
        let mut stmt = self.conn.prepare(
            "SELECT hash, GROUP_CONCAT(path, '|'), SUM(size) as total_size, COUNT(*) as count
             FROM files
             WHERE size >= ?1
             GROUP BY hash
             HAVING count > 1
             ORDER BY total_size DESC"
        )?;

        let groups = stmt.query_map([min_size as i64], |row| {
            let hash: String = row.get(0)?;
            let paths_str: String = row.get(1)?;
            let total_size: i64 = row.get(2)?;
            let count: i64 = row.get(3)?;

            let paths: Vec<String> = paths_str.split('|').map(|s| s.to_string()).collect();

            Ok(DuplicateGroup {
                hash,
                paths,
                size_each: total_size / count,
                count: count as usize,
            })
        })?;

        let mut result = Vec::new();
        for group in groups {
            result.push(group?);
        }
        Ok(result)
    }

    /// Get duplicate stats
    fn duplicate_stats(&self) -> Result<DuplicateStats> {
        // Count files with duplicates
        let dup_file_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE hash IN (
                SELECT hash FROM files GROUP BY hash HAVING COUNT(*) > 1
            )",
            [],
            |row| row.get(0),
        )?;

        // Count unique hashes with duplicates
        let dup_hash_count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT hash) FROM (
                SELECT hash FROM files GROUP BY hash HAVING COUNT(*) > 1
            )",
            [],
            |row| row.get(0),
        )?;

        // Calculate wasted space (total size - size of one copy per hash)
        let wasted: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(wasted), 0) FROM (
                SELECT hash, (COUNT(*) - 1) * size as wasted
                FROM files
                GROUP BY hash
                HAVING COUNT(*) > 1
            )",
            [],
            |row| row.get(0),
        )?;

        Ok(DuplicateStats {
            duplicate_files: dup_file_count as u64,
            duplicate_groups: dup_hash_count as u64,
            wasted_space: wasted as u64,
        })
    }

    /// Remove entries for files that no longer exist
    fn prune_missing(&self, base_path: &Path) -> Result<u64> {
        let mut stmt = self.conn.prepare("SELECT id, path FROM files")?;
        let rows: Vec<(i64, String)> = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?.filter_map(|r| r.ok()).collect();

        let mut removed = 0;
        for (id, path) in rows {
            let full_path = base_path.join(&path);
            if !full_path.exists() {
                self.conn.execute("DELETE FROM files WHERE id = ?1", [id])?;
                removed += 1;
            }
        }

        Ok(removed)
    }

    /// Delete a file entry
    fn delete_entry(&self, path: &str) -> Result<()> {
        self.conn.execute("DELETE FROM files WHERE path = ?1", [path])?;
        Ok(())
    }
}

#[derive(Debug)]
struct DuplicateGroup {
    #[allow(dead_code)]
    hash: String,
    paths: Vec<String>,
    size_each: i64,
    count: usize,
}

#[derive(Debug)]
struct DuplicateStats {
    duplicate_files: u64,
    duplicate_groups: u64,
    wasted_space: u64,
}

// ============================================================================
// Scan Command
// ============================================================================

fn scan(path_str: &str, force: bool) -> Result<()> {
    let path = PathBuf::from(shellexpand::tilde(path_str).as_ref());
    let name = path_to_name(&path);

    ui::header(&format!("Scanning: {}", path.display()));

    if !path.exists() {
        anyhow::bail!("Path does not exist: {}", path.display());
    }

    let manifest = Manifest::open(&name)?;

    // First pass: count files
    ui::info("Counting files...");
    let mut file_count = 0u64;
    let mut total_size = 0u64;

    for entry in WalkDir::new(&path).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            file_count += 1;
            if let Ok(meta) = entry.metadata() {
                total_size += meta.len();
            }
        }
    }

    ui::kv("Files found", &file_count.to_string());
    ui::kv("Total size", &format_size(total_size));
    println!();

    if file_count == 0 {
        ui::info("No files to scan.");
        return Ok(());
    }

    // Second pass: hash files
    let pb = ProgressBar::new(file_count);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    let mut hashed = 0u64;
    let mut errors = 0u64;

    for entry in WalkDir::new(&path).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        let file_path = entry.path();
        let rel_path = file_path.strip_prefix(&path).unwrap_or(file_path);
        let rel_path_str = rel_path.to_string_lossy().to_string();

        pb.set_message(truncate_path(&rel_path_str, 30));

        // Get metadata
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                errors += 1;
                pb.inc(1);
                continue;
            }
        };

        let size = meta.len();
        let mtime = meta.modified()
            .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64)
            .unwrap_or(0);

        // Skip if already scanned and not force (check mtime)
        if !force {
            // TODO: Could check mtime here to skip unchanged files
        }

        // Hash the file
        match hash_file(file_path) {
            Ok(hash) => {
                manifest.upsert(&rel_path_str, &hash, size, mtime)?;
                hashed += 1;
            }
            Err(_) => {
                errors += 1;
            }
        }

        pb.inc(1);
    }

    pb.finish_and_clear();

    // Prune missing files
    let pruned = manifest.prune_missing(&path)?;

    println!();
    ui::success(&format!("Scan complete: {} files hashed", hashed));
    if errors > 0 {
        ui::warn(&format!("  Errors: {} (could not read)", errors));
    }
    if pruned > 0 {
        ui::dim(&format!("  Pruned: {} (no longer exist)", pruned));
    }

    // Show quick duplicate summary
    let dup_stats = manifest.duplicate_stats()?;
    if dup_stats.duplicate_groups > 0 {
        println!();
        ui::warn(&format!(
            "Found {} duplicate groups ({} files, {} wasted)",
            dup_stats.duplicate_groups,
            dup_stats.duplicate_files,
            format_size(dup_stats.wasted_space)
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
    let name = path_to_name(&path);

    ui::header(&format!("Manifest Stats: {}", path.display()));

    let manifest = Manifest::open(&name)?;

    let file_count = manifest.file_count()?;
    let total_size = manifest.total_size()?;
    let dup_stats = manifest.duplicate_stats()?;

    println!();
    ui::kv("Total files", &file_count.to_string());
    ui::kv("Total size", &format_size(total_size));
    println!();
    ui::kv("Duplicate groups", &dup_stats.duplicate_groups.to_string());
    ui::kv("Duplicate files", &dup_stats.duplicate_files.to_string());
    ui::kv("Wasted space", &format_size(dup_stats.wasted_space));

    if dup_stats.wasted_space > 0 {
        let savings_pct = (dup_stats.wasted_space as f64 / total_size as f64) * 100.0;
        ui::kv("Potential savings", &format!("{:.1}%", savings_pct));
    }

    Ok(())
}

// ============================================================================
// Duplicates Command
// ============================================================================

fn duplicates(path_str: &str, min_size: u64, delete: bool) -> Result<()> {
    let path = PathBuf::from(shellexpand::tilde(path_str).as_ref());
    let name = path_to_name(&path);

    ui::header(&format!("Duplicates: {}", path.display()));

    let manifest = Manifest::open(&name)?;

    let groups = manifest.find_duplicates(min_size)?;

    if groups.is_empty() {
        ui::success("No duplicates found!");
        return Ok(());
    }

    let mut total_wasted: u64 = 0;

    println!();
    for (i, group) in groups.iter().enumerate() {
        let wasted = group.size_each as u64 * (group.count as u64 - 1);
        total_wasted += wasted;

        println!(
            "{}. {} ({} each, {} copies, {} wasted)",
            (i + 1).to_string().bold(),
            format_size(group.size_each as u64).yellow(),
            format_size(group.size_each as u64),
            group.count,
            format_size(wasted).red()
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
    ui::kv("Total wasted space", &format_size(total_wasted));

    if delete {
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

        for group in &groups {
            // Keep first, delete rest
            for file_path in group.paths.iter().skip(1) {
                let full_path = path.join(file_path);
                match fs::remove_file(&full_path) {
                    Ok(()) => {
                        manifest.delete_entry(file_path)?;
                        deleted_count += 1;
                        deleted_size += group.size_each as u64;
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
            format_size(deleted_size)
        ));
    } else {
        ui::dim("Run with --delete to interactively remove duplicates");
    }

    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

/// Hash a file using BLAKE3
fn hash_file(path: &Path) -> Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file); // 1MB buffer
    let mut hasher = Hasher::new();

    let mut buffer = [0u8; 65536]; // 64KB chunks
    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Convert path to manifest name
fn path_to_name(path: &Path) -> String {
    // Use the last component or volume name
    path.file_name()
        .or_else(|| path.components().last().map(|c| c.as_os_str()))
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "default".to_string())
        .replace(['/', '\\', ':'], "_")
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

/// Truncate path for display
fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        format!("...{}", &path[path.len() - max_len + 3..])
    }
}
