//! Core types for toolchain management.
//!
//! This module contains the fundamental data structures used throughout
//! the toolchain crate, including tool definitions, platform information,
//! installation options, and result types.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Supported build tools.
///
/// This enum represents all build tools that can be installed and managed
/// by this crate. Each tool has associated metadata like its name, GitHub
/// repository, and binary name.
///
/// # Example
///
/// ```
/// use toolchain::Tool;
///
/// let tool = Tool::Buck2;
/// assert_eq!(tool.name(), "buck2");
/// assert_eq!(tool.github_repo(), "facebook/buck2");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    /// Meta's Buck2 build system.
    ///
    /// Buck2 is a fast, hermetic build system from Meta that supports
    /// large-scale monorepos with excellent caching and remote execution.
    Buck2,
    // Future: Bazel, Pants, Please, etc.
}

impl Tool {
    /// Get the tool name as a string.
    ///
    /// Returns the lowercase identifier used for the tool in file names
    /// and display output.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Buck2 => "buck2",
        }
    }

    /// Get the GitHub repository for this tool.
    ///
    /// Returns the repository in "owner/repo" format.
    #[must_use]
    pub fn github_repo(&self) -> &'static str {
        match self {
            Self::Buck2 => "facebook/buck2",
        }
    }

    /// Get the binary name for this tool.
    ///
    /// Returns the name of the executable binary (without extension).
    #[must_use]
    pub fn binary_name(&self) -> &'static str {
        match self {
            Self::Buck2 => "buck2",
        }
    }

    /// Get all supported tools.
    ///
    /// Returns an iterator over all tool variants.
    #[must_use]
    pub fn all() -> &'static [Tool] {
        &[Tool::Buck2]
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Target platform for binary downloads.
///
/// Represents a target platform with OS, architecture, and triple information
/// used for selecting the correct binary to download.
///
/// # Example
///
/// ```
/// use toolchain::Platform;
///
/// let platform = Platform::new("macos", "aarch64", "aarch64-apple-darwin");
/// assert_eq!(platform.os, "macos");
/// assert!(platform.is_macos());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Platform {
    /// Operating system (e.g., "macos", "linux", "windows").
    pub os: String,
    /// CPU architecture (e.g., "aarch64", "x86_64").
    pub arch: String,
    /// Platform triple (e.g., "aarch64-apple-darwin").
    pub triple: String,
}

impl Platform {
    /// Create a new platform.
    #[must_use]
    pub fn new(os: impl Into<String>, arch: impl Into<String>, triple: impl Into<String>) -> Self {
        Self {
            os: os.into(),
            arch: arch.into(),
            triple: triple.into(),
        }
    }

    /// Check if this platform is macOS.
    #[must_use]
    pub fn is_macos(&self) -> bool {
        self.os == "macos"
    }

    /// Check if this platform is Linux.
    #[must_use]
    pub fn is_linux(&self) -> bool {
        self.os == "linux"
    }

    /// Check if this platform is Windows.
    #[must_use]
    pub fn is_windows(&self) -> bool {
        self.os == "windows"
    }

    /// Check if this platform uses ARM architecture.
    #[must_use]
    pub fn is_arm(&self) -> bool {
        self.arch == "aarch64"
    }

    /// Check if this platform uses x86_64 architecture.
    #[must_use]
    pub fn is_x86_64(&self) -> bool {
        self.arch == "x86_64"
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.triple)
    }
}

/// Information about an installed tool.
///
/// Contains metadata about a tool installation, including version,
/// path, and installation timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledTool {
    /// The tool.
    pub tool: Tool,
    /// Installed version (tag or "latest").
    pub version: String,
    /// Path to the installed binary.
    pub path: PathBuf,
    /// Installation timestamp (ISO 8601 format).
    pub installed_at: String,
}

impl InstalledTool {
    /// Create a new InstalledTool.
    #[must_use]
    pub fn new(tool: Tool, version: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            tool,
            version: version.into(),
            path: path.into(),
            installed_at: String::new(),
        }
    }
}

/// A release available for download.
///
/// Represents a GitHub release with its metadata and downloadable assets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    /// Release tag (e.g., "latest", "2024-01-15").
    pub tag: String,
    /// Release name.
    pub name: String,
    /// Whether this is a prerelease.
    pub prerelease: bool,
    /// Published date (ISO 8601 format).
    pub published_at: String,
    /// Available assets.
    pub assets: Vec<ReleaseAsset>,
}

impl Release {
    /// Find an asset by name.
    #[must_use]
    pub fn find_asset(&self, name: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|a| a.name == name)
    }

    /// Find an asset matching a platform triple.
    ///
    /// Looks for assets with names containing the platform triple.
    #[must_use]
    pub fn find_asset_for_platform(&self, triple: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|a| a.name.contains(triple))
    }
}

