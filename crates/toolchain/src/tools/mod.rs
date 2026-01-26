//! Tool-specific installation logic.
//!
//! This module provides the [`ToolInstaller`] trait and implementations
//! for installing specific build tools. Each tool has its own installer
//! that knows how to decompress, install, and verify the tool.
//!
//! # Supported Tools
//!
//! - [`buck2::Buck2Installer`] - Meta's Buck2 build system

pub mod buck2;

use crate::error::Result;
use crate::types::{InstallOptions, InstallResult, Platform, Tool};
use std::path::Path;

/// Trait for tool-specific installation logic.
///
/// Implementors of this trait handle the specifics of installing a particular
/// tool, including decompression, file placement, and verification.
pub trait ToolInstaller: Send + Sync {
    /// Get the tool this installer handles.
    fn tool(&self) -> Tool;

    /// Install the tool from downloaded bytes.
    ///
    /// The `bytes` parameter contains the raw (possibly compressed) binary data.
    /// The installer is responsible for decompression if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Decompression fails
    /// - The installation directory cannot be created
    /// - Writing the binary fails
    /// - Verification fails
    fn install(
        &self,
        bytes: &[u8],
        platform: &Platform,
        options: &InstallOptions,
    ) -> Result<InstallResult>;

    /// Check if the tool is installed.
    ///
    /// Returns `true` if the tool is found in PATH.
    fn is_installed(&self) -> Result<bool>;

    /// Get the installed version.
    ///
    /// Returns `None` if the tool is not installed.
    fn installed_version(&self) -> Result<Option<String>>;

    /// Get the default installation directory.
    ///
    /// Typically `~/.local/bin` on Unix or an equivalent on Windows.
    fn default_install_dir(&self) -> Result<std::path::PathBuf>;

    /// Verify the installation works.
    ///
    /// Runs the tool with a version flag to ensure it's functional.
    fn verify(&self, path: &Path) -> Result<()>;
}
