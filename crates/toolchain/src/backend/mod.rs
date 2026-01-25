//! Backend traits and implementations for fetching tool releases.
//!
//! This module provides the [`Backend`] trait and implementations for
//! different release sources. The primary implementation is [`github::GitHubBackend`]
//! for fetching releases from GitHub.
//!
//! # Testing
//!
//! Use [`MockBackend`] for testing without network access:
//!
//! ```
//! use toolchain::backend::{Backend, MockBackend};
//! use toolchain::{Tool, Platform, Release, ReleaseAsset};
//!
//! let mut mock = MockBackend::new();
//! mock.add_release(Tool::Buck2, Release {
//!     tag: "2024-01-15".to_string(),
//!     name: "Release 2024-01-15".to_string(),
//!     prerelease: false,
//!     published_at: "2024-01-15T00:00:00Z".to_string(),
//!     assets: vec![],
//! });
//!
//! let releases = mock.fetch_releases(Tool::Buck2).unwrap();
//! assert_eq!(releases.len(), 1);
//! ```

pub mod github;

use crate::error::{Error, Result};
use crate::types::{Platform, Release, ReleaseAsset, Tool};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Backend trait for fetching releases.
///
/// This abstraction allows for different sources of releases
/// (GitHub, local cache, mirrors, etc.) and enables testing.
pub trait Backend: Send + Sync {
    /// Fetch available releases for a tool.
    ///
    /// Returns releases sorted from newest to oldest.
    fn fetch_releases(&self, tool: Tool) -> Result<Vec<Release>>;

    /// Fetch a specific release by tag.
    ///
    /// # Errors
    ///
    /// Returns `Error::VersionNotFound` if the tag doesn't exist.
    fn fetch_release(&self, tool: Tool, tag: &str) -> Result<Release>;

    /// Download a release asset.
    ///
    /// Returns the raw (possibly compressed) bytes of the asset.
    ///
    /// # Errors
    ///
    /// Returns `Error::DownloadFailed` if the asset cannot be downloaded.
    fn download_asset(&self, tool: Tool, release: &Release, platform: &Platform) -> Result<Vec<u8>>;
}

/// Mock backend for testing without network access.
///
/// This backend stores releases and assets in memory and can be
/// configured to return specific responses for testing.
#[derive(Debug, Clone, Default)]
pub struct MockBackend {
    releases: Arc<Mutex<HashMap<Tool, Vec<Release>>>>,
    assets: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl MockBackend {
    /// Create a new empty mock backend.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a release for a tool.
    pub fn add_release(&mut self, tool: Tool, release: Release) {
        let mut releases = self.releases.lock().unwrap();
        releases.entry(tool).or_default().push(release);
    }

    /// Set all releases for a tool.
    pub fn set_releases(&mut self, tool: Tool, releases: Vec<Release>) {
        let mut all_releases = self.releases.lock().unwrap();
        all_releases.insert(tool, releases);
    }

    /// Add asset data for a given asset name.
    pub fn add_asset(&mut self, name: impl Into<String>, data: Vec<u8>) {
        let mut assets = self.assets.lock().unwrap();
        assets.insert(name.into(), data);
    }

    /// Create a mock backend pre-configured with Buck2 releases.
    #[must_use]
    pub fn with_buck2_releases() -> Self {
        let mut mock = Self::new();

        // Add a sample release
        mock.add_release(
            Tool::Buck2,
            Release {
                tag: "2024-01-15".to_string(),
                name: "Release 2024-01-15".to_string(),
                prerelease: false,
                published_at: "2024-01-15T00:00:00Z".to_string(),
                assets: vec![
                    ReleaseAsset {
                        name: "buck2-aarch64-apple-darwin.zst".to_string(),
                        download_url: "mock://buck2-aarch64-apple-darwin.zst".to_string(),
                        size: 50 * 1024 * 1024,
                    },
                    ReleaseAsset {
                        name: "buck2-x86_64-apple-darwin.zst".to_string(),
                        download_url: "mock://buck2-x86_64-apple-darwin.zst".to_string(),
                        size: 52 * 1024 * 1024,
                    },
                    ReleaseAsset {
                        name: "buck2-x86_64-unknown-linux-gnu.zst".to_string(),
                        download_url: "mock://buck2-x86_64-unknown-linux-gnu.zst".to_string(),
                        size: 55 * 1024 * 1024,
                    },
                    ReleaseAsset {
                        name: "buck2-aarch64-unknown-linux-gnu.zst".to_string(),
                        download_url: "mock://buck2-aarch64-unknown-linux-gnu.zst".to_string(),
                        size: 54 * 1024 * 1024,
                    },
                ],
            },
        );

        mock
    }
}

impl Backend for MockBackend {
    fn fetch_releases(&self, tool: Tool) -> Result<Vec<Release>> {
        let releases = self.releases.lock().unwrap();
        Ok(releases.get(&tool).cloned().unwrap_or_default())
    }

