//! Core types for Homebrew package management.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Type of Homebrew package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageType {
    /// Homebrew tap (third-party repository)
    Tap,
    /// Homebrew formula (CLI tool)
    Brew,
    /// Homebrew cask (GUI application)
    Cask,
    /// Mac App Store app (via mas)
    Mas,
    /// VS Code extension
    Vscode,
}

impl PackageType {
    /// Get the Brewfile directive name for this package type.
    pub fn directive(&self) -> &'static str {
        match self {
            PackageType::Tap => "tap",
            PackageType::Brew => "brew",
            PackageType::Cask => "cask",
            PackageType::Mas => "mas",
            PackageType::Vscode => "vscode",
        }
    }

    /// Parse a package type from a Brewfile directive.
    pub fn from_directive(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "tap" => Some(PackageType::Tap),
            "brew" => Some(PackageType::Brew),
            "cask" => Some(PackageType::Cask),
            "mas" => Some(PackageType::Mas),
            "vscode" => Some(PackageType::Vscode),
            _ => None,
        }
    }
}

impl std::fmt::Display for PackageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.directive())
    }
}

/// A package definition from a Brewfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    /// Package name (e.g., "git", "homebrew/cask-fonts")
    pub name: String,
    /// Type of package
    pub package_type: PackageType,
    /// Optional version (from comment or pin)
    pub version: Option<String>,
    /// Additional options (e.g., restart_service: :changed)
    pub options: HashMap<String, String>,
}

impl Package {
    /// Create a new package with the given name and type.
    pub fn new(name: impl Into<String>, package_type: PackageType) -> Self {
        Self {
            name: name.into(),
            package_type,
            version: None,
            options: HashMap::new(),
        }
    }

    /// Create a tap package.
    pub fn tap(name: impl Into<String>) -> Self {
        Self::new(name, PackageType::Tap)
    }

    /// Create a brew formula package.
    pub fn brew(name: impl Into<String>) -> Self {
        Self::new(name, PackageType::Brew)
    }

    /// Create a cask package.
    pub fn cask(name: impl Into<String>) -> Self {
        Self::new(name, PackageType::Cask)
    }

    /// Create a mas app package.
    pub fn mas(name: impl Into<String>, id: impl Into<String>) -> Self {
        let mut pkg = Self::new(name, PackageType::Mas);
        pkg.options.insert("id".to_string(), id.into());
        pkg
    }

    /// Create a vscode extension package.
    pub fn vscode(name: impl Into<String>) -> Self {
        Self::new(name, PackageType::Vscode)
    }

    /// Set the version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Add an option.
    pub fn with_option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }

    /// Get the mas app ID if this is a mas package.
    pub fn mas_id(&self) -> Option<&str> {
        if self.package_type == PackageType::Mas {
            self.options.get("id").map(|s| s.as_str())
        } else {
            None
        }
    }
}

/// Information about an installed package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    /// Package name
    pub name: String,
    /// Package type
    pub package_type: PackageType,
    /// Installed version
    pub version: String,
    /// Whether this package was explicitly installed (not a dependency)
    pub installed_on_request: bool,
}

/// Configuration for retry logic.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_attempts: u32,
    /// Base delay between retries
    pub base_delay: Duration,
    /// Multiplier for exponential backoff
    pub backoff_factor: f64,
    /// Maximum delay between retries
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_secs(10),
            backoff_factor: 2.0,
            max_delay: Duration::from_secs(300), // 5 minutes max
        }
    }
}

impl RetryConfig {
    /// Create a new retry config with custom settings.
    pub fn new(max_attempts: u32, base_delay: Duration, backoff_factor: f64) -> Self {
        Self {
            max_attempts,
            base_delay,
            backoff_factor,
            max_delay: Duration::from_secs(300),
        }
    }

    /// Calculate the delay for a given attempt number (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let delay = self.base_delay.as_secs_f64() * self.backoff_factor.powi(attempt as i32);
        let capped = delay.min(self.max_delay.as_secs_f64());
        Duration::from_secs_f64(capped)
    }

    /// Create a config that never retries.
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            ..Default::default()
        }
    }
}

/// Result of a bundle operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BundleResult {
    /// Packages that were successfully installed
    pub installed: Vec<String>,
    /// Packages that failed to install
    pub failed: Vec<(String, String)>,
    /// Packages that were already installed
    pub skipped: Vec<String>,
    /// Packages that were upgraded
    pub upgraded: Vec<String>,
}

impl BundleResult {
    /// Check if all packages were handled successfully (installed or skipped).
    pub fn is_success(&self) -> bool {
        self.failed.is_empty()
    }

    /// Total number of packages processed.
    pub fn total(&self) -> usize {
        self.installed.len() + self.failed.len() + self.skipped.len() + self.upgraded.len()
    }

    /// Merge another result into this one.
    pub fn merge(&mut self, other: BundleResult) {
        self.installed.extend(other.installed);
        self.failed.extend(other.failed);
        self.skipped.extend(other.skipped);
        self.upgraded.extend(other.upgraded);
    }
}

/// Drift detection result comparing installed packages to Brewfile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditResult {
    /// Packages installed but not in Brewfile
    pub untracked: Vec<InstalledPackage>,
    /// Packages in Brewfile but not installed
    pub missing: Vec<Package>,
    /// Packages with version mismatch
    pub mismatched: Vec<(Package, InstalledPackage)>,
}

