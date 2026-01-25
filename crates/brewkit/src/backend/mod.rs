//! Backend abstraction for Homebrew operations.
//!
//! The [`Backend`] trait defines the interface for interacting with Homebrew,
//! allowing for different implementations (real CLI, mock for testing).

pub mod brew;

use crate::error::Result;
use crate::types::{BundleResult, InstalledPackage, Package, PackageType};
use std::path::Path;

/// Backend trait for Homebrew operations.
///
/// This trait abstracts the underlying Homebrew implementation, enabling:
/// - Real CLI execution via `brew` command
/// - Mock implementations for testing
/// - Potential future native integrations
pub trait Backend: Send + Sync {
    /// Check if Homebrew is available.
    fn is_available(&self) -> bool;

    /// Install a package.
    fn install(&self, package: &Package) -> Result<()>;

    /// Uninstall a package.
    fn uninstall(&self, package: &Package) -> Result<()>;

    /// Check if a package is installed.
    fn is_installed(&self, package: &Package) -> Result<bool>;

    /// List all installed packages of a given type.
    fn list_installed(&self, package_type: PackageType) -> Result<Vec<InstalledPackage>>;

    /// Get version info for a package (from `brew info --json`).
    fn get_version(&self, package: &Package) -> Result<Option<String>>;

    /// Run `brew bundle` with a Brewfile.
    fn bundle(&self, brewfile_path: &Path, verbose: bool) -> Result<BundleResult>;

    /// Run `brew update` to refresh package lists.
    fn update(&self) -> Result<()>;

    /// Run `brew upgrade` for a specific package or all packages.
    fn upgrade(&self, package: Option<&Package>) -> Result<()>;

    /// List all installed taps.
    fn list_taps(&self) -> Result<Vec<String>> {
        Ok(self
            .list_installed(PackageType::Tap)?
            .into_iter()
            .map(|p| p.name)
            .collect())
    }

    /// List all installed formulas.
    fn list_formulas(&self) -> Result<Vec<InstalledPackage>> {
        self.list_installed(PackageType::Brew)
    }

    /// List all installed casks.
    fn list_casks(&self) -> Result<Vec<InstalledPackage>> {
        self.list_installed(PackageType::Cask)
    }
}

/// Get the default backend (real brew CLI).
pub fn default_backend() -> Result<brew::BrewBackend> {
    brew::BrewBackend::new()
}
