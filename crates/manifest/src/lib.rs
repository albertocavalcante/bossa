//! Root-bound content manifests with atomic scan publication.
//!
//! A successful generation describes a traversal, not a filesystem snapshot.
//! Legacy unversioned databases remain untouched; create a new store and scan.

mod error;
mod native_path;
mod types;

pub use error::{Error, Result};
pub use native_path::NativePath;
pub use types::{
    CrossManifestDuplicate, DuplicateGroup, DuplicateStats, ManifestStats, NoProgress,
    ProgressCallback, ScanProgress, ScanResult,
};

use blake3::Hasher;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SCHEMA: i64 = 2;

/// A versioned manifest; read-only opens do not create or migrate databases.
pub struct Manifest {
    conn: Connection,
    db_path: PathBuf,
    legacy: bool,
}

fn absolute(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path)?;
    NativePath::from_path(&path).to_path_buf()?;
    Ok(path)
}

// SQLite may create sidecar shared-memory files even with READ_ONLY. Refuse
// WAL-mode headers before opening; immutable mode could ignore committed WAL.
fn check_read_only_header(path: &Path) -> Result<()> {
    let mut file = File::open(path)?;
    let mut header = [0u8; 100];
    file.read_exact(&mut header)?;
    if &header[..16] != b"SQLite format 3\0" {
        return Err(Error::Scan("invalid SQLite header".into()));
    }
    if header[18] != 1 || header[19] != 1 {
        return Err(Error::Scan(
            "WAL-mode manifest cannot be inspected without possible sidecar writes".into(),
        ));
    }
    Ok(())
}

fn schema(conn: &Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    match version {
        SCHEMA => Ok(()),
        0 => Err(Error::Legacy),
        n => Err(Error::UnsupportedSchema(n)),
    }
}

impl Manifest {
    /// Open an existing versioned store without creating or migrating it.
    pub fn open(db_path: &Path) -> Result<Self> {
        check_read_only_header(db_path)?;
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != 0 && version != SCHEMA {
            return Err(Error::UnsupportedSchema(version));
        }
        Ok(Self {
            conn,
            db_path: db_path.to_path_buf(),
            legacy: version == 0,
        })
    }

    /// Read a root-bound store without checking whether its root is online.
    pub fn open_existing(db_path: &Path, root: &Path) -> Result<Self> {
        let manifest = Self::open(db_path)?;
        let root = absolute(root)?;
        if !manifest.legacy {
            manifest.check_declared(&root)?;
        }
        Ok(manifest)
    }

