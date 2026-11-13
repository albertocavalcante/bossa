//! Disk backup command - copy directory with progress
//!
//! Copies files from source to destination while:
//! - Showing progress with a progress bar
//! - Skipping system files (.DS_Store, .Spotlight-V100, .fseventsd, .Trashes)
//! - Validating destination has enough space
//! - Supporting dry-run mode
//! - Refusing overlapping source/destination trees
//! - Recreating symlinks instead of copying their targets
//! - Refusing destination symlinks found during backup

use anyhow::{Context, Result};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::path::{Component, Path, PathBuf};
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

/// Kind of filesystem entry (as seen without following symlinks)
#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Dir,
    File,
    Symlink,
}

/// Entry to be copied
struct CopyEntry {
    source: PathBuf,
    dest: PathBuf,
    size: u64,
    kind: EntryKind,
}

impl CopyEntry {
    fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }
}

/// Run the backup command
pub fn run(source: &str, destination: &str, dry_run: bool) -> Result<()> {
    let source_path = expand_path(source);
    let dest_path = expand_path(destination);

    // Validate source exists
    if !source_path.exists() {
        anyhow::bail!("Source path does not exist: {}", source_path.display());
    }

    // Refuse overlapping trees before touching anything: copying a tree onto
    // itself truncates every file (fs::copy opens the destination with O_TRUNC).
    let dest_without_trailing_slash: PathBuf = dest_path.components().collect();
    if fs::symlink_metadata(&dest_without_trailing_slash).is_ok_and(|m| m.file_type().is_symlink())
    {
        anyhow::bail!("Destination root is a symlink: {}", dest_path.display());
    }
    let source_root = resolve_path(&source_path)?;
    let dest_root = resolve_path(&dest_path)?;
    check_no_overlap(&source_root, &dest_root)?;

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
    let entries = collect_entries(&source_root, &dest_root)?;

    if entries.is_empty() {
        ui::info("No files to copy");
        return Ok(());
    }

    let total_size: u64 = entries.iter().map(|e| e.size).sum();
    let file_count = entries.iter().filter(|e| !e.is_dir()).count();
    let dir_count = entries.iter().filter(|e| e.is_dir()).count();

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
    let skip_info = count_skipped(&source_root);
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
        let file_entries: Vec<_> = entries.iter().filter(|e| !e.is_dir()).collect();

        for entry in file_entries.iter().take(max_show) {
            let rel_path = entry
                .source
                .strip_prefix(&source_root)
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
        perform_backup(&entries, &source_root, &dest_root, total_size)?;
    }

    Ok(())
}

/// Expand ~ and environment variables in paths
fn expand_path(path: &str) -> PathBuf {
    crate::paths::expand(path)
}

/// Resolve a path to its canonical form, even if it does not exist yet.
///
/// The nearest existing ancestor is canonicalized (resolving symlinks) and the
/// non-existent remainder is re-appended; ambiguous parent traversal is rejected.
fn resolve_path(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)
        .with_context(|| format!("Failed to resolve path: {}", path.display()))?;
    for ancestor in absolute.ancestors() {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                let rest = absolute.strip_prefix(ancestor)?;
                for component in rest.components() {
                    match component {
                        Component::Normal(part) => resolved.push(part),
                        Component::CurDir => {}
                        // A missing component followed by `..` may reach an existing
                        // symlink. Refuse ambiguous paths instead of normalizing it.
                        _ => anyhow::bail!("Cannot safely resolve path: {}", path.display()),
                    }
                }
                return Ok(resolved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("Failed to resolve path: {}", path.display()));
            }
        }
    }
    anyhow::bail!("Failed to resolve path: {}", path.display())
}

