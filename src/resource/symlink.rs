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

    /// Expand ~ and environment variables in paths
    pub fn expand_paths(&self) -> Result<(PathBuf, PathBuf)> {
        let source = crate::paths::expand(&self.source.to_string_lossy());
        let target = crate::paths::expand(&self.target.to_string_lossy());
        Ok((source, target))
    }

    /// Check current symlink state
    fn check_current(&self) -> Result<SymlinkState> {
        let (source, target) = self.expand_paths()?;

        if !target.exists() && !target.is_symlink() {
            return Ok(SymlinkState::Missing);
        }

        if target.is_symlink() {
            let link_target = fs::read_link(&target).context("Failed to read symlink")?;

            // Canonicalize for comparison
            let expected = source.canonicalize().unwrap_or(source.clone());
            let actual = if link_target.is_absolute() {
                link_target.canonicalize().unwrap_or(link_target)
            } else {
                target
                    .parent()
                    .map(|p| p.join(&link_target))
                    .and_then(|p| p.canonicalize().ok())
                    .unwrap_or(link_target)
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
        let (source, target) = self.expand_paths()?;

        // Ensure source exists
        if !source.exists() {
            bail!("Source does not exist: {}", source.display());
        }

        // Ensure parent directory exists
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create parent directory: {}", parent.display())
            })?;
        }

        // Remove existing symlink if wrong target
        if target.is_symlink() {
            fs::remove_file(&target).with_context(|| {
                format!("Failed to remove existing symlink: {}", target.display())
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
            SymlinkState::WrongTarget(actual) => Ok(ResourceState::Modified {
                from: actual.to_string_lossy().to_string(),
                to: self.source.to_string_lossy().to_string(),
            }),
            SymlinkState::FileExists => Ok(ResourceState::Modified {
                from: "regular file".to_string(),
                to: format!("symlink -> {}", self.source.display()),
            }),
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
            SymlinkState::WrongTarget(_) => {
                self.create_symlink()?;
                Ok(ApplyResult::Modified)
            }
            SymlinkState::FileExists => {
                // Don't overwrite existing files automatically
                Ok(ApplyResult::Skipped {
                    reason: format!("File exists at {}", self.target.display()),
                })
            }
        }
    }
}