    fn fetch_release(&self, tool: Tool, tag: &str) -> Result<Release> {
        let releases = self.releases.lock().unwrap();
        releases
            .get(&tool)
            .and_then(|r| r.iter().find(|release| release.tag == tag))
            .cloned()
            .ok_or_else(|| Error::VersionNotFound {
                tool: tool.to_string(),
                version: tag.to_string(),
            })
    }

    fn download_asset(&self, tool: Tool, release: &Release, platform: &Platform) -> Result<Vec<u8>> {
        let binary_name = tool.binary_name();
        let expected_name = format!("{}-{}.zst", binary_name, platform.triple);

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == expected_name)
            .ok_or_else(|| Error::DownloadFailed {
                tool: tool.to_string(),
                message: format!("no asset found for platform {}", platform.triple),
            })?;

        let assets = self.assets.lock().unwrap();
        assets.get(&asset.name).cloned().ok_or_else(|| Error::DownloadFailed {
            tool: tool.to_string(),
            message: format!("mock asset not configured: {}", asset.name),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_backend_new() {
        let mock = MockBackend::new();
        let releases = mock.fetch_releases(Tool::Buck2).unwrap();
        assert!(releases.is_empty());
    }

    #[test]
    fn test_mock_backend_add_release() {
        let mut mock = MockBackend::new();
        mock.add_release(
            Tool::Buck2,
            Release {
                tag: "2024-01-15".to_string(),
                name: "Release".to_string(),
                prerelease: false,
                published_at: String::new(),
                assets: vec![],
            },
        );

        let releases = mock.fetch_releases(Tool::Buck2).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "2024-01-15");
    }

    #[test]
    fn test_mock_backend_fetch_release() {
        let mut mock = MockBackend::new();
        mock.add_release(
            Tool::Buck2,
            Release {
                tag: "2024-01-15".to_string(),
                name: "Release".to_string(),
                prerelease: false,
                published_at: String::new(),
                assets: vec![],
            },
        );

        let release = mock.fetch_release(Tool::Buck2, "2024-01-15").unwrap();
        assert_eq!(release.tag, "2024-01-15");

        let not_found = mock.fetch_release(Tool::Buck2, "nonexistent");
        assert!(not_found.is_err());
    }

    #[test]
    fn test_mock_backend_with_buck2_releases() {
        let mock = MockBackend::with_buck2_releases();
        let releases = mock.fetch_releases(Tool::Buck2).unwrap();
        assert_eq!(releases.len(), 1);
        assert!(!releases[0].assets.is_empty());
    }

    #[test]
    fn test_mock_backend_download_asset() {
        let mut mock = MockBackend::with_buck2_releases();
        mock.add_asset("buck2-aarch64-apple-darwin.zst", vec![0x28, 0xb5, 0x2f, 0xfd]);

        let release = mock.fetch_release(Tool::Buck2, "2024-01-15").unwrap();
        let platform = Platform::new("macos", "aarch64", "aarch64-apple-darwin");

        let data = mock.download_asset(Tool::Buck2, &release, &platform).unwrap();
        assert_eq!(data, vec![0x28, 0xb5, 0x2f, 0xfd]);
    }

    #[test]
    fn test_mock_backend_download_asset_not_configured() {
        let mock = MockBackend::with_buck2_releases();

        let release = mock.fetch_release(Tool::Buck2, "2024-01-15").unwrap();
        let platform = Platform::new("macos", "aarch64", "aarch64-apple-darwin");

        let result = mock.download_asset(Tool::Buck2, &release, &platform);
        assert!(result.is_err());
    }
}
