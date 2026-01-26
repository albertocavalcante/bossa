//! # toolchain
//!
//! Pure Rust library for installing and managing build tools.
//!
//! This crate provides functionality for:
//! - Installing build tools (Buck2, Bazel, etc.) from official releases
//! - Managing multiple versions of tools
//! - Platform detection for correct binary selection
//! - Automatic decompression (zstd)
//!
//! ## Example
//!
//! ```no_run
//! use toolchain::{Client, Tool, InstallOptions};
//!
//! // Create a client
//! let client = Client::new();
//!
//! // Install Buck2 (latest version)
//! let result = client.install(Tool::Buck2, InstallOptions::default().force(true))
//!     .expect("installation failed");
//!
//! println!("Installed {} {} to {}", result.tool, result.version, result.path.display());
//!
//! // Check if installed
//! assert!(client.is_installed(Tool::Buck2).unwrap());
//!
//! // Get version
//! if let Some(version) = client.version(Tool::Buck2).unwrap() {
//!     println!("Buck2 version: {}", version);
//! }
//! ```
//!
//! ## Supported Tools
//!
//! | Tool  | Source                          | Platforms                    |
//! |-------|---------------------------------|------------------------------|
//! | Buck2 | github.com/facebook/buck2       | macOS, Linux, Windows        |
//!
//! ## Platform Detection
//!
//! The library automatically detects the current platform and downloads
//! the appropriate binary:
//!
//! ```no_run
//! use toolchain::platform;
//!
//! let platform = platform::detect().expect("unsupported platform");
//! println!("Platform: {}", platform.triple);
//! // Output: "aarch64-apple-darwin" (on Apple Silicon Mac)
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod backend;
pub mod error;
pub mod platform;
pub mod tools;
pub mod types;

pub use error::{Error, ErrorCategory, Result};
pub use types::{
    InstallOptions, InstallResult, InstalledTool, Platform, Release, ReleaseAsset, Tool,
};

use backend::Backend;
pub use backend::MockBackend;
use backend::github::GitHubBackend;
use tools::ToolInstaller;
use tools::buck2::Buck2Installer;

/// High-level client for toolchain operations.
///
/// The client provides a simple interface for installing and managing
/// build tools across different platforms.
///
/// # Example
///
/// ```no_run
/// use toolchain::{Client, Tool, InstallOptions};
///
/// let client = Client::new();
///
/// // Check if a tool is installed
/// if !client.is_installed(Tool::Buck2).unwrap() {
///     // Install it
///     client.install(Tool::Buck2, InstallOptions::default().force(true)).unwrap();
/// }
/// ```
pub struct Client {
    backend: Box<dyn Backend>,
}

impl Client {
    /// Create a new Client with the default GitHub backend.
    #[must_use]
    pub fn new() -> Self {
        Self {
            backend: Box::new(GitHubBackend::new()),
        }
    }

    /// Create a client with a custom backend (useful for testing).
    #[must_use]
    pub fn with_backend(backend: Box<dyn Backend>) -> Self {
        Self { backend }
    }

    // =========================================================================
    // Installation Operations
    // =========================================================================

    /// Install a tool.
    ///
    /// Downloads the appropriate binary for the current platform and installs
    /// it to the specified (or default) location.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use toolchain::{Client, Tool, InstallOptions};
    ///
    /// let client = Client::new();
    ///
    /// // Install latest
    /// client.install(Tool::Buck2, InstallOptions::default().force(true)).unwrap();
    ///
    /// // Install specific version
    /// client.install(Tool::Buck2, InstallOptions::new().version("2024-01-15")).unwrap();
    /// ```
    pub fn install(&self, tool: Tool, options: InstallOptions) -> Result<InstallResult> {
        // Detect platform
        let platform = platform::detect()?;

        // Fetch the release
        let tag = options.version.as_deref().unwrap_or("latest");
        let release = self.backend.fetch_release(tool, tag)?;

        // Download the asset
        let compressed = self.backend.download_asset(tool, &release, &platform)?;

        // Get the appropriate installer
        let installer = self.get_installer(tool);

        // Install
        installer.install(&compressed, &platform, &options)
    }

    /// Check if a tool is installed.
    pub fn is_installed(&self, tool: Tool) -> Result<bool> {
        let installer = self.get_installer(tool);
        installer.is_installed()
    }

    /// Get the installed version of a tool.
    pub fn version(&self, tool: Tool) -> Result<Option<String>> {
        let installer = self.get_installer(tool);
        installer.installed_version()
    }

    // =========================================================================
    // Release Information
    // =========================================================================

    /// List available releases for a tool.
    ///
    /// Returns releases from newest to oldest.
    pub fn list_releases(&self, tool: Tool) -> Result<Vec<Release>> {
        self.backend.fetch_releases(tool)
    }

    /// Get information about a specific release.
    pub fn get_release(&self, tool: Tool, tag: &str) -> Result<Release> {
        self.backend.fetch_release(tool, tag)
    }

    // =========================================================================
    // Internal
    // =========================================================================

    /// Get the installer for a tool.
    fn get_installer(&self, tool: Tool) -> Box<dyn ToolInstaller> {
        match tool {
            Tool::Buck2 => Box::new(Buck2Installer::new()),
        }
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;

    #[test]
    fn test_client_creation() {
        let client = Client::new();
        // Just verify it doesn't panic
        let _ = client;
    }

    #[test]
    fn test_client_default() {
        let client = Client::default();
        // Verify it creates a valid client
        let _ = client;
    }

    #[test]
    fn test_client_with_mock_backend() {
        let mock = MockBackend::with_buck2_releases();
        let client = Client::with_backend(Box::new(mock));

        // Verify we can list releases
        let releases = client.list_releases(Tool::Buck2).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "2024-01-15");
    }

    #[test]
    fn test_client_get_release() {
        let mock = MockBackend::with_buck2_releases();
        let client = Client::with_backend(Box::new(mock));

        let release = client.get_release(Tool::Buck2, "2024-01-15").unwrap();
        assert_eq!(release.tag, "2024-01-15");
        assert!(!release.assets.is_empty());
    }

    #[test]
    fn test_client_get_release_not_found() {
        let mock = MockBackend::new();
        let client = Client::with_backend(Box::new(mock));

        let result = client.get_release(Tool::Buck2, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_tool_display() {
        assert_eq!(Tool::Buck2.name(), "buck2");
        assert_eq!(Tool::Buck2.github_repo(), "facebook/buck2");
    }

    #[test]
    fn test_tool_all() {
        let tools = Tool::all();
        assert!(!tools.is_empty());
        assert!(tools.contains(&Tool::Buck2));
    }
}