/// Refuse to back up when source and destination are the same tree or nest
/// inside one another.
fn check_no_overlap(source: &Path, dest: &Path) -> Result<()> {
    if dest == source {
        anyhow::bail!(
            "Destination is the same as the source ({}); refusing to back up onto itself",
            source.display()
        );
    }
    if dest.starts_with(source) {
        anyhow::bail!(
            "Destination {} is inside the source {}; refusing to back up",
            dest.display(),
            source.display()
        );
    }
    if source.starts_with(dest) {
        anyhow::bail!(
            "Source {} is inside the destination {}; refusing to back up",
            source.display(),
            dest.display()
        );
    }

    Ok(())
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

        // With follow_links(false) this is the symlink's own metadata. The
        // root is always followed so it is never recreated as a symlink.
        let metadata = if entry.depth() == 0 {
            fs::metadata(path)
        } else {
            entry.metadata().map_err(std::io::Error::from)
        }
        .context("Failed to read metadata")?;
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            EntryKind::Symlink
        } else if file_type.is_dir() {
            EntryKind::Dir
        } else {
            EntryKind::File
        };
        let size = if kind == EntryKind::File {
            metadata.len()
        } else {
            0
        };

        entries.push(CopyEntry {
            source: path.to_path_buf(),
            dest: dest_path,
            size,
            kind,
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
#[allow(unsafe_code)] // statvfs requires unsafe FFI
fn check_destination_space(dest: &Path, required: u64) -> Result<()> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    // Check the destination, or its nearest existing ancestor if it doesn't exist
    let check_path = dest
        .ancestors()
        .find(|p| p.exists())
        .map_or_else(|| PathBuf::from("/"), Path::to_path_buf);

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

        statvfs_to_u64(stat.f_bavail) * statvfs_to_u64(stat.f_frsize)
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

#[cfg(unix)]
fn statvfs_to_u64<T: Into<u64>>(value: T) -> u64 {
    value.into()
}

#[cfg(not(unix))]
fn check_destination_space(_dest: &Path, _required: u64) -> Result<()> {
    ui::warn("Cannot check destination space on this platform");
    Ok(())
}

/// Perform the actual backup
fn perform_backup(
    entries: &[CopyEntry],
    source_base: &Path,
    dest_base: &Path,
    total_size: u64,
) -> Result<()> {
    let pb = ProgressBar::new(total_size);
    let style = ProgressStyle::default_bar();
    let style = match style
        .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) {msg}")
    {
        Ok(style) => style.progress_chars("=>-"),
        Err(_) => ProgressStyle::default_bar(),
    };
    pb.set_style(style);

    let mut copied_count = 0;
    let mut error_count = 0;
    let mut copied_bytes = 0u64;

    // First pass: create directories (parents before children, in walk order)
    for entry in entries.iter().filter(|e| e.is_dir()) {
        if let Err(e) = create_real_dir(dest_base, &entry.dest) {
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
    for entry in entries.iter().filter(|e| !e.is_dir()) {
        let rel_path = entry
            .source
            .strip_prefix(source_base)
            .unwrap_or(&entry.source);

        pb.set_message(ui::truncate_path(&rel_path.display().to_string(), 40));

        // Ensure the parent directory exists and that nothing between the
        // destination root and this entry is a symlink we would write through.
        if let Err(e) =
            check_no_symlink_ancestors(dest_base, &entry.dest).and_then(|()| {
                match entry.dest.parent() {
                    Some(parent) if fs::symlink_metadata(parent).is_err() => {
                        fs::create_dir_all(parent)
                    }
                    _ => Ok(()),
                }
            })
        {
            pb.suspend(|| {
                println!(
                    "  {} Failed to prepare parent directory of {}: {}",
                    "!".red(),
                    rel_path.display(),
                    e
                );
            });
            error_count += 1;
            continue;
        }

        match copy_entry(entry) {
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
        anyhow::bail!("Backup finished with {error_count} errors");
    }

    Ok(())
}

/// Fail if any directory strictly between `dest_base` and `dest` is a
/// symlink. Such links are left by earlier backups and writing below them
/// would modify whatever they point to.
fn check_no_symlink_ancestors(dest_base: &Path, dest: &Path) -> std::io::Result<()> {
    let Some(rel_parent) = dest.strip_prefix(dest_base).ok().and_then(Path::parent) else {
        return Ok(());
    };

    reject_symlink(dest_base)?;
    let mut current = dest_base.to_path_buf();
    for component in rel_parent.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(std::io::Error::other(format!(
                    "{} is a symlink; refusing to write through it",
                    current.display()
                )));
            }
            Ok(_) => {}
            // Nothing below a missing directory can be a symlink.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

/// Create `dest` as a real directory, refusing an existing symlink.
fn create_real_dir(dest_base: &Path, dest: &Path) -> std::io::Result<()> {
    check_no_symlink_ancestors(dest_base, dest)?;
    reject_symlink(dest)?;
    fs::create_dir_all(dest)
}

fn reject_symlink(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(std::io::Error::other(format!(
            "{} is a symlink; refusing to write through it",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Copy a single non-directory entry, returning the number of bytes copied.
///
/// Never copies a file onto itself, recreates symlinks rather than copying
/// their targets. Files are copied to a temporary sibling before replacement,
/// preventing an existing hard link from being truncated.
fn copy_entry(entry: &CopyEntry) -> std::io::Result<u64> {
    if entry.kind == EntryKind::Symlink {
        return copy_symlink(&entry.source, &entry.dest).map(|()| 0);
    }

    if let (Ok(src_meta), Ok(dest_meta)) = (
        fs::metadata(&entry.source),
        fs::symlink_metadata(&entry.dest),
    ) && same_file(&src_meta, &dest_meta)
    {
        return Err(std::io::Error::other(
            "source and destination are the same file",
        ));
    }

    reject_symlink(&entry.dest)?;
    let parent = entry.dest.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    let mut source = fs::File::open(&entry.source)?;
    let bytes = std::io::copy(&mut source, &mut tmp)?;
    tmp.as_file()
        .set_permissions(source.metadata()?.permissions())?;
    tmp.persist(&entry.dest).map_err(|e| e.error)?;
    Ok(bytes)
}

/// Recreate a symlink at `dest` pointing to the same target as `source`.
#[cfg(unix)]
fn copy_symlink(source: &Path, dest: &Path) -> std::io::Result<()> {
    let target = fs::read_link(source)?;

    if let Ok(dest_meta) = fs::symlink_metadata(dest) {
        let src_meta = fs::symlink_metadata(source)?;
        if same_file(&src_meta, &dest_meta) {
            return Err(std::io::Error::other(
                "source and destination are the same file",
            ));
        }
        if !dest_meta.file_type().is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "destination exists and is not a symlink",
            ));
        }
        if fs::read_link(dest)? == target {
            return Ok(());
        }
        return Err(std::io::Error::other(
            "destination symlink has a different target; refusing to replace it",
        ));
    }

    std::os::unix::fs::symlink(target, dest)
}

#[cfg(not(unix))]
fn copy_symlink(_source: &Path, _dest: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "copying symlinks is not supported on this platform",
    ))
}

/// Whether two metadata entries refer to the same underlying file.
#[cfg(unix)]
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(_a: &fs::Metadata, _b: &fs::Metadata) -> bool {
    false
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

    fn make_source(root: &Path) -> PathBuf {
        let src = root.join("src");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("a.txt"), "alpha").unwrap();
        fs::write(src.join("nested").join("b.txt"), "bravo").unwrap();
        src
    }

    fn assert_source_intact(src: &Path) {
        assert_eq!(fs::read_to_string(src.join("a.txt")).unwrap(), "alpha");
        assert_eq!(
            fs::read_to_string(src.join("nested").join("b.txt")).unwrap(),
            "bravo"
        );
    }

    fn path_str(path: &Path) -> &str {
        path.to_str().unwrap()
    }

    #[test]
    fn test_backup_refuses_same_source_and_destination() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());

        let result = run(path_str(&src), path_str(&src), false);

        assert!(result.is_err(), "backup onto itself must be refused");
        assert_source_intact(&src);
    }

    #[test]
    fn test_backup_refuses_same_source_and_destination_with_trailing_slash() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        let dest = format!("{}/", src.display());

        let result = run(path_str(&src), &dest, false);

        assert!(result.is_err(), "backup onto itself must be refused");
        assert_source_intact(&src);
    }

    #[test]
    fn test_backup_refuses_destination_inside_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        let dest = src.join("sub");

        let result = run(path_str(&src), path_str(&dest), false);

        assert!(result.is_err(), "destination inside source must be refused");
        assert!(!dest.exists(), "nothing should be written");
        assert_source_intact(&src);
    }

    #[test]
    fn test_backup_refuses_source_inside_destination() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());

        let result = run(path_str(&src.join("nested")), path_str(tmp.path()), false);

        assert!(result.is_err(), "source inside destination must be refused");
        assert_source_intact(&src);
    }

    #[cfg(unix)]
    #[test]
    fn test_backup_refuses_destination_symlinked_to_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&src, &link).unwrap();

        let result = run(path_str(&src), path_str(&link), false);

        assert!(
            result.is_err(),
            "destination resolving to source must be refused"
        );
        assert_source_intact(&src);
    }

    #[test]
    fn test_backup_refuses_dry_run_overlap() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());

        assert!(run(path_str(&src), path_str(&src), true).is_err());
        assert_source_intact(&src);
    }

    #[test]
    fn test_backup_copies_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        let dest = tmp.path().join("backup").join("docs");

        run(path_str(&src), path_str(&dest), false).unwrap();

        assert_source_intact(&src);
        assert_source_intact(&dest);
    }

    #[test]
    fn test_backup_returns_error_on_partial_failure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        let dest = tmp.path().join("backup");
        // A directory where a file should go makes that one copy fail.
        fs::create_dir_all(dest.join("a.txt")).unwrap();

        let result = run(path_str(&src), path_str(&dest), false);

        assert!(result.is_err(), "failed copies must produce an error");
        // The other file is still copied.
        assert_eq!(
            fs::read_to_string(dest.join("nested").join("b.txt")).unwrap(),
            "bravo"
        );
        // No temporary sibling remains after the failed rename.
        let mut names: Vec<_> = fs::read_dir(&dest)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, ["a.txt", "nested"].map(std::ffi::OsString::from));
        fs::remove_dir(dest.join("a.txt")).unwrap();
        run(path_str(&src), path_str(&dest), false).unwrap();
        assert_source_intact(&dest);
        assert_source_intact(&src);
    }

    #[cfg(unix)]
    #[test]
    fn test_backup_recreates_symlinks() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        std::os::unix::fs::symlink("a.txt", src.join("file_link")).unwrap();
        std::os::unix::fs::symlink("nested", src.join("dir_link")).unwrap();
        let dest = tmp.path().join("backup");

        run(path_str(&src), path_str(&dest), false).unwrap();

        for name in ["file_link", "dir_link"] {
            let meta = fs::symlink_metadata(dest.join(name)).unwrap();
            assert!(
                meta.file_type().is_symlink(),
                "{name} should stay a symlink"
            );
        }
        assert_eq!(
            fs::read_link(dest.join("file_link")).unwrap(),
            PathBuf::from("a.txt")
        );
        assert_eq!(
            fs::read_link(dest.join("dir_link")).unwrap(),
            PathBuf::from("nested")
        );

        // Identical symlinks need no replacement on a repeat backup.
        run(path_str(&src), path_str(&dest), false).unwrap();
        assert_eq!(
            fs::read_link(dest.join("file_link")).unwrap(),
            PathBuf::from("a.txt")
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_refuses_changed_symlink_target() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        symlink("a.txt", src.join("link")).unwrap();
        let dest = tmp.path().join("backup");
        run(path_str(&src), path_str(&dest), false).unwrap();
        fs::remove_file(src.join("link")).unwrap();
        symlink("nested", src.join("link")).unwrap();
        assert!(run(path_str(&src), path_str(&dest), false).is_err());
        assert_eq!(
            fs::read_link(dest.join("link")).unwrap(),
            Path::new("a.txt")
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_refuses_destination_symlinks_without_touching_targets() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        let dest = tmp.path().join("backup");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&dest).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("target"), "keep").unwrap();
        symlink(outside.join("target"), dest.join("a.txt")).unwrap();
        symlink(&outside, dest.join("nested")).unwrap();
        assert!(run(path_str(&src), path_str(&dest), false).is_err());
        assert_eq!(fs::read_to_string(outside.join("target")).unwrap(), "keep");
        assert!(!outside.join("b.txt").exists());
        assert!(
            fs::symlink_metadata(dest.join("a.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::symlink_metadata(dest.join("nested"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_source_intact(&src);
    }

    #[cfg(unix)]
    #[test]
    fn test_refuses_ambiguous_missing_component_before_parent() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        symlink(&src, tmp.path().join("alias")).unwrap();
        let dest = tmp.path().join("missing/../alias/new");
        assert!(run(path_str(&src), path_str(&dest), true).is_err());
        assert!(!src.join("new").exists());
        assert_source_intact(&src);
    }

    #[cfg(unix)]
    #[test]
    fn test_existing_hard_link_to_another_source_file_is_not_truncated() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        let dest = tmp.path().join("backup");
        fs::create_dir_all(&dest).unwrap();
        fs::hard_link(src.join("nested/b.txt"), dest.join("a.txt")).unwrap();
        run(path_str(&src), path_str(&dest), false).unwrap();
        assert_source_intact(&src);
        assert_source_intact(&dest);
    }

    #[cfg(unix)]
    #[test]
    fn test_backup_follows_symlinked_source_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        let link = tmp.path().join("src_link");
        std::os::unix::fs::symlink(&src, &link).unwrap();
        let dest = tmp.path().join("backup");

        run(path_str(&link), path_str(&dest), false).unwrap();

        for path in [dest.clone(), dest.join("nested")] {
            let meta = fs::symlink_metadata(&path).unwrap();
            assert!(
                meta.file_type().is_dir(),
                "{} must be a real dir",
                path.display()
            );
        }
        assert!(
            fs::symlink_metadata(dest.join("a.txt"))
                .unwrap()
                .file_type()
                .is_file()
        );
        assert_source_intact(&dest);
        assert_source_intact(&src);
    }

    #[cfg(unix)]
    #[test]
    fn test_copy_entry_skips_same_inode() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = make_source(tmp.path());
        let alias = tmp.path().join("alias");
        fs::hard_link(src.join("a.txt"), &alias).unwrap();

        let entry = CopyEntry {
            source: src.join("a.txt"),
            dest: alias,
            size: 5,
            kind: EntryKind::File,
        };

        assert!(copy_entry(&entry).is_err());
        assert_source_intact(&src);
    }
}
