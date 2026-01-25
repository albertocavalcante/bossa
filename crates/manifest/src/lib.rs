//! # Manifest
//!
//! Content hashing and duplicate detection for storage volumes.
//!
//! This crate provides functionality to:
//! - Scan directories and compute BLAKE3 hashes for all files
//! - Store file metadata in a SQLite database
//! - Find duplicate files by content hash
//! - Track storage statistics and wasted space
//!
//! ## Example
//!
//! ```no_run
//! use manifest::Manifest;
//! use std::path::Path;
//!
//! // Open or create a manifest database
//! let manifest = Manifest::open(Path::new("/path/to/manifest.db"))?;
//!
//! // Scan a directory
//! let base_path = Path::new("/Volumes/MyDrive");
//! let result = manifest.scan(base_path, false, &mut manifest::NoProgress)?;
//!
//! // Get statistics
//! let stats = manifest.stats()?;
//! println!("Files: {}, Duplicates: {}", stats.file_count, stats.duplicates.duplicate_groups);
//!
//! // Find duplicates larger than 1MB
//! let duplicates = manifest.find_duplicates(1024 * 1024)?;
//! for group in duplicates {
//!     println!("{} copies of {} bytes each", group.count, group.size_each);
//! }
//! # Ok::<(), manifest::Error>(())
//! ```

mod error;
mod types;

pub use error::{Error, Result};
pub use types::{
    CrossManifestDuplicate, DuplicateGroup, DuplicateStats, ManifestStats, NoProgress,
    ProgressCallback, ScanProgress, ScanResult,
};

use blake3::Hasher;
use rusqlite::{params, Connection};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use walkdir::WalkDir;

/// A content manifest database for tracking file hashes
pub struct Manifest {
    conn: Connection,
}

impl Manifest {
    /// Open or create a manifest database at the given path
    ///
    /// Creates the database file and necessary tables if they don't exist.
    pub fn open(db_path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;

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
            ",
        )?;

