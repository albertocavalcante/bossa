//! GitHub releases backend.
//!
//! This module provides the [`GitHubBackend`] implementation for fetching
//! tool releases from GitHub's Releases API.
//!
//! # Rate Limiting
//!
//! The GitHub API has rate limits. For unauthenticated requests, the limit
//! is 60 requests per hour. If you need more, consider using a GitHub token.

use crate::backend::Backend;
use crate::error::{Error, Result};
use crate::types::{Platform, Release, ReleaseAsset, Tool};
use serde::Deserialize;

/// Maximum download size (100 MB should cover most build tools).
const MAX_BODY_SIZE: u64 = 100 * 1024 * 1024;

/// GitHub releases backend.
///
/// Fetches releases from GitHub's API and downloads assets.
///
/// # Example
///
/// ```no_run
/// use toolchain::backend::github::GitHubBackend;
/// use toolchain::backend::Backend;
/// use toolchain::Tool;
///
/// let backend = GitHubBackend::new();
/// let releases = backend.fetch_releases(Tool::Buck2).unwrap();
/// println!("Found {} releases", releases.len());
/// ```
pub struct GitHubBackend {
    /// HTTP agent for requests.
    agent: ureq::Agent,
    /// GitHub API base URL.
    api_base: String,
}

impl GitHubBackend {
    /// Create a new GitHub backend.
    #[must_use]
    pub fn new() -> Self {
        let agent = ureq::Agent::new_with_defaults();
        Self {
            agent,
            api_base: "https://api.github.com".to_string(),
        }
    }

    /// Create a backend with a custom API base (for testing).
    #[must_use]
    pub fn with_api_base(api_base: impl Into<String>) -> Self {
        let agent = ureq::Agent::new_with_defaults();
        Self {
            agent,
            api_base: api_base.into(),
        }
    }

    /// Get the current API base URL.
    #[must_use]
    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    /// Build the API URL for releases.
    fn releases_url(&self, tool: Tool) -> String {
        format!("{}/repos/{}/releases", self.api_base, tool.github_repo())
    }

    /// Build the API URL for a specific release.
    fn release_url(&self, tool: Tool, tag: &str) -> String {
        format!(
            "{}/repos/{}/releases/tags/{}",
            self.api_base,
            tool.github_repo(),
            tag
        )
    }

    /// Find the asset matching the platform.
    fn find_asset<'a>(
        &self,
        tool: Tool,
        release: &'a Release,
        platform: &Platform,
    ) -> Result<&'a ReleaseAsset> {
        let binary_name = tool.binary_name();
        let expected_name = format!("{}-{}.zst", binary_name, platform.triple);

        release
            .assets
            .iter()
            .find(|a| a.name == expected_name)
            .ok_or_else(|| Error::DownloadFailed {
                tool: tool.to_string(),
                message: format!(
                    "no asset found for platform {} (expected {})",
                    platform.triple, expected_name
                ),
            })
    }
}

impl Default for GitHubBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for GitHubBackend {
    fn fetch_releases(&self, tool: Tool) -> Result<Vec<Release>> {
        let url = self.releases_url(tool);

        let response: Vec<GitHubRelease> = self
            .agent
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "toolchain-rs")
            .call()?
            .body_mut()
            .read_json()?;

        Ok(response.into_iter().map(Into::into).collect())
    }

    fn fetch_release(&self, tool: Tool, tag: &str) -> Result<Release> {
        let url = self.release_url(tool, tag);

        let response: GitHubRelease = self
            .agent
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "toolchain-rs")
            .call()?
            .body_mut()
            .read_json()?;

        Ok(response.into())
    }

    fn download_asset(&self, tool: Tool, release: &Release, platform: &Platform) -> Result<Vec<u8>> {
        let asset = self.find_asset(tool, release, platform)?;

        // Download the asset with increased size limit
        let mut response = self
            .agent
            .get(&asset.download_url)
            .header("Accept", "application/octet-stream")
            .header("User-Agent", "toolchain-rs")
            .call()?;

        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_SIZE)
            .read_to_vec()
            .map_err(|e| Error::DownloadFailed {
                tool: tool.to_string(),
                message: e.to_string(),
            })?;

        Ok(bytes)
    }
}

// =============================================================================
// GitHub API response types
// =============================================================================

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

