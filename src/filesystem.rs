use std::fs;
use std::io;
use std::path::Path;

/// Check ancestors after a path lookup reports NotFound. Unavailable links and
/// non-directory parents remain errors. This neither reserves absence nor
/// authorizes overwriting or deleting anything.
pub fn confirm_missing_parent(path: &Path) -> io::Result<()> {
    // Missing leaves under inaccessible or dangling directories are unavailable.
    for ancestor in path.ancestors().skip(1) {
        let ancestor = if ancestor.as_os_str().is_empty() {
            Path::new(".")
        } else {
            ancestor
        };
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                if !fs::metadata(ancestor)?.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        "Path parent is not a directory",
                    ));
                }
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Resolve existing ancestors without treating broken links or non-directories as absence.
pub fn resolve_location(path: &Path) -> anyhow::Result<std::path::PathBuf> {
    anyhow::ensure!(path.is_absolute(), "Location must be absolute");
    use anyhow::Context as _;
    let mut existing = path.to_path_buf();
    let mut suffix = Vec::new();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(_) => {
                let mut resolved = existing.canonicalize().context("Location unavailable")?;
                anyhow::ensure!(
                    suffix.is_empty() || fs::metadata(&existing)?.is_dir(),
                    "Location parent is not a directory"
                );
                for part in suffix.into_iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(
                    existing
                        .file_name()
                        .context("No existing location ancestor")?
                        .to_os_string(),
                );
                existing = existing.parent().context("No location parent")?.into();
            }
            Err(e) => return Err(e).context("Could not inspect location"),
        }
    }
}
