//! Symlink resource - native stow replacement

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

use super::{ApplyContext, ApplyResult, Resource, ResourceState, SudoRequirement};

/// A symlink to create
#[derive(Debug, Clone)]
pub struct Symlink {
    /// Source path (what the symlink points to)
    pub source: PathBuf,
    /// Target path (where the symlink is created)
    pub target: PathBuf,
}

impl Symlink {
    pub fn new(source: impl AsRef<Path>, target: impl AsRef<Path>) -> Self {
        Self {
            source: source.as_ref().to_path_buf(),
            target: target.as_ref().to_path_buf(),
        }
    }

    /// Paths are already resolved by the configuration boundary. Filesystem
    /// names, including literal dollar signs and non-UTF-8 bytes, stay literal.
    pub fn paths(&self) -> Result<(PathBuf, PathBuf)> {
        let source = self.source.clone();
        let target = self.target.clone();
        if !source.is_absolute() || !target.is_absolute() {
            bail!("Symlink paths must be absolute after expansion");
        }
        Ok((source, target))
    }

    /// Check current symlink state
    fn check_current(&self) -> Result<SymlinkState> {
        let (source, target) = self.paths()?;

        let expected = source
            .canonicalize()
            .with_context(|| format!("Symlink source is unavailable: {}", source.display()))?;
        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                bossa::confirm_missing_parent(&target)?;
                return Ok(SymlinkState::Missing);
            }
            Err(error) => return Err(error).context("Could not observe symlink destination"),
        };

        if metadata.file_type().is_symlink() {
            let link_target = fs::read_link(&target).context("Failed to read symlink")?;

            // Canonicalize for comparison
            let resolved = if link_target.is_absolute() {
                link_target
            } else {
                target
                    .parent()
                    .context("Symlink destination has no parent")?
                    .join(link_target)
            };
            let actual = match resolved.canonicalize() {
                Ok(path) => path,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => resolved,
                Err(error) => {
                    return Err(error).context("Could not observe existing symlink source");
                }
            };

            if expected == actual {
                Ok(SymlinkState::Correct)
            } else {
                Ok(SymlinkState::WrongTarget(actual))
            }
        } else {
            Ok(SymlinkState::FileExists)
        }
    }

    /// Create the symlink
    fn create_symlink(&self) -> Result<()> {
        let (source, target) = self.paths()?;

        // Ensure source exists
        fs::metadata(&source)
            .with_context(|| format!("Source is unavailable: {}", source.display()))?;

        // Ensure parent directory exists
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create parent directory: {}", parent.display())
            })?;
        }

        // Create symlink
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source, &target).with_context(|| {
            format!(
                "Failed to create symlink: {} -> {}",
                target.display(),
                source.display()
            )
        })?;

        #[cfg(windows)]
        {
            use std::os::windows::fs::{symlink_dir, symlink_file};

            if source.is_dir() {
                // Use junction for directories (doesn't require admin privileges)
                // Fall back to symlink_dir if junction fails
                match junction::create(&source, &target) {
                    Ok(()) => (),
                    Err(e) => {
                        log::debug!("Junction creation failed ({}), trying symlink_dir", e);
                        symlink_dir(&source, &target).with_context(|| {
                            format!(
                                "Failed to create directory symlink: {} -> {}",
                                target.display(),
                                source.display()
                            )
                        })?;
                    }
                }
            } else {
                symlink_file(&source, &target).with_context(|| {
                    format!(
                        "Failed to create file symlink: {} -> {}",
                        target.display(),
                        source.display()
                    )
                })?;
            }
        }

        #[cfg(not(any(unix, windows)))]
        bail!("Symlinks not supported on this platform");

        Ok(())
    }
}

#[derive(Debug)]
enum SymlinkState {
    Missing,
    Correct,
    WrongTarget(PathBuf),
    FileExists,
}

impl Resource for Symlink {
    fn id(&self) -> String {
        self.target.to_string_lossy().to_string()
    }

    fn description(&self) -> String {
        format!(
            "Symlink {} -> {}",
            self.target.display(),
            self.source.display()
        )
    }

    fn resource_type(&self) -> &'static str {
        "symlink"
    }

    fn sudo_requirement(&self) -> SudoRequirement {
        SudoRequirement::None
    }

    fn current_state(&self) -> Result<ResourceState> {
        match self.check_current()? {
            SymlinkState::Missing => Ok(ResourceState::Absent),
            SymlinkState::Correct => Ok(ResourceState::Present {
                details: Some(format!("-> {}", self.source.display())),
            }),
            SymlinkState::WrongTarget(actual) => bail!(
                "Existing symlink at {} points to {}; preserved for explicit reconciliation",
                self.target.display(),
                actual.display()
            ),
            SymlinkState::FileExists => bail!(
                "Existing file/directory at {}; preserved for explicit reconciliation",
                self.target.display()
            ),
        }
    }

    fn desired_state(&self) -> ResourceState {
        ResourceState::Present {
            details: Some(format!("-> {}", self.source.display())),
        }
    }

    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped {
                reason: "Dry run".to_string(),
            });
        }

        match self.check_current()? {
            SymlinkState::Correct => Ok(ApplyResult::NoChange),
            SymlinkState::Missing => {
                self.create_symlink()?;
                Ok(ApplyResult::Created)
            }
            SymlinkState::WrongTarget(_) => bail!(
                "Existing symlink has a different source: {}; preserved for explicit reconciliation",
                self.target.display()
            ),
            SymlinkState::FileExists => {
                bail!(
                    "Existing file/directory at {}; preserved for explicit reconciliation",
                    self.target.display()
                )
            }
        }
    }
}