impl From<GitHubRelease> for Release {
    fn from(r: GitHubRelease) -> Self {
        Self {
            tag: r.tag_name.clone(),
            name: r.name.unwrap_or(r.tag_name),
            prerelease: r.prerelease,
            published_at: r.published_at.unwrap_or_default(),
            assets: r.assets.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<GitHubAsset> for ReleaseAsset {
    fn from(a: GitHubAsset) -> Self {
        Self {
            name: a.name,
            download_url: a.browser_download_url,
            size: a.size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Platform;

    #[test]
    fn test_releases_url() {
        let backend = GitHubBackend::new();
        let url = backend.releases_url(Tool::Buck2);
        assert_eq!(url, "https://api.github.com/repos/facebook/buck2/releases");
    }

    #[test]
    fn test_release_url() {
        let backend = GitHubBackend::new();
        let url = backend.release_url(Tool::Buck2, "latest");
        assert_eq!(
            url,
            "https://api.github.com/repos/facebook/buck2/releases/tags/latest"
        );
    }

    #[test]
    fn test_custom_api_base() {
        let backend = GitHubBackend::with_api_base("https://custom.api.com");
        assert_eq!(backend.api_base(), "https://custom.api.com");

        let url = backend.releases_url(Tool::Buck2);
        assert_eq!(url, "https://custom.api.com/repos/facebook/buck2/releases");
    }

    #[test]
    fn test_default_impl() {
        let backend = GitHubBackend::default();
        assert_eq!(backend.api_base(), "https://api.github.com");
    }

    #[test]
    fn test_find_asset() {
        let backend = GitHubBackend::new();
        let release = Release {
            tag: "2024-01-15".to_string(),
            name: "Release".to_string(),
            prerelease: false,
            published_at: String::new(),
            assets: vec![
                ReleaseAsset {
                    name: "buck2-aarch64-apple-darwin.zst".to_string(),
                    download_url: "https://example.com/darwin.zst".to_string(),
                    size: 1024,
                },
                ReleaseAsset {
                    name: "buck2-x86_64-unknown-linux-gnu.zst".to_string(),
                    download_url: "https://example.com/linux.zst".to_string(),
                    size: 2048,
                },
            ],
        };

        let platform = Platform::new("macos", "aarch64", "aarch64-apple-darwin");
        let asset = backend.find_asset(Tool::Buck2, &release, &platform);
        assert!(asset.is_ok());
        assert_eq!(asset.unwrap().name, "buck2-aarch64-apple-darwin.zst");

        let linux_platform = Platform::new("linux", "x86_64", "x86_64-unknown-linux-gnu");
        let linux_asset = backend.find_asset(Tool::Buck2, &release, &linux_platform);
        assert!(linux_asset.is_ok());
    }

    #[test]
    fn test_find_asset_not_found() {
        let backend = GitHubBackend::new();
        let release = Release {
            tag: "2024-01-15".to_string(),
            name: "Release".to_string(),
            prerelease: false,
            published_at: String::new(),
            assets: vec![ReleaseAsset {
                name: "buck2-aarch64-apple-darwin.zst".to_string(),
                download_url: "https://example.com/darwin.zst".to_string(),
                size: 1024,
            }],
        };

        let platform = Platform::new("windows", "x86_64", "x86_64-pc-windows-msvc");
        let asset = backend.find_asset(Tool::Buck2, &release, &platform);
        assert!(asset.is_err());
    }

    #[test]
    fn test_github_release_conversion() {
        let gh_release = GitHubRelease {
            tag_name: "2024-01-15".to_string(),
            name: Some("Release 2024-01-15".to_string()),
            prerelease: false,
            published_at: Some("2024-01-15T00:00:00Z".to_string()),
            assets: vec![GitHubAsset {
                name: "buck2.zst".to_string(),
                browser_download_url: "https://example.com/buck2.zst".to_string(),
                size: 1024,
            }],
        };

        let release: Release = gh_release.into();
        assert_eq!(release.tag, "2024-01-15");
        assert_eq!(release.name, "Release 2024-01-15");
        assert!(!release.prerelease);
        assert_eq!(release.assets.len(), 1);
    }

    #[test]
    fn test_github_release_conversion_with_defaults() {
        let gh_release = GitHubRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            prerelease: true,
            published_at: None,
            assets: vec![],
        };

        let release: Release = gh_release.into();
        assert_eq!(release.tag, "v1.0.0");
        assert_eq!(release.name, "v1.0.0"); // Falls back to tag_name
        assert!(release.prerelease);
        assert_eq!(release.published_at, "");
        assert!(release.assets.is_empty());
    }

    #[test]
    fn test_github_asset_conversion() {
        let gh_asset = GitHubAsset {
            name: "buck2-darwin.zst".to_string(),
            browser_download_url: "https://example.com/buck2-darwin.zst".to_string(),
            size: 50 * 1024 * 1024,
        };

        let asset: ReleaseAsset = gh_asset.into();
        assert_eq!(asset.name, "buck2-darwin.zst");
        assert_eq!(asset.download_url, "https://example.com/buck2-darwin.zst");
        assert_eq!(asset.size, 50 * 1024 * 1024);
    }
}