        Ok(Self { conn })
    }

    /// Scan a directory and update the manifest
    ///
    /// # Arguments
    /// * `base_path` - The root directory to scan
    /// * `force` - If true, re-hash all files even if unchanged
    /// * `progress` - Callback for progress updates
    ///
    /// # Returns
    /// A `ScanResult` with statistics about the scan
    pub fn scan<P: ProgressCallback>(
        &self,
        base_path: &Path,
        force: bool,
        progress: &mut P,
    ) -> Result<ScanResult> {
        if !base_path.exists() {
            return Err(Error::PathNotFound(base_path.to_path_buf()));
        }

        // First pass: count files
        let mut file_count = 0u64;
        let mut total_size = 0u64;

        for entry in WalkDir::new(base_path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                file_count += 1;
                if let Ok(meta) = entry.metadata() {
                    total_size += meta.len();
                }
            }
        }

        progress.on_start(file_count, total_size);

        // Even if no files to scan, we still need to prune missing entries
        if file_count == 0 {
            let pruned = self.prune_missing(base_path)?;
            let result = ScanResult {
                pruned,
                ..Default::default()
            };
            progress.on_complete(&result);
            return Ok(result);
        }

        // Second pass: hash files
        let mut hashed = 0u64;
        let mut errors = 0u64;

        for entry in WalkDir::new(base_path).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }

            let file_path = entry.path();
            let rel_path = file_path.strip_prefix(base_path).unwrap_or(file_path);
            let rel_path_str = rel_path.to_string_lossy().to_string();

            // Get metadata
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => {
                    errors += 1;
                    progress.on_file_complete(false);
                    continue;
                }
            };

            let size = meta.len();
            let mtime = meta
                .modified()
                .map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64
                })
                .unwrap_or(0);

            progress.on_file(rel_path, size);

            // Skip if already scanned and not force (could check mtime)
            let _ = force; // TODO: implement incremental scanning

            // Hash the file
            match hash_file(file_path) {
                Ok(hash) => {
                    self.upsert(&rel_path_str, &hash, size, mtime)?;
                    hashed += 1;
                    progress.on_file_complete(true);
                }
                Err(_) => {
                    errors += 1;
                    progress.on_file_complete(false);
                }
            }
        }

        // Prune missing files
        let pruned = self.prune_missing(base_path)?;

        // Get duplicate stats
        let duplicates = self.duplicate_stats()?;

        let result = ScanResult {
            hashed,
            errors,
            pruned,
            duplicates,
        };

        progress.on_complete(&result);
        Ok(result)
    }

    /// Get manifest statistics
    pub fn stats(&self) -> Result<ManifestStats> {
        let file_count = self.file_count()?;
        let total_size = self.total_size()?;
        let duplicates = self.duplicate_stats()?;

        Ok(ManifestStats {
            file_count,
            total_size,
            duplicates,
        })
    }

    /// Find duplicate file groups
    ///
    /// # Arguments
    /// * `min_size` - Minimum file size to consider (in bytes)
    ///
    /// # Returns
    /// A list of `DuplicateGroup`s, sorted by total wasted space (descending)
    pub fn find_duplicates(&self, min_size: u64) -> Result<Vec<DuplicateGroup>> {
        let mut stmt = self.conn.prepare(
            "SELECT hash, GROUP_CONCAT(path, '|'), SUM(size) as total_size, COUNT(*) as count
             FROM files
             WHERE size >= ?1
             GROUP BY hash
             HAVING count > 1
             ORDER BY total_size DESC",
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
                size_each: (total_size / count) as u64,
                count: count as usize,
            })
        })?;

        let mut result = Vec::new();
        for group in groups {
            result.push(group?);
        }
        Ok(result)
    }

    /// Delete a file entry from the manifest
    ///
    /// This only removes the entry from the database, not the actual file.
    pub fn delete_entry(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", [path])?;
        Ok(())
    }

    /// Get total file count
    pub fn file_count(&self) -> Result<u64> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    /// Get total size of all files
    pub fn total_size(&self) -> Result<u64> {
        let size: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(size), 0) FROM files",
            [],
            |row| row.get(0),
        )?;
        Ok(size as u64)
    }

    /// Find files that exist in both this manifest and another
    ///
    /// Uses SQL ATTACH DATABASE for efficient cross-manifest comparison.
    ///
    /// # Arguments
    /// * `other_db_path` - Path to the other manifest database
    /// * `min_size` - Minimum file size to consider (in bytes)
    ///
    /// # Returns
    /// A list of `CrossManifestDuplicate`s, sorted by size (descending)
    ///
    /// # Errors
    /// Returns an error if `other_db_path` does not exist or cannot be attached.
    pub fn compare_with(
        &self,
        other_db_path: &Path,
        min_size: u64,
    ) -> Result<Vec<CrossManifestDuplicate>> {
        // Validate the other database exists
        if !other_db_path.exists() {
            return Err(Error::PathNotFound(other_db_path.to_path_buf()));
        }

        // Attach the other database
        self.conn.execute(
            "ATTACH DATABASE ?1 AS other",
            [other_db_path.to_string_lossy().as_ref()],
        )?;

        // Find matching hashes across both manifests
        let mut stmt = self.conn.prepare(
            "SELECT m.hash, m.size, m.path, o.path
             FROM files m
             INNER JOIN other.files o ON m.hash = o.hash
             WHERE m.size >= ?1
             ORDER BY m.size DESC",
        )?;

        let duplicates = stmt
            .query_map([min_size as i64], |row| {
                Ok(CrossManifestDuplicate {
                    hash: row.get(0)?,
                    size: row.get::<_, i64>(1)? as u64,
                    source_path: row.get(2)?,
                    other_path: row.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Detach the other database
        self.conn.execute("DETACH DATABASE other", [])?;

        Ok(duplicates)
    }

    /// Get duplicate statistics
    pub fn duplicate_stats(&self) -> Result<DuplicateStats> {
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

    /// Remove entries for files that no longer exist
    fn prune_missing(&self, base_path: &Path) -> Result<u64> {
        let mut stmt = self.conn.prepare("SELECT id, path FROM files")?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let mut removed = 0;
        for (id, path) in rows {
            let full_path = base_path.join(&path);
            if !full_path.exists() {
                self.conn
                    .execute("DELETE FROM files WHERE id = ?1", [id])?;
                removed += 1;
            }
        }

        Ok(removed)
    }
}

/// Hash a file using BLAKE3
fn hash_file(path: &Path) -> std::io::Result<String> {
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

// ============================================================================
// Utility functions
// ============================================================================

/// Convert a path to a manifest name
///
/// Uses the last path component, replacing invalid characters.
pub fn path_to_name(path: &Path) -> String {
    path.file_name()
        .or_else(|| path.components().last().map(|c| c.as_os_str()))
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "default".to_string())
        .replace(['/', '\\', ':'], "_")
}

/// Format bytes as human-readable size
pub fn format_size(bytes: u64) -> String {
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_open_creates_db() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");

        let manifest = Manifest::open(&db_path).unwrap();
        assert_eq!(manifest.file_count().unwrap(), 0);
        assert!(db_path.exists());
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("manifest.db");
        let scan_dir = tmp.path().join("data");
        std::fs::create_dir(&scan_dir).unwrap();

        let manifest = Manifest::open(&db_path).unwrap();
        let result = manifest.scan(&scan_dir, false, &mut NoProgress).unwrap();

        assert_eq!(result.hashed, 0);
        assert_eq!(result.errors, 0);
    }

    #[test]
    fn test_scan_with_files() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("manifest.db");
        let scan_dir = tmp.path().join("data");
        std::fs::create_dir(&scan_dir).unwrap();

        // Create test files
        std::fs::write(scan_dir.join("a.txt"), "hello").unwrap();
        std::fs::write(scan_dir.join("b.txt"), "world").unwrap();
        std::fs::write(scan_dir.join("c.txt"), "hello").unwrap(); // duplicate of a.txt

        let manifest = Manifest::open(&db_path).unwrap();
        let result = manifest.scan(&scan_dir, false, &mut NoProgress).unwrap();

        assert_eq!(result.hashed, 3);
        assert_eq!(result.errors, 0);
        assert_eq!(result.duplicates.duplicate_groups, 1);
        assert_eq!(result.duplicates.duplicate_files, 2);
    }

    #[test]
    fn test_find_duplicates() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("manifest.db");
        let scan_dir = tmp.path().join("data");
        std::fs::create_dir(&scan_dir).unwrap();

        // Create duplicate files
        std::fs::write(scan_dir.join("a.txt"), "duplicate content").unwrap();
        std::fs::write(scan_dir.join("b.txt"), "duplicate content").unwrap();
        std::fs::write(scan_dir.join("unique.txt"), "unique").unwrap();

        let manifest = Manifest::open(&db_path).unwrap();
        manifest.scan(&scan_dir, false, &mut NoProgress).unwrap();

        let dups = manifest.find_duplicates(0).unwrap();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].count, 2);
        assert_eq!(dups[0].paths.len(), 2);
    }

    #[test]
    fn test_path_to_name() {
        assert_eq!(path_to_name(Path::new("/Volumes/T9")), "T9");
        assert_eq!(path_to_name(Path::new("/home/user/data")), "data");
        // Root path "/" has component "/" which becomes "_" after sanitization
        assert_eq!(path_to_name(Path::new("/")), "_");
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_size(1024 * 1024 * 1024 * 1024), "1.00 TB");
    }

    #[test]
    fn test_prune_missing() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("manifest.db");
        let scan_dir = tmp.path().join("data");
        std::fs::create_dir(&scan_dir).unwrap();

        // Create and scan a file
        let file_path = scan_dir.join("temp.txt");
        std::fs::write(&file_path, "temporary").unwrap();

        let manifest = Manifest::open(&db_path).unwrap();
        manifest.scan(&scan_dir, false, &mut NoProgress).unwrap();
        assert_eq!(manifest.file_count().unwrap(), 1);

        // Delete the file and re-scan
        std::fs::remove_file(&file_path).unwrap();
        let result = manifest.scan(&scan_dir, false, &mut NoProgress).unwrap();

        assert_eq!(result.pruned, 1);
        assert_eq!(manifest.file_count().unwrap(), 0);
    }

    #[test]
    fn test_compare_with() {
        let tmp = TempDir::new().unwrap();

        // Create two separate scan directories
        let dir_a = tmp.path().join("storage_a");
        let dir_b = tmp.path().join("storage_b");
        std::fs::create_dir(&dir_a).unwrap();
        std::fs::create_dir(&dir_b).unwrap();

        // Create files - "shared content" exists in both, others are unique
        std::fs::write(dir_a.join("shared.txt"), "shared content").unwrap();
        std::fs::write(dir_a.join("unique_a.txt"), "only in A").unwrap();

        std::fs::write(dir_b.join("also_shared.txt"), "shared content").unwrap();
        std::fs::write(dir_b.join("unique_b.txt"), "only in B").unwrap();

        // Create and populate manifests
        let db_a = tmp.path().join("manifest_a.db");
        let db_b = tmp.path().join("manifest_b.db");

        let manifest_a = Manifest::open(&db_a).unwrap();
        manifest_a.scan(&dir_a, false, &mut NoProgress).unwrap();

        let manifest_b = Manifest::open(&db_b).unwrap();
        manifest_b.scan(&dir_b, false, &mut NoProgress).unwrap();

        // Compare manifests
        let cross_dups = manifest_a.compare_with(&db_b, 0).unwrap();

        assert_eq!(cross_dups.len(), 1);
        assert_eq!(cross_dups[0].source_path, "shared.txt");
        assert_eq!(cross_dups[0].other_path, "also_shared.txt");
        assert_eq!(cross_dups[0].size, 14); // "shared content".len()
    }

    #[test]
    fn test_compare_with_min_size() {
        let tmp = TempDir::new().unwrap();

        let dir_a = tmp.path().join("storage_a");
        let dir_b = tmp.path().join("storage_b");
        std::fs::create_dir(&dir_a).unwrap();
        std::fs::create_dir(&dir_b).unwrap();

        // Create small shared file (5 bytes) and large shared file (100 bytes)
        std::fs::write(dir_a.join("small.txt"), "small").unwrap();
        std::fs::write(dir_a.join("large.txt"), "x".repeat(100)).unwrap();

        std::fs::write(dir_b.join("small.txt"), "small").unwrap();
        std::fs::write(dir_b.join("large.txt"), "x".repeat(100)).unwrap();

        let db_a = tmp.path().join("manifest_a.db");
        let db_b = tmp.path().join("manifest_b.db");

        let manifest_a = Manifest::open(&db_a).unwrap();
        manifest_a.scan(&dir_a, false, &mut NoProgress).unwrap();

        let manifest_b = Manifest::open(&db_b).unwrap();
        manifest_b.scan(&dir_b, false, &mut NoProgress).unwrap();

        // With min_size=50, only large file should match
        let cross_dups = manifest_a.compare_with(&db_b, 50).unwrap();

        assert_eq!(cross_dups.len(), 1);
        assert_eq!(cross_dups[0].source_path, "large.txt");
        assert_eq!(cross_dups[0].size, 100);
    }

    #[test]
    fn test_compare_with_missing_database() {
        let tmp = TempDir::new().unwrap();

        // Create manifest A
        let dir_a = tmp.path().join("storage_a");
        std::fs::create_dir(&dir_a).unwrap();
        std::fs::write(dir_a.join("file.txt"), "content").unwrap();

        let db_a = tmp.path().join("manifest_a.db");
        let manifest_a = Manifest::open(&db_a).unwrap();
        manifest_a.scan(&dir_a, false, &mut NoProgress).unwrap();

        // Try to compare with non-existent database
        let missing_db = tmp.path().join("does_not_exist.db");
        let result = manifest_a.compare_with(&missing_db, 0);

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::PathNotFound(path) => {
                assert_eq!(path, missing_db);
            }
            other => panic!("Expected PathNotFound error, got: {:?}", other),
        }
    }
}
