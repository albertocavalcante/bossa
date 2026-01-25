//! Buck2-specific installation logic.
//!
//! This module provides the [`Buck2Installer`] for installing Meta's Buck2
//! build system from GitHub releases.
//!
//! Buck2 releases are distributed as zstd-compressed binaries for each
//! supported platform.

use crate::error::{Error, Result};
use crate::platform;
use crate::tools::ToolInstaller;
use crate::types::{InstallOptions, InstallResult, Platform, Tool};
use std::fs;
use std::io::Cursor;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Buck2 installer.
///
/// Handles downloading, decompressing, and installing Buck2 binaries
/// from GitHub releases.
///
/// # Example
///
/// ```no_run
/// use toolchain::tools::buck2::Buck2Installer;
/// use toolchain::tools::ToolInstaller;
///
/// let installer = Buck2Installer::new();
/// println!("Installing to: {:?}", installer.default_install_dir().unwrap());
/// ```
pub struct Buck2Installer;

impl Buck2Installer {
    /// Create a new Buck2 installer.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Decompress a zstd-compressed binary.
    fn decompress(&self, compressed: &[u8]) -> Result<Vec<u8>> {
        let cursor = Cursor::new(compressed);
        let mut decoder = zstd::Decoder::new(cursor)
            .map_err(|e| Error::DecompressionFailed(e.to_string()))?;

        let mut decompressed = Vec::new();
        std::io::copy(&mut decoder, &mut decompressed)
            .map_err(|e| Error::DecompressionFailed(e.to_string()))?;

        Ok(decompressed)
    }

    /// Find buck2 in PATH.
    fn find_in_path(&self) -> Option<PathBuf> {
        which::which("buck2").ok()
    }
}

impl Default for Buck2Installer {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolInstaller for Buck2Installer {
    fn tool(&self) -> Tool {
        Tool::Buck2
    }

    fn install(
        &self,
        compressed_bytes: &[u8],
        _platform: &Platform,
        options: &InstallOptions,
    ) -> Result<InstallResult> {
        // Determine install directory
        let install_dir = options
            .install_dir
            .clone()
            .or_else(|| self.default_install_dir().ok())
            .ok_or_else(|| Error::Other("cannot determine install directory".to_string()))?;

        // Ensure directory exists
        fs::create_dir_all(&install_dir).map_err(|e| Error::io(&install_dir, e))?;

        // Determine binary path
        let binary_name = format!("buck2{}", platform::executable_extension());
        let binary_path = install_dir.join(&binary_name);

        // Check for existing installation
        let was_upgrade = binary_path.exists();
        let previous_version = if was_upgrade {
            self.installed_version().ok().flatten()
        } else {
            None
        };

        // Check if we should overwrite
        if was_upgrade && !options.force {
            return Err(Error::Other(format!(
                "buck2 already installed at {}. Use --force to overwrite.",
                binary_path.display()
            )));
        }

        // Decompress
        let decompressed = self.decompress(compressed_bytes)?;

        // Write binary
        fs::write(&binary_path, &decompressed).map_err(|e| Error::io(&binary_path, e))?;

        // Make executable (Unix only)
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&binary_path)
                .map_err(|e| Error::io(&binary_path, e))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&binary_path, perms).map_err(|e| Error::io(&binary_path, e))?;
        }

        // Verify installation
        self.verify(&binary_path)?;

        // Get installed version
        let version = self
            .get_version_from_binary(&binary_path)?
            .unwrap_or_else(|| options.version.clone().unwrap_or("latest".to_string()));

        Ok(InstallResult {
            tool: Tool::Buck2,
            version,
            path: binary_path,
            was_upgrade,
            previous_version,
        })
    }

    fn is_installed(&self) -> Result<bool> {
        Ok(self.find_in_path().is_some())
    }

    fn installed_version(&self) -> Result<Option<String>> {
        if let Some(path) = self.find_in_path() {
            self.get_version_from_binary(&path)
        } else {
            Ok(None)
        }
    }

    fn default_install_dir(&self) -> Result<PathBuf> {
        // Prefer ~/.local/bin (XDG-compliant)
        if let Some(home) = dirs::home_dir() {
            let local_bin = home.join(".local").join("bin");
            return Ok(local_bin);
        }

        // Fallback to /usr/local/bin
        Ok(PathBuf::from("/usr/local/bin"))
    }

    fn verify(&self, path: &Path) -> Result<()> {
        let output = Command::new(path)
            .arg("--version")
            .output()
            .map_err(|e| Error::Other(format!("failed to execute buck2: {}", e)))?;

        if !output.status.success() {
            return Err(Error::Other(format!(
                "buck2 verification failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }
}

impl Buck2Installer {
    /// Get version from a buck2 binary.
    fn get_version_from_binary(&self, path: &Path) -> Result<Option<String>> {
        let output = Command::new(path)
            .arg("--version")
            .output()
            .map_err(|e| Error::Other(format!("failed to execute buck2: {}", e)))?;

        if !output.status.success() {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Output format: "buck2 <version> <hash> <date>"
        // Example: "buck2 2024-01-15 abc1234 ..."
        let version = stdout
            .split_whitespace()
            .nth(1)
            .map(|s| s.to_string());

        Ok(version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_install_dir() {
        let installer = Buck2Installer::new();
        let dir = installer.default_install_dir().unwrap();
        assert!(dir.to_string_lossy().contains("bin"));
    }

    #[test]
    fn test_tool_type() {
        let installer = Buck2Installer::new();
        assert_eq!(installer.tool(), Tool::Buck2);
    }

    #[test]
    fn test_installer_default() {
        let installer = Buck2Installer::default();
        assert_eq!(installer.tool(), Tool::Buck2);
    }

    #[test]
    fn test_default_install_dir_contains_local() {
        let installer = Buck2Installer::new();
        let dir = installer.default_install_dir().unwrap();

        // Should be either ~/.local/bin or /usr/local/bin
        let path_str = dir.to_string_lossy();
        assert!(
            path_str.contains(".local/bin") || path_str.contains("local/bin"),
            "Expected path to contain 'local/bin', got: {}",
            path_str
        );
    }

    #[test]
    fn test_decompress_invalid_data() {
        let installer = Buck2Installer::new();
        let invalid_data = vec![0, 1, 2, 3, 4, 5];

        let result = installer.decompress(&invalid_data);
        assert!(result.is_err());

        if let Err(Error::DecompressionFailed(msg)) = result {
            assert!(!msg.is_empty());
        } else {
            panic!("Expected DecompressionFailed error");
        }
    }

    #[test]
    fn test_decompress_empty_data() {
        let installer = Buck2Installer::new();
        let empty_data: Vec<u8> = vec![];

        let result = installer.decompress(&empty_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_find_in_path_when_not_installed() {
        // Create an installer and check find_in_path
        // This test is environment-dependent, so we just verify it doesn't panic
        let installer = Buck2Installer::new();
        let _result = installer.find_in_path();
        // Can be Some or None depending on whether buck2 is installed
    }
}