/// An asset within a release.
///
/// Represents a downloadable file (binary, archive, etc.) from a release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseAsset {
    /// Asset name (e.g., "buck2-aarch64-apple-darwin.zst").
    pub name: String,
    /// Download URL.
    pub download_url: String,
    /// Size in bytes.
    pub size: u64,
}

impl ReleaseAsset {
    /// Get the file extension of this asset.
    ///
    /// Returns `None` if the file has no extension.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let name = &self.name;
        match name.rfind('.') {
            Some(idx) if idx > 0 && idx < name.len() - 1 => Some(&name[idx + 1..]),
            _ => None,
        }
    }

    /// Check if this asset is a zstd-compressed file.
    #[must_use]
    pub fn is_zstd(&self) -> bool {
        self.name.ends_with(".zst") || self.name.ends_with(".zstd")
    }

    /// Get a human-readable size string.
    #[must_use]
    pub fn human_size(&self) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if self.size >= GB {
            format!("{:.1} GB", self.size as f64 / GB as f64)
        } else if self.size >= MB {
            format!("{:.1} MB", self.size as f64 / MB as f64)
        } else if self.size >= KB {
            format!("{:.1} KB", self.size as f64 / KB as f64)
        } else {
            format!("{} B", self.size)
        }
    }
}

/// Options for installation.
///
/// Use the builder pattern to configure installation options.
///
/// # Example
///
/// ```
/// use toolchain::InstallOptions;
/// use std::path::PathBuf;
///
/// let options = InstallOptions::new()
///     .version("2024-01-15")
///     .install_dir("/usr/local/bin")
///     .force(true);
///
/// assert_eq!(options.version, Some("2024-01-15".to_string()));
/// assert!(options.force);
/// ```
#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Version to install (None = latest).
    pub version: Option<String>,
    /// Installation directory (None = default).
    pub install_dir: Option<PathBuf>,
    /// Whether to overwrite existing installation.
    pub force: bool,
}

impl InstallOptions {
    /// Create new install options with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the version to install.
    ///
    /// Pass the release tag, e.g., "2024-01-15" or "latest".
    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set the installation directory.
    ///
    /// If not set, defaults to `~/.local/bin` or `/usr/local/bin`.
    #[must_use]
    pub fn install_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.install_dir = Some(dir.into());
        self
    }

    /// Set whether to force reinstall.
    ///
    /// When true, overwrites existing installations without prompting.
    #[must_use]
    pub fn force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    /// Check if a specific version was requested.
    #[must_use]
    pub fn has_version(&self) -> bool {
        self.version.is_some()
    }
}

/// Result of an installation operation.
///
/// Contains information about the completed installation, including
/// the installed version and path, and whether it was an upgrade.
#[derive(Debug, Clone)]
pub struct InstallResult {
    /// The tool that was installed.
    pub tool: Tool,
    /// The version that was installed.
    pub version: String,
    /// Path to the installed binary.
    pub path: PathBuf,
    /// Whether this was a fresh install or upgrade.
    pub was_upgrade: bool,
    /// Previous version if this was an upgrade.
    pub previous_version: Option<String>,
}

impl InstallResult {
    /// Check if this installation was an upgrade from a different version.
    #[must_use]
    pub fn is_version_change(&self) -> bool {
        match &self.previous_version {
            Some(prev) => prev != &self.version,
            None => false,
        }
    }
}

