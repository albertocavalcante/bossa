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
