//! # brewkit
//!
//! Pure Rust library for Homebrew package management.
//!
//! This crate provides functionality for:
//! - Parsing and generating Brewfiles
//! - Installing packages with smart retry logic
//! - Detecting drift between installed packages and Brewfile
//! - Managing taps, formulas, casks, mas apps, and VS Code extensions
//!
//! ## Example
//!
//! ```no_run
//! use brewkit::Client;
//! use std::path::Path;
//!
//! // Create a client
//! let client = Client::new().expect("Homebrew not available");
//!
//! // Parse a Brewfile
//! let brewfile = client.parse_brewfile(Path::new("Brewfile")).expect("Failed to parse");
//!
//! // Check what's missing
//! let audit = client.audit(&brewfile).expect("Audit failed");
//! for pkg in &audit.missing {
//!     println!("Missing: {} ({})", pkg.name, pkg.package_type);
//! }
//!
//! // Install a package
//! use brewkit::Package;
//! let git = Package::brew("git");
//! client.install(&git).expect("Install failed");
//! ```
//!
//! ## Retry Logic
//!
//! Network errors during installation are automatically retried with
//! exponential backoff. Configure retry behavior with [`RetryConfig`].
//!
//! ```no_run
//! use brewkit::{Client, RetryConfig, Package};
//! use std::time::Duration;
//!
//! let client = Client::new().unwrap();
//! let config = RetryConfig::new(3, Duration::from_secs(5), 2.0);
//!
//! let package = Package::brew("curl");
//! client.install_with_retry(&package, &config).unwrap();
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod audit;
pub mod backend;
pub mod brewfile;
pub mod bundle;
pub mod error;
pub mod retry;
pub mod types;

pub use error::{Error, ErrorCategory, Result};
pub use types::{
    AuditResult, Brewfile, BundleResult, InstalledPackage, Package, PackageType, RetryConfig,
};

use backend::{Backend, brew::BrewBackend};
use std::path::Path;

/// High-level client for Homebrew operations.
///
/// The client wraps a backend and provides convenient methods for
/// common operations like installing packages, parsing Brewfiles,
/// and detecting drift.
pub struct Client {
    backend: Box<dyn Backend>,
}

impl Client {
    /// Create a new Client with the default backend.
    ///
    /// Returns an error if Homebrew is not installed.
    pub fn new() -> Result<Self> {
        let backend = BrewBackend::new()?;
        Ok(Self {
            backend: Box::new(backend),
        })
    }

    /// Create a client with a custom backend (useful for testing).
    pub fn with_backend(backend: Box<dyn Backend>) -> Self {
        Self { backend }
    }

    /// Check if Homebrew is available.
    pub fn is_available(&self) -> bool {
        self.backend.is_available()
    }

    // =========================================================================
    // Package Operations
    // =========================================================================

    /// Install a package.
    pub fn install(&self, package: &Package) -> Result<()> {
        self.backend.install(package)
    }

    /// Install a package with retry logic.
    pub fn install_with_retry(&self, package: &Package, config: &RetryConfig) -> Result<()> {
        retry::with_retry(config, Some(&retry::PrintCallback), || {
            self.backend.install(package)
        })
    }

    /// Install a package with retry and custom callback.
    pub fn install_with_retry_callback(
        &self,
        package: &Package,
        config: &RetryConfig,
        callback: &dyn retry::RetryCallback,
    ) -> Result<()> {
        retry::with_retry(config, Some(callback), || self.backend.install(package))
    }

    /// Uninstall a package.
    pub fn uninstall(&self, package: &Package) -> Result<()> {
        self.backend.uninstall(package)
    }

    /// Check if a package is installed.
    pub fn is_installed(&self, package: &Package) -> Result<bool> {
        self.backend.is_installed(package)
    }

    /// Get the installed version of a package.
    pub fn get_version(&self, package: &Package) -> Result<Option<String>> {
        self.backend.get_version(package)
    }

    /// Update Homebrew package lists.
    pub fn update(&self) -> Result<()> {
        self.backend.update()
    }

    /// Upgrade a package (or all packages if None).
    pub fn upgrade(&self, package: Option<&Package>) -> Result<()> {
        self.backend.upgrade(package)
    }