impl fmt::Display for InstallResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.was_upgrade {
            if let Some(prev) = &self.previous_version {
                write!(
                    f,
                    "{} upgraded from {} to {} at {}",
                    self.tool,
                    prev,
                    self.version,
                    self.path.display()
                )
            } else {
                write!(
                    f,
                    "{} {} reinstalled at {}",
                    self.tool,
                    self.version,
                    self.path.display()
                )
            }
        } else {
            write!(
                f,
                "{} {} installed at {}",
                self.tool,
                self.version,
                self.path.display()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Tool tests
    // =========================================================================

    #[test]
    fn test_tool_name() {
        assert_eq!(Tool::Buck2.name(), "buck2");
    }

    #[test]
    fn test_tool_github_repo() {
        assert_eq!(Tool::Buck2.github_repo(), "facebook/buck2");
    }

    #[test]
    fn test_tool_binary_name() {
        assert_eq!(Tool::Buck2.binary_name(), "buck2");
    }

    #[test]
    fn test_tool_display() {
        assert_eq!(format!("{}", Tool::Buck2), "buck2");
    }

    #[test]
    fn test_tool_all() {
        let all = Tool::all();
        assert!(!all.is_empty());
        assert!(all.contains(&Tool::Buck2));
    }

    #[test]
    fn test_tool_serialization() {
        let tool = Tool::Buck2;
        let json = serde_json::to_string(&tool).unwrap();
        assert_eq!(json, "\"buck2\"");

        let parsed: Tool = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Tool::Buck2);
    }

    // =========================================================================
    // Platform tests
    // =========================================================================

    #[test]
    fn test_platform_new() {
        let platform = Platform::new("macos", "aarch64", "aarch64-apple-darwin");
        assert_eq!(platform.os, "macos");
        assert_eq!(platform.arch, "aarch64");
        assert_eq!(platform.triple, "aarch64-apple-darwin");
    }

    #[test]
    fn test_platform_is_macos() {
        let macos = Platform::new("macos", "aarch64", "aarch64-apple-darwin");
        let linux = Platform::new("linux", "x86_64", "x86_64-unknown-linux-gnu");

        assert!(macos.is_macos());
        assert!(!linux.is_macos());
    }

    #[test]
    fn test_platform_is_linux() {
        let macos = Platform::new("macos", "aarch64", "aarch64-apple-darwin");
        let linux = Platform::new("linux", "x86_64", "x86_64-unknown-linux-gnu");

        assert!(!macos.is_linux());
        assert!(linux.is_linux());
    }

    #[test]
    fn test_platform_is_windows() {
        let windows = Platform::new("windows", "x86_64", "x86_64-pc-windows-msvc");
        let linux = Platform::new("linux", "x86_64", "x86_64-unknown-linux-gnu");

        assert!(windows.is_windows());
        assert!(!linux.is_windows());
    }

    #[test]
    fn test_platform_is_arm() {
        let arm = Platform::new("macos", "aarch64", "aarch64-apple-darwin");
        let x86 = Platform::new("macos", "x86_64", "x86_64-apple-darwin");

        assert!(arm.is_arm());
        assert!(!x86.is_arm());
    }

    #[test]
    fn test_platform_is_x86_64() {
        let arm = Platform::new("macos", "aarch64", "aarch64-apple-darwin");
        let x86 = Platform::new("macos", "x86_64", "x86_64-apple-darwin");

        assert!(!arm.is_x86_64());
        assert!(x86.is_x86_64());
    }

    #[test]
    fn test_platform_display() {
        let platform = Platform::new("macos", "aarch64", "aarch64-apple-darwin");
        assert_eq!(format!("{platform}"), "aarch64-apple-darwin");
    }

    // =========================================================================
    // Release and ReleaseAsset tests
    // =========================================================================

    fn create_test_release() -> Release {
        Release {
            tag: "2024-01-15".to_string(),
            name: "Release 2024-01-15".to_string(),
            prerelease: false,
            published_at: "2024-01-15T00:00:00Z".to_string(),
            assets: vec![
                ReleaseAsset {
                    name: "buck2-aarch64-apple-darwin.zst".to_string(),
                    download_url: "https://example.com/buck2-aarch64-apple-darwin.zst".to_string(),
                    size: 50 * 1024 * 1024, // 50 MB
                },
                ReleaseAsset {
                    name: "buck2-x86_64-unknown-linux-gnu.zst".to_string(),
                    download_url: "https://example.com/buck2-x86_64-unknown-linux-gnu.zst"
                        .to_string(),
                    size: 52 * 1024 * 1024, // 52 MB
                },
            ],
        }
    }

    #[test]
    fn test_release_find_asset() {
        let release = create_test_release();

        let asset = release.find_asset("buck2-aarch64-apple-darwin.zst");
        assert!(asset.is_some());
        assert_eq!(asset.unwrap().name, "buck2-aarch64-apple-darwin.zst");

        let not_found = release.find_asset("nonexistent.zst");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_release_find_asset_for_platform() {
        let release = create_test_release();

        let asset = release.find_asset_for_platform("aarch64-apple-darwin");
        assert!(asset.is_some());

        let linux_asset = release.find_asset_for_platform("x86_64-unknown-linux-gnu");
        assert!(linux_asset.is_some());

        let not_found = release.find_asset_for_platform("riscv64");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_release_asset_extension() {
        let asset = ReleaseAsset {
            name: "buck2-aarch64-apple-darwin.zst".to_string(),
            download_url: "https://example.com/asset.zst".to_string(),
            size: 1024,
        };
        assert_eq!(asset.extension(), Some("zst"));

        let no_ext = ReleaseAsset {
            name: "README".to_string(),
            download_url: "https://example.com/README".to_string(),
            size: 100,
        };
        assert_eq!(no_ext.extension(), None);

        let hidden_file = ReleaseAsset {
            name: ".gitignore".to_string(),
            download_url: "https://example.com/.gitignore".to_string(),
            size: 50,
        };
        assert_eq!(hidden_file.extension(), None);

        let trailing_dot = ReleaseAsset {
            name: "file.".to_string(),
            download_url: "https://example.com/file.".to_string(),
            size: 50,
        };
        assert_eq!(trailing_dot.extension(), None);
    }

    #[test]
    fn test_release_asset_is_zstd() {
        let zst = ReleaseAsset {
            name: "file.zst".to_string(),
            download_url: String::new(),
            size: 0,
        };
        let zstd = ReleaseAsset {
            name: "file.zstd".to_string(),
            download_url: String::new(),
            size: 0,
        };
        let tar = ReleaseAsset {
            name: "file.tar.gz".to_string(),
            download_url: String::new(),
            size: 0,
        };

        assert!(zst.is_zstd());
        assert!(zstd.is_zstd());
        assert!(!tar.is_zstd());
    }

    #[test]
    fn test_release_asset_human_size() {
        let bytes = ReleaseAsset {
            name: "small".to_string(),
            download_url: String::new(),
            size: 500,
        };
        assert_eq!(bytes.human_size(), "500 B");

        let kb = ReleaseAsset {
            name: "kb".to_string(),
            download_url: String::new(),
            size: 2048,
        };
        assert_eq!(kb.human_size(), "2.0 KB");

        let mb = ReleaseAsset {
            name: "mb".to_string(),
            download_url: String::new(),
            size: 50 * 1024 * 1024,
        };
        assert_eq!(mb.human_size(), "50.0 MB");

        let gb = ReleaseAsset {
            name: "gb".to_string(),
            download_url: String::new(),
            size: 2 * 1024 * 1024 * 1024,
        };
        assert_eq!(gb.human_size(), "2.0 GB");
    }

    // =========================================================================
    // InstallOptions tests
    // =========================================================================

    #[test]
    fn test_install_options_default() {
        let options = InstallOptions::default();
        assert!(options.version.is_none());
        assert!(options.install_dir.is_none());
        assert!(!options.force);
    }

    #[test]
    fn test_install_options_builder() {
        let options = InstallOptions::new()
            .version("2024-01-15")
            .install_dir("/usr/local/bin")
            .force(true);

        assert_eq!(options.version, Some("2024-01-15".to_string()));
        assert_eq!(options.install_dir, Some(PathBuf::from("/usr/local/bin")));
        assert!(options.force);
    }

    #[test]
    fn test_install_options_has_version() {
        let without = InstallOptions::new();
        let with = InstallOptions::new().version("2024-01-15");

        assert!(!without.has_version());
        assert!(with.has_version());
    }

    // =========================================================================
    // InstallResult tests
    // =========================================================================

    #[test]
    fn test_install_result_is_version_change() {
        let fresh_install = InstallResult {
            tool: Tool::Buck2,
            version: "2024-01-15".to_string(),
            path: PathBuf::from("/usr/local/bin/buck2"),
            was_upgrade: false,
            previous_version: None,
        };
        assert!(!fresh_install.is_version_change());

        let upgrade = InstallResult {
            tool: Tool::Buck2,
            version: "2024-01-15".to_string(),
            path: PathBuf::from("/usr/local/bin/buck2"),
            was_upgrade: true,
            previous_version: Some("2024-01-01".to_string()),
        };
        assert!(upgrade.is_version_change());

        let reinstall = InstallResult {
            tool: Tool::Buck2,
            version: "2024-01-15".to_string(),
            path: PathBuf::from("/usr/local/bin/buck2"),
            was_upgrade: true,
            previous_version: Some("2024-01-15".to_string()),
        };
        assert!(!reinstall.is_version_change());
    }

    #[test]
    fn test_install_result_display_fresh() {
        let result = InstallResult {
            tool: Tool::Buck2,
            version: "2024-01-15".to_string(),
            path: PathBuf::from("/usr/local/bin/buck2"),
            was_upgrade: false,
            previous_version: None,
        };
        let display = format!("{result}");
        assert!(display.contains("buck2"));
        assert!(display.contains("2024-01-15"));
        assert!(display.contains("installed"));
    }

    #[test]
    fn test_install_result_display_upgrade() {
        let result = InstallResult {
            tool: Tool::Buck2,
            version: "2024-01-15".to_string(),
            path: PathBuf::from("/usr/local/bin/buck2"),
            was_upgrade: true,
            previous_version: Some("2024-01-01".to_string()),
        };
        let display = format!("{result}");
        assert!(display.contains("upgraded"));
        assert!(display.contains("2024-01-01"));
        assert!(display.contains("2024-01-15"));
    }

    // =========================================================================
    // InstalledTool tests
    // =========================================================================

    #[test]
    fn test_installed_tool_new() {
        let installed = InstalledTool::new(Tool::Buck2, "2024-01-15", "/usr/local/bin/buck2");
        assert_eq!(installed.tool, Tool::Buck2);
        assert_eq!(installed.version, "2024-01-15");
        assert_eq!(installed.path, PathBuf::from("/usr/local/bin/buck2"));
    }
}