    /// Create a fresh store or open an existing versioned store for this root.
    /// Existing unversioned files are never initialized or modified.
    pub fn open_for_root(db_path: &Path, root: &Path) -> Result<Self> {
        let root = absolute(root)?;
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(db_path)
        {
            Ok(file) => {
                drop(file);
                let conn = Connection::open(db_path)?;
                conn.execute_batch("PRAGMA synchronous=FULL; BEGIN IMMEDIATE;
                    CREATE TABLE meta (declared BLOB NOT NULL, canonical BLOB, root_dev INTEGER, root_ino INTEGER, generation INTEGER NOT NULL, stale INTEGER NOT NULL, last_scanned_at INTEGER);
                    CREATE TABLE files (path BLOB PRIMARY KEY, hash TEXT NOT NULL, size INTEGER NOT NULL, mtime_sec INTEGER NOT NULL, mtime_nsec INTEGER NOT NULL);
                    CREATE INDEX files_hash ON files(hash, size);
                    PRAGMA user_version=2;")?;
                conn.execute(
                    "INSERT INTO meta VALUES (?1, NULL, NULL, NULL, 0, 1, NULL)",
                    [NativePath::from_path(&root).to_blob()],
                )?;
                conn.execute_batch("COMMIT;")?;
                Ok(Self {
                    conn,
                    db_path: db_path.to_path_buf(),
                    legacy: false,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let checked = Self::open_existing(db_path, &root)?;
                if checked.legacy {
                    return Err(Error::Legacy);
                }
                drop(checked);
                let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
                schema(&conn)?;
                let manifest = Self {
                    conn,
                    db_path: db_path.to_path_buf(),
                    legacy: false,
                };
                manifest.check_declared(&root)?;
                Ok(manifest)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn check_declared(&self, root: &Path) -> Result<()> {
        let stored: Vec<u8> = self
            .conn
            .query_row("SELECT declared FROM meta", [], |r| r.get(0))?;
        if stored != NativePath::from_path(root).to_blob() {
            return Err(Error::RootMismatch);
        }
        Ok(())
    }

    /// Whether this is a legacy, unqualified read-only store.
    pub fn is_legacy(&self) -> bool {
        self.legacy
    }

    fn require_fresh(&self) -> Result<()> {
        if self.legacy {
            return Err(Error::Legacy);
        }
        let (generation, stale): (i64, i64) =
            self.conn
                .query_row("SELECT generation, stale FROM meta", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        if generation == 0 || stale != 0 {
            Err(Error::Stale)
        } else {
            Ok(())
        }
    }

    /// Publish a complete generation atomically. A failed traversal, hash or
    /// commit retains the previous complete generation. Cached mtimes are not a snapshot.
    pub fn scan<P: ProgressCallback>(
        &self,
        base_path: &Path,
        force: bool,
        progress: &mut P,
    ) -> Result<ScanResult> {
        let declared = absolute(base_path)?;
        self.check_declared(&declared)?;
        let canonical = fs::canonicalize(&declared)?;
        let root_before = root_observation(&canonical)?;
        let tx = self.conn.unchecked_transaction()?;
        // Acquire the write lock before observing the current binding.
        tx.execute("UPDATE meta SET stale=stale", [])?;
        let old: (Option<Vec<u8>>, Option<i64>, Option<i64>) =
            tx.query_row("SELECT canonical, root_dev, root_ino FROM meta", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        if let Some(old_root) = &old.0
            && *old_root != NativePath::from_path(&canonical).to_blob()
        {
            return Err(Error::RootMismatch);
        }
        let can_cache = !force
            && old.0.is_some()
            && root_before.0.is_some()
            && root_before.1.is_some()
            && old.1 == root_before.0
            && old.2 == root_before.1;
        tx.execute_batch("CREATE TEMP TABLE next_files (path BLOB PRIMARY KEY, hash TEXT NOT NULL, size INTEGER NOT NULL, mtime_sec INTEGER NOT NULL, mtime_nsec INTEGER NOT NULL);")?;
        progress.on_start(0, 0);
        let mut hashed = 0u64;
        for entry in WalkDir::new(&canonical).follow_links(false) {
            if progress.is_cancelled() {
                return Err(Error::Scan("scan cancelled".into()));
            }
            let entry = entry.map_err(|e| {
                Error::Scan(format!(
                    "traversal failed: {:?}",
                    e.io_error().map(std::io::Error::kind)
                ))
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(&canonical)
                .map_err(|_| Error::RootMismatch)?;
            let identity = NativePath::from_path(relative);
            identity.relative()?;
            let meta = fs::symlink_metadata(path)?;
            if !meta.file_type().is_file() {
                return Err(Error::Scan("file changed during traversal".into()));
            }
            let modified = meta
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::Scan("file timestamp predates epoch".into()))?;
            let size =
                i64::try_from(meta.len()).map_err(|_| Error::Scan("file too large".into()))?;
            let seconds = i64::try_from(modified.as_secs())
                .map_err(|_| Error::Scan("timestamp out of range".into()))?;
            let nanos = i64::from(modified.subsec_nanos());
            let key = identity.to_blob();
            progress.on_file(relative, meta.len());
            let cached: Option<String> = if can_cache {
                tx.query_row("SELECT hash FROM files WHERE path=?1 AND size=?2 AND mtime_sec=?3 AND mtime_nsec=?4", params![key, size, seconds, nanos], |r| r.get(0)).optional()?
            } else {
                None
            };
            let hash = match cached {
                Some(hash) => hash,
                None => {
                    hashed += 1;
                    hash_file(path)?
                }
            };
            let after = fs::symlink_metadata(path)?;
            if !same_observation(&meta, &after) {
                return Err(Error::Scan("file changed while scanning".into()));
            }
            tx.execute(
                "INSERT INTO next_files VALUES (?1, ?2, ?3, ?4, ?5)",
                params![key, hash, size, seconds, nanos],
            )?;
            progress.on_file_complete(true);
        }
        if progress.is_cancelled() {
            return Err(Error::Scan("scan cancelled".into()));
        }
        if fs::canonicalize(&declared)? != canonical || root_observation(&canonical)? != root_before
        {
            return Err(Error::RootMismatch);
        }
        let pruned: i64 = tx.query_row(
            "SELECT COUNT(*) FROM files WHERE path NOT IN (SELECT path FROM next_files)",
            [],
            |r| r.get(0),
        )?;
        tx.execute_batch(
            "DELETE FROM files; INSERT INTO files SELECT * FROM next_files; DROP TABLE next_files;",
        )?;
        tx.execute("UPDATE meta SET canonical=?1, root_dev=?2, root_ino=?3, generation=generation+1, stale=0, last_scanned_at=unixepoch()", params![NativePath::from_path(&canonical).to_blob(), root_before.0, root_before.1])?;
        let (groups, files, wasted): (i64,i64,i64) = tx.query_row("SELECT COUNT(*),COALESCE(SUM(n),0),COALESCE(SUM((n-1)*size),0) FROM (SELECT COUNT(*) n,size FROM files GROUP BY hash,size HAVING n>1)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let duplicates = DuplicateStats {
            duplicate_groups: u64::try_from(groups)
                .map_err(|_| Error::Scan("invalid count".into()))?,
            duplicate_files: u64::try_from(files)
                .map_err(|_| Error::Scan("invalid count".into()))?,
            wasted_space: u64::try_from(wasted).map_err(|_| Error::Scan("invalid size".into()))?,
        };
        let result = ScanResult {
            hashed,
            errors: 0,
            pruned: u64::try_from(pruned).map_err(|_| Error::Scan("negative count".into()))?,
            duplicates,
        };
        tx.commit()?;
        progress.on_complete(&result);
        Ok(result)
    }

    /// Statistics for the last complete, non-stale generation.
    pub fn stats(&self) -> Result<ManifestStats> {
        let _snapshot = self.conn.unchecked_transaction()?;
        if self.legacy {
            let (count, size): (i64, i64) = self.conn.query_row(
                "SELECT COUNT(*),COALESCE(SUM(size),0) FROM files",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let (groups, files, wasted): (i64,i64,i64) = self.conn.query_row("SELECT COUNT(*),COALESCE(SUM(n),0),COALESCE(SUM((n-1)*size),0) FROM (SELECT COUNT(*) n,size FROM files GROUP BY hash,size HAVING n>1)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            let last_scanned_at =
                self.conn
                    .query_row("SELECT MAX(scanned_at) FROM files", [], |r| r.get(0))?;
            return Ok(ManifestStats {
                last_scanned_at,
                file_count: u64::try_from(count).map_err(|_| Error::Legacy)?,
                total_size: u64::try_from(size).map_err(|_| Error::Legacy)?,
                duplicates: DuplicateStats {
                    duplicate_files: u64::try_from(files).map_err(|_| Error::Legacy)?,
                    duplicate_groups: u64::try_from(groups).map_err(|_| Error::Legacy)?,
                    wasted_space: u64::try_from(wasted).map_err(|_| Error::Legacy)?,
                },
            });
        }
        Ok(ManifestStats {
            last_scanned_at: self
                .conn
                .query_row("SELECT last_scanned_at FROM meta", [], |r| r.get(0))?,
            file_count: self.file_count()?,
            total_size: self.total_size()?,
            duplicates: self.duplicate_stats_inner()?,
        })
    }
    /// Count entries in the current generation.
    pub fn file_count(&self) -> Result<u64> {
        self.require_fresh()?;
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        u64::try_from(n).map_err(|_| Error::Scan("invalid count".into()))
    }
    /// Sum file sizes in the current generation.
    pub fn total_size(&self) -> Result<u64> {
        self.require_fresh()?;
        let n: i64 = self
            .conn
            .query_row("SELECT COALESCE(SUM(size),0) FROM files", [], |r| r.get(0))?;
        u64::try_from(n).map_err(|_| Error::Scan("invalid size".into()))
    }

    /// Retrieve each duplicate path as a separate lossless row.
    pub fn find_duplicates(&self, min_size: u64) -> Result<Vec<DuplicateGroup>> {
        let _snapshot = self.conn.unchecked_transaction()?;
        self.find_duplicates_inner(min_size)
    }
    fn find_duplicates_inner(&self, min_size: u64) -> Result<Vec<DuplicateGroup>> {
        self.require_fresh()?;
        let min_size =
            i64::try_from(min_size).map_err(|_| Error::Scan("minimum size out of range".into()))?;
        let mut statement = self.conn.prepare("SELECT f.hash,f.path,f.size FROM files f JOIN (SELECT hash,size FROM files WHERE size>=?1 GROUP BY hash,size HAVING COUNT(*)>1) d ON f.hash=d.hash AND f.size=d.size ORDER BY f.hash,f.size,f.path")?;
        let rows = statement.query_map([min_size], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut groups: Vec<DuplicateGroup> = Vec::new();
        for row in rows {
            let (hash, blob, size) = row?;
            let path = NativePath::from_blob(&blob)?;
            path.relative()?;
            let size = u64::try_from(size).map_err(|_| Error::Scan("invalid size".into()))?;
            if let Some(group) = groups.last_mut()
                && group.hash == hash
                && group.size_each == size
            {
                group.paths.push(path);
                group.count += 1;
            } else {
                groups.push(DuplicateGroup {
                    hash,
                    paths: vec![path],
                    size_each: size,
                    count: 1,
                });
            }
        }
        Ok(groups)
    }
    /// Duplicate summary for the current generation.
    pub fn duplicate_stats(&self) -> Result<DuplicateStats> {
        let _snapshot = self.conn.unchecked_transaction()?;
        self.duplicate_stats_inner()
    }
    fn duplicate_stats_inner(&self) -> Result<DuplicateStats> {
        let groups = self.find_duplicates_inner(0)?;
        Ok(DuplicateStats {
            duplicate_files: groups.iter().map(|g| g.count as u64).sum(),
            duplicate_groups: groups.len() as u64,
            wasted_space: groups.iter().map(DuplicateGroup::wasted_space).sum(),
        })
    }
    /// Compare complete generations without attaching or modifying either database.
    pub fn compare_with(
        &self,
        other_db_path: &Path,
        min_size: u64,
    ) -> Result<Vec<CrossManifestDuplicate>> {
        let other = Self::open(other_db_path)?;
        let _left_snapshot = self.conn.unchecked_transaction()?;
        let _right_snapshot = other.conn.unchecked_transaction()?;
        self.require_fresh()?;
        other.require_fresh()?;
        let left = self.rows(min_size)?;
        let right = other.rows(min_size)?;
        let mut by_hash: HashMap<(String, u64), Vec<NativePath>> = HashMap::new();
        for (hash, size, path) in right {
            by_hash.entry((hash, size)).or_default().push(path);
        }
        let mut result = Vec::new();
        for (hash, size, source_path) in left {
            if let Some(paths) = by_hash.get(&(hash.clone(), size)) {
                for other_path in paths {
                    result.push(CrossManifestDuplicate {
                        hash: hash.clone(),
                        size,
                        source_path: source_path.clone(),
                        other_path: other_path.clone(),
                    });
                }
            }
        }
        result.sort_by_key(|item| std::cmp::Reverse(item.size));
        Ok(result)
    }
    /// Verify both keeper and candidate against the current root and generation,
    /// durably mark inventory stale, then delete candidates. A failure may
    /// occur after partial deletion; stale state remains until a complete scan.
    /// This does not protect against adversarial replacement after verification.
    #[cfg(unix)]
    pub fn delete_duplicates(&self, root: &Path, groups: &[DuplicateGroup]) -> Result<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        self.require_fresh()?;
        let declared = absolute(root)?;
        self.check_declared(&declared)?;
        let canonical = fs::canonicalize(&declared)?;
        let (stored, dev, ino, generation): (Vec<u8>, Option<i64>, Option<i64>, i64) =
            self.conn.query_row(
                "SELECT canonical,root_dev,root_ino,generation FROM meta",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        if stored != NativePath::from_path(&canonical).to_blob()
            || root_observation(&canonical)? != (dev, ino)
            || dev.is_none()
            || ino.is_none()
        {
            return Err(Error::RootMismatch);
        }
        if groups.is_empty() {
            return Ok((0, 0));
        }
        // Ensure every proposed identity belongs to this generation before
        // invalidating it. Never trust caller-provided duplicate groups alone.
        for group in groups {
            if group.paths.len() < 2 || group.count != group.paths.len() {
                return Err(Error::Stale);
            }
            for path in &group.paths {
                path.relative()?;
                let count: i64 = self.conn.query_row(
                    "SELECT COUNT(*) FROM files WHERE path=?1 AND hash=?2 AND size=?3",
                    params![
                        path.to_blob(),
                        group.hash,
                        i64::try_from(group.size_each).map_err(|_| Error::Stale)?
                    ],
                    |r| r.get(0),
                )?;
                if count != 1 {
                    return Err(Error::Stale);
                }
            }
        }
        let mut writer =
            Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        schema(&writer)?;
        writer.execute_batch("PRAGMA synchronous=FULL; BEGIN IMMEDIATE;")?;
        let changed = writer.execute("UPDATE meta SET stale=1 WHERE stale=0 AND generation>0 AND declared=?1 AND canonical=?2 AND root_dev=?3 AND root_ino=?4 AND generation=?5", params![NativePath::from_path(&declared).to_blob(),stored,dev,ino,generation])?;
        if changed != 1 {
            writer.execute_batch("ROLLBACK;")?;
            return Err(Error::Stale);
        }
        writer.execute_batch("COMMIT;")?;
        // Retake the write lock and verify that no intervening scan published a
        // new generation; hold it until all deletion dispatches have finished.
        let tx = writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (stale, current_generation): (i64, i64) =
            tx.query_row("SELECT stale,generation FROM meta", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        if stale != 1 || current_generation != generation {
            return Err(Error::Stale);
        }
        let mut count = 0;
        let mut bytes = 0;
        for group in groups {
            let keeper = group.paths[0].relative()?;
            for candidate in group.paths.iter().skip(1) {
                if fs::canonicalize(&declared)? != canonical
                    || root_observation(&canonical)? != (dev, ino)
                {
                    return Err(Error::RootMismatch);
                }
                let keeper_path = verified_file(&canonical, &keeper, &group.hash, group.size_each)?;
                let candidate_path = verified_file(
                    &canonical,
                    &candidate.relative()?,
                    &group.hash,
                    group.size_each,
                )?;
                let keep_meta = fs::metadata(&keeper_path)?;
                let candidate_meta = fs::metadata(&candidate_path)?;
                if keeper_path == candidate_path
                    || (keep_meta.dev() == candidate_meta.dev()
                        && keep_meta.ino() == candidate_meta.ino())
                {
                    return Err(Error::Stale);
                }
                fs::remove_file(candidate_path)?;
                count += 1;
                bytes += group.size_each;
            }
        }
        tx.commit()?;
        Ok((count, bytes))
    }
    /// Native file identity verification is unavailable on this platform.
    #[cfg(not(unix))]
    pub fn delete_duplicates(
        &self,
        _root: &Path,
        _groups: &[DuplicateGroup],
    ) -> Result<(u64, u64)> {
        Err(Error::Scan(
            "duplicate deletion is unsupported on this platform".into(),
        ))
    }

    fn rows(&self, minimum: u64) -> Result<Vec<(String, u64, NativePath)>> {
        let minimum =
            i64::try_from(minimum).map_err(|_| Error::Scan("minimum size out of range".into()))?;
        let mut stmt = self
            .conn
            .prepare("SELECT hash,size,path FROM files WHERE size>=?1")?;
        let rows = stmt.query_map([minimum], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (hash, size, blob) = row?;
            let path = NativePath::from_blob(&blob)?;
            path.relative()?;
            result.push((
                hash,
                u64::try_from(size).map_err(|_| Error::Scan("invalid size".into()))?,
                path,
            ));
        }
        Ok(result)
    }
}

#[cfg(unix)]
fn verified_file(root: &Path, relative: &Path, hash: &str, size: u64) -> Result<PathBuf> {
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.len() != size {
        return Err(Error::Stale);
    }
    let canonical = fs::canonicalize(&path)?;
    if !canonical.starts_with(root) || hash_file(&path)? != hash {
        return Err(Error::Stale);
    }
    if !same_observation(&metadata, &fs::symlink_metadata(&path)?) {
        return Err(Error::Stale);
    }
    Ok(path)
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Hasher::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(unix)]
fn root_observation(path: &Path) -> Result<(Option<i64>, Option<i64>)> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(path)?;
    if !meta.is_dir() {
        return Err(Error::Scan("root is not a directory".into()));
    }
    Ok((Some(meta.dev() as i64), Some(meta.ino() as i64)))
}
#[cfg(not(unix))]
fn root_observation(path: &Path) -> Result<(Option<i64>, Option<i64>)> {
    if !fs::metadata(path)?.is_dir() {
        return Err(Error::Scan("root is not a directory".into()));
    }
    Ok((None, None))
}
fn same_observation(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if a.dev() != b.dev() || a.ino() != b.ino() {
            return false;
        }
    }
    b.is_file() && a.len() == b.len() && a.modified().ok() == b.modified().ok()
}

/// Derive a stable database filename from declared native identity, even offline.
pub fn manifest_name(root: &Path) -> Result<String> {
    let root = absolute(root)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(std::env::consts::OS.as_bytes());
    hasher.update(&[0]);
    hasher.update(&NativePath::from_path(&root).to_blob());
    Ok(format!("v2-{}", hasher.finalize().to_hex()))
}
/// Presentation-only legacy path label.
pub fn path_to_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .replace(['/', '\\', ':'], "_")
}
/// Format bytes as human-readable size.
pub fn format_size(bytes: u64) -> String {
    const K: u64 = 1024;
    const M: u64 = K * 1024;
    const G: u64 = M * 1024;
    const T: u64 = G * 1024;
    if bytes >= T {
        format!("{:.2} TB", bytes as f64 / T as f64)
    } else if bytes >= G {
        format!("{:.1} GB", bytes as f64 / G as f64)
    } else if bytes >= M {
        format!("{:.1} MB", bytes as f64 / M as f64)
    } else if bytes >= K {
        format!("{:.1} KB", bytes as f64 / K as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, PathBuf, PathBuf) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        let db = temp.path().join("manifest.db");
        (temp, root, db)
    }

    #[test]
    fn scans_are_root_bound_complete_and_force_rehashes() {
        let (_temp, root, db) = fixture();
        fs::write(root.join("a"), "same").unwrap();
        let store = Manifest::open_for_root(&db, &root).unwrap();
        assert_eq!(store.scan(&root, false, &mut NoProgress).unwrap().hashed, 1);
        assert_eq!(store.scan(&root, false, &mut NoProgress).unwrap().hashed, 0);
        assert_eq!(store.scan(&root, true, &mut NoProgress).unwrap().hashed, 1);
        let other = root.parent().unwrap().join("other");
        fs::create_dir(&other).unwrap();
        assert!(matches!(
            store.scan(&other, true, &mut NoProgress),
            Err(Error::RootMismatch)
        ));
        assert_eq!(store.stats().unwrap().file_count, 1);
        fs::remove_file(root.join("a")).unwrap();
        let result = store.scan(&root, false, &mut NoProgress).unwrap();
        assert_eq!(result.pruned, 1);
        assert_eq!(store.stats().unwrap().file_count, 0);
    }

    struct RemoveOnFile;
    impl ProgressCallback for RemoveOnFile {
        fn on_start(&mut self, _: u64, _: u64) {}
        fn on_file(&mut self, _: &Path, _: u64) {}
        fn on_file_complete(&mut self, _: bool) {}
        fn on_complete(&mut self, _: &ScanResult) {}
    }

    #[test]
    fn failed_publication_keeps_previous_generation() {
        let (_temp, root, db) = fixture();
        fs::write(root.join("a"), "same").unwrap();
        let store = Manifest::open_for_root(&db, &root).unwrap();
        store.scan(&root, true, &mut NoProgress).unwrap();
        // A deterministic database failure aborts publication after traversal.
        store.conn.execute_batch("CREATE TRIGGER reject_publish BEFORE DELETE ON files BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
        fs::write(root.join("b"), "new").unwrap();
        assert!(store.scan(&root, true, &mut RemoveOnFile).is_err());
        assert_eq!(store.stats().unwrap().file_count, 1);
    }

    struct Vanish(PathBuf);
    impl ProgressCallback for Vanish {
        fn on_start(&mut self, _: u64, _: u64) {}
        fn on_file(&mut self, path: &Path, _: u64) {
            fs::remove_file(self.0.join(path)).unwrap();
        }
        fn on_file_complete(&mut self, _: bool) {}
        fn on_complete(&mut self, _: &ScanResult) {}
    }

    struct Cancel;
    impl ProgressCallback for Cancel {
        fn is_cancelled(&self) -> bool {
            true
        }
        fn on_start(&mut self, _: u64, _: u64) {}
        fn on_file(&mut self, _: &Path, _: u64) {}
        fn on_file_complete(&mut self, _: bool) {}
        fn on_complete(&mut self, _: &ScanResult) {}
    }
    #[test]
    fn cancellation_preserves_previous_generation() {
        let (_temp, root, db) = fixture();
        fs::write(root.join("a"), "old").unwrap();
        let store = Manifest::open_for_root(&db, &root).unwrap();
        store.scan(&root, true, &mut NoProgress).unwrap();
        assert!(store.scan(&root, true, &mut Cancel).is_err());
        assert_eq!(store.stats().unwrap().file_count, 1);
    }

    #[test]
    fn failed_hash_preserves_previous_generation() {
        let (_temp, root, db) = fixture();
        fs::write(root.join("a"), "old").unwrap();
        let store = Manifest::open_for_root(&db, &root).unwrap();
        store.scan(&root, true, &mut NoProgress).unwrap();
        assert!(store.scan(&root, true, &mut Vanish(root.clone())).is_err());
        assert_eq!(store.stats().unwrap().file_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn changed_root_symlink_does_not_rebind() {
        let (temp, root, db) = fixture();
        let link = temp.path().join("alias");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let store = Manifest::open_for_root(&db, &link).unwrap();
        store.scan(&link, true, &mut NoProgress).unwrap();
        let other = temp.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&other, &link).unwrap();
        assert!(matches!(
            store.scan(&link, true, &mut NoProgress),
            Err(Error::RootMismatch)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn verified_deletion_marks_stale_until_new_complete_scan() {
        let (_temp, root, db) = fixture();
        fs::write(root.join("a"), "same").unwrap();
        fs::write(root.join("b"), "same").unwrap();
        let store = Manifest::open_for_root(&db, &root).unwrap();
        store.scan(&root, true, &mut NoProgress).unwrap();
        let groups = store.find_duplicates(0).unwrap();
        assert_eq!(store.delete_duplicates(&root, &groups).unwrap(), (1, 4));
        assert!(matches!(store.stats(), Err(Error::Stale)));
        store.scan(&root, true, &mut NoProgress).unwrap();
        assert_eq!(store.stats().unwrap().file_count, 1);
    }

    #[test]
    fn read_only_open_rejects_wal_without_sidecars() {
        let (temp, _root, db) = fixture();
        let conn = Connection::open(&db).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        conn.execute_batch("CREATE TABLE files(path TEXT, hash TEXT, size INTEGER, mtime INTEGER, scanned_at INTEGER);").unwrap();
        drop(conn);
        let mut before: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        before.sort();
        assert!(Manifest::open(&db).is_err());
        let mut after: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        after.sort();
        assert_eq!(before, after);
    }

    #[test]
    fn legacy_stats_are_read_only_and_unqualified() {
        let (_temp, root, db) = fixture();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE files(path TEXT, hash TEXT, size INTEGER, mtime INTEGER, scanned_at INTEGER); INSERT INTO files VALUES ('a','h',4,0,123);").unwrap();
        drop(conn);
        let before = fs::read(&db).unwrap();
        let store = Manifest::open_existing(&db, &root).unwrap();
        assert!(store.is_legacy());
        assert_eq!(store.stats().unwrap().last_scanned_at, Some(123));
        assert!(matches!(store.find_duplicates(0), Err(Error::Legacy)));
        drop(store);
        assert!(matches!(
            Manifest::open_for_root(&db, &root),
            Err(Error::Legacy)
        ));
        assert_eq!(before, fs::read(&db).unwrap());
        let missing = db.parent().unwrap().join("missing/sub.db");
        assert!(Manifest::open(&missing).is_err());
        assert!(!missing.parent().unwrap().exists());
    }

    #[cfg(unix)]
    #[test]
    fn native_paths_round_trip_and_cross_compare() {
        use std::os::unix::ffi::OsStringExt;
        let (_temp, root, db) = fixture();
        let raw_name = std::ffi::OsString::from_vec(b"raw-\xff".to_vec());
        let identity = NativePath::from_path(Path::new(&raw_name));
        assert_eq!(
            NativePath::from_blob(&identity.to_blob()).unwrap(),
            identity
        );
        let encoded = serde_json::to_string(&identity).unwrap();
        assert_eq!(
            serde_json::from_str::<NativePath>(&encoded).unwrap(),
            identity
        );
        assert_eq!(
            identity.to_path_buf().unwrap().as_os_str(),
            raw_name.as_os_str()
        );

        // These names are always exercised, even when the filesystem rejects
        // non-UTF-8 names. Only macOS EILSEQ qualifies that single fixture.
        let literal_name = "a|\nname";
        fs::write(root.join(literal_name), "same").unwrap();
        fs::write(root.join("other"), "same").unwrap();
        let raw_created = match fs::write(root.join(&raw_name), "same") {
            Ok(()) => true,
            #[cfg(target_os = "macos")]
            Err(error) if error.raw_os_error() == Some(92) => false,
            Err(error) => panic!("could not create native-byte fixture: {error}"),
        };
        let store = Manifest::open_for_root(&db, &root).unwrap();
        store.scan(&root, true, &mut NoProgress).unwrap();
        let group = store.find_duplicates(0).unwrap().remove(0);
        assert!(
            group
                .paths
                .iter()
                .any(|p| p.to_path_buf().unwrap().as_os_str() == std::ffi::OsStr::new(literal_name))
        );
        if raw_created {
            assert!(
                group
                    .paths
                    .iter()
                    .any(|p| p.to_path_buf().unwrap().as_os_str() == raw_name.as_os_str())
            );
        }
        let json = serde_json::to_string(&group).unwrap();
        let decoded: DuplicateGroup = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.paths, group.paths);
        let (_temp2, root2, db2) = fixture();
        fs::write(root2.join("shared"), "same").unwrap();
        let other = Manifest::open_for_root(&db2, &root2).unwrap();
        other.scan(&root2, true, &mut NoProgress).unwrap();
        assert_eq!(
            store.compare_with(&db2, 0).unwrap().len(),
            if raw_created { 3 } else { 2 }
        );
    }

    #[cfg(unix)]
    #[test]
    fn deletion_rejects_foreign_groups_and_replaced_symlink() {
        let (temp, root, db) = fixture();
        fs::write(root.join("a"), "same").unwrap();
        fs::write(root.join("b"), "same").unwrap();
        let store = Manifest::open_for_root(&db, &root).unwrap();
        store.scan(&root, true, &mut NoProgress).unwrap();
        let groups = store.find_duplicates(0).unwrap();
        let mut forged = groups.clone();
        forged[0].hash = "different".into();
        assert!(store.delete_duplicates(&root, &forged).is_err());
        assert_eq!(store.stats().unwrap().file_count, 2);
        let outside = temp.path().join("outside");
        fs::write(&outside, "same").unwrap();
        let candidate = root.join(groups[0].paths[1].to_path_buf().unwrap());
        fs::remove_file(&candidate).unwrap();
        std::os::unix::fs::symlink(&outside, &candidate).unwrap();
        assert!(store.delete_duplicates(&root, &groups).is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "same");
        assert!(matches!(store.stats(), Err(Error::Stale)));
    }

    #[cfg(unix)]
    #[test]
    fn deletion_requires_durable_stale_marker_and_fresh_proof() {
        let (_temp, root, db) = fixture();
        fs::write(root.join("a"), "same").unwrap();
        fs::write(root.join("b"), "same").unwrap();
        let store = Manifest::open_for_root(&db, &root).unwrap();
        store.scan(&root, true, &mut NoProgress).unwrap();
        let groups = store.find_duplicates(0).unwrap();
        store.conn.execute_batch("CREATE TRIGGER reject_stale BEFORE UPDATE OF stale ON meta WHEN NEW.stale=1 BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
        assert!(store.delete_duplicates(&root, &groups).is_err());
        assert!(root.join("a").exists() && root.join("b").exists());
        store
            .conn
            .execute_batch("DROP TRIGGER reject_stale;")
            .unwrap();
        fs::write(root.join("b"), "diff").unwrap();
        assert!(store.delete_duplicates(&root, &groups).is_err());
        assert!(root.join("a").exists() && root.join("b").exists());
        assert!(matches!(store.stats(), Err(Error::Stale)));
    }
}