    // =========================================================================
    // List Operations
    // =========================================================================

    /// List all installed packages of a given type.
    pub fn list_installed(&self, package_type: PackageType) -> Result<Vec<InstalledPackage>> {
        self.backend.list_installed(package_type)
    }

    /// List all installed taps.
    pub fn list_taps(&self) -> Result<Vec<String>> {
        self.backend.list_taps()
    }

    /// List all installed formulas.
    pub fn list_formulas(&self) -> Result<Vec<InstalledPackage>> {
        self.backend.list_formulas()
    }

    /// List all installed casks.
    pub fn list_casks(&self) -> Result<Vec<InstalledPackage>> {
        self.backend.list_casks()
    }

    // =========================================================================
    // Brewfile Operations
    // =========================================================================

    /// Parse a Brewfile from a path.
    pub fn parse_brewfile(&self, path: &Path) -> Result<Brewfile> {
        brewfile::parse_file(path)
    }

    /// Parse a Brewfile from a string.
    pub fn parse_brewfile_string(&self, content: &str) -> Result<Brewfile> {
        brewfile::parse_string(content)
    }

    /// Generate a Brewfile from installed packages.
    pub fn capture_brewfile(&self) -> Result<Brewfile> {
        let mut brewfile = Brewfile::new();

        // Taps
        for tap in self.backend.list_taps()? {
            brewfile.add(Package::tap(tap));
        }

        // Formulas (only explicitly installed)
        for pkg in self.backend.list_formulas()? {
            if pkg.installed_on_request {
                brewfile.add(Package::brew(&pkg.name).with_version(&pkg.version));
            }
        }

        // Casks
        for pkg in self.backend.list_casks()? {
            brewfile.add(Package::cask(&pkg.name).with_version(&pkg.version));
        }

        Ok(brewfile)
    }

    /// Write a Brewfile to a path.
    pub fn write_brewfile(&self, brewfile: &Brewfile, path: &Path) -> Result<()> {
        let options = brewfile::WriteOptions {
            include_versions: true,
            group_by_type: true,
            sort_packages: true,
        };
        brewfile::write_file(brewfile, path, &options)?;
        Ok(())
    }

    /// Run `brew bundle` with a Brewfile.
    pub fn bundle(&self, brewfile_path: &Path) -> Result<BundleResult> {
        self.backend.bundle(brewfile_path, true)
    }

    // =========================================================================
    // Audit Operations
    // =========================================================================

    /// Audit installed packages against a Brewfile.
    ///
    /// Returns drift information including:
    /// - Packages installed but not in Brewfile (untracked)
    /// - Packages in Brewfile but not installed (missing)
    /// - Packages with version mismatches
    pub fn audit(&self, brewfile: &Brewfile) -> Result<AuditResult> {
        audit::audit(self.backend.as_ref(), brewfile)
    }

    /// Audit with custom options.
    pub fn audit_with_options(
        &self,
        brewfile: &Brewfile,
        options: &audit::AuditOptions,
    ) -> Result<AuditResult> {
        audit::audit_with_options(self.backend.as_ref(), brewfile, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Most tests require Homebrew to be installed.
    // These are integration tests that would run in CI with brew available.

    #[test]
    fn test_package_constructors() {
        let tap = Package::tap("homebrew/cask");
        assert_eq!(tap.name, "homebrew/cask");
        assert_eq!(tap.package_type, PackageType::Tap);

        let brew = Package::brew("git");
        assert_eq!(brew.name, "git");
        assert_eq!(brew.package_type, PackageType::Brew);

        let cask = Package::cask("firefox");
        assert_eq!(cask.name, "firefox");
        assert_eq!(cask.package_type, PackageType::Cask);
    }

    #[test]
    fn test_parse_brewfile_string() {
        // Create a mock client for testing parsing (doesn't need brew)
        let content = r#"
tap "homebrew/cask"
brew "git" # 2.40.0
cask "firefox"
"#;
        let brewfile = brewfile::parse_string(content).unwrap();

        assert_eq!(brewfile.packages.len(), 3);
        assert_eq!(brewfile.taps().len(), 1);
        assert_eq!(brewfile.brews().len(), 1);
        assert_eq!(brewfile.casks().len(), 1);
    }
}