impl AuditResult {
    /// Check if there is any drift.
    pub fn has_drift(&self) -> bool {
        !self.untracked.is_empty() || !self.missing.is_empty() || !self.mismatched.is_empty()
    }
}

/// Parsed Brewfile representation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Brewfile {
    /// Path to the Brewfile
    pub path: Option<PathBuf>,
    /// All packages in the Brewfile
    pub packages: Vec<Package>,
}

impl Brewfile {
    /// Create an empty Brewfile.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a Brewfile with a path.
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            packages: Vec::new(),
        }
    }

    /// Add a package.
    pub fn add(&mut self, package: Package) {
        self.packages.push(package);
    }

    /// Get all packages of a specific type.
    pub fn packages_of_type(&self, package_type: PackageType) -> Vec<&Package> {
        self.packages
            .iter()
            .filter(|p| p.package_type == package_type)
            .collect()
    }

    /// Get taps.
    pub fn taps(&self) -> Vec<&Package> {
        self.packages_of_type(PackageType::Tap)
    }

    /// Get formulas.
    pub fn brews(&self) -> Vec<&Package> {
        self.packages_of_type(PackageType::Brew)
    }

    /// Get casks.
    pub fn casks(&self) -> Vec<&Package> {
        self.packages_of_type(PackageType::Cask)
    }

    /// Get mas apps.
    pub fn mas_apps(&self) -> Vec<&Package> {
        self.packages_of_type(PackageType::Mas)
    }

    /// Get vscode extensions.
    pub fn vscode_extensions(&self) -> Vec<&Package> {
        self.packages_of_type(PackageType::Vscode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_type_directive() {
        assert_eq!(PackageType::Tap.directive(), "tap");
        assert_eq!(PackageType::Brew.directive(), "brew");
        assert_eq!(PackageType::Cask.directive(), "cask");
        assert_eq!(PackageType::Mas.directive(), "mas");
        assert_eq!(PackageType::Vscode.directive(), "vscode");
    }

    #[test]
    fn test_package_type_from_directive() {
        assert_eq!(PackageType::from_directive("tap"), Some(PackageType::Tap));
        assert_eq!(PackageType::from_directive("BREW"), Some(PackageType::Brew));
        assert_eq!(PackageType::from_directive("unknown"), None);
    }

    #[test]
    fn test_package_constructors() {
        let tap = Package::tap("homebrew/cask");
        assert_eq!(tap.package_type, PackageType::Tap);
        assert_eq!(tap.name, "homebrew/cask");

        let brew = Package::brew("git").with_version("2.40.0");
        assert_eq!(brew.package_type, PackageType::Brew);
        assert_eq!(brew.version, Some("2.40.0".to_string()));

        let mas = Package::mas("Xcode", "497799835");
        assert_eq!(mas.package_type, PackageType::Mas);
        assert_eq!(mas.mas_id(), Some("497799835"));
    }

    #[test]
    fn test_retry_config_delay() {
        let config = RetryConfig::new(5, Duration::from_secs(10), 2.0);

        assert_eq!(config.delay_for_attempt(0), Duration::from_secs(10));
        assert_eq!(config.delay_for_attempt(1), Duration::from_secs(20));
        assert_eq!(config.delay_for_attempt(2), Duration::from_secs(40));
        assert_eq!(config.delay_for_attempt(3), Duration::from_secs(80));
        assert_eq!(config.delay_for_attempt(4), Duration::from_secs(160));
    }

    #[test]
    fn test_retry_config_max_delay() {
        let config = RetryConfig {
            max_delay: Duration::from_secs(30),
            ..RetryConfig::new(5, Duration::from_secs(10), 2.0)
        };

        // Should cap at 30 seconds
        assert_eq!(config.delay_for_attempt(2), Duration::from_secs(30));
        assert_eq!(config.delay_for_attempt(3), Duration::from_secs(30));
    }

    #[test]
    fn test_bundle_result() {
        let mut result = BundleResult::default();
        assert!(result.is_success());

        result.installed.push("git".to_string());
        result.skipped.push("curl".to_string());
        assert!(result.is_success());
        assert_eq!(result.total(), 2);

        result
            .failed
            .push(("foo".to_string(), "not found".to_string()));
        assert!(!result.is_success());
        assert_eq!(result.total(), 3);
    }

    #[test]
    fn test_brewfile_packages_by_type() {
        let mut brewfile = Brewfile::new();
        brewfile.add(Package::tap("homebrew/cask"));
        brewfile.add(Package::brew("git"));
        brewfile.add(Package::brew("curl"));
        brewfile.add(Package::cask("visual-studio-code"));

        assert_eq!(brewfile.taps().len(), 1);
        assert_eq!(brewfile.brews().len(), 2);
        assert_eq!(brewfile.casks().len(), 1);
        assert_eq!(brewfile.mas_apps().len(), 0);
    }

    #[test]
    fn test_audit_result_has_drift() {
        let mut result = AuditResult::default();
        assert!(!result.has_drift());

        result.untracked.push(InstalledPackage {
            name: "foo".to_string(),
            package_type: PackageType::Brew,
            version: "1.0".to_string(),
            installed_on_request: true,
        });
        assert!(result.has_drift());
    }
}
