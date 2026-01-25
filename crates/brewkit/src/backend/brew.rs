//! Real Homebrew CLI backend using `brew` commands.

use crate::backend::Backend;
use crate::error::{Error, Result};
use crate::types::{BundleResult, InstalledPackage, Package, PackageType};
use std::path::Path;
use std::process::Command;

/// Backend that executes real `brew` commands.
pub struct BrewBackend {
    /// Path to the brew executable
    brew_path: String,
}

impl BrewBackend {
    /// Create a new BrewBackend.
    ///
    /// Returns an error if Homebrew is not installed.
    pub fn new() -> Result<Self> {
        let brew_path = find_brew()?;
        Ok(Self { brew_path })
    }

    /// Run a brew command and return output.
    fn run_brew(&self, args: &[&str]) -> Result<std::process::Output> {
        let output = Command::new(&self.brew_path)
            .args(args)
            .output()
            .map_err(|e| Error::CommandFailed {
                message: format!("failed to execute brew: {}", e),
                stderr: String::new(),
            })?;
        Ok(output)
    }

    /// Run a brew command and check for success.
    fn run_brew_checked(&self, args: &[&str], package_name: Option<&str>) -> Result<String> {
        let output = self.run_brew(args)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::from_brew_output(&stderr, package_name));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

impl Backend for BrewBackend {
    fn is_available(&self) -> bool {
        self.run_brew(&["--version"]).is_ok()
    }

    fn install(&self, package: &Package) -> Result<()> {
        let args = match package.package_type {
            PackageType::Tap => vec!["tap", package.name.as_str()],
            PackageType::Brew => vec!["install", "--formula", package.name.as_str()],
            PackageType::Cask => vec!["install", "--cask", package.name.as_str()],
            PackageType::Mas => {
                // mas install <app_id>
                let id = package.mas_id().ok_or_else(|| Error::Other(
                    "mas package missing id".to_string()
                ))?;
                return run_mas_install(id);
            }
            PackageType::Vscode => {
                // code --install-extension <ext_id>
                return run_vscode_install(&package.name);
            }
        };

        self.run_brew_checked(&args, Some(&package.name))?;
        Ok(())
    }

    fn uninstall(&self, package: &Package) -> Result<()> {
        let args = match package.package_type {
            PackageType::Tap => vec!["untap", package.name.as_str()],
            PackageType::Brew => vec!["uninstall", "--formula", package.name.as_str()],
            PackageType::Cask => vec!["uninstall", "--cask", package.name.as_str()],
            PackageType::Mas => {
                return Err(Error::Other("mas uninstall not supported".to_string()));
            }
            PackageType::Vscode => {
                return run_vscode_uninstall(&package.name);
            }
        };

        self.run_brew_checked(&args, Some(&package.name))?;
        Ok(())
    }

    fn is_installed(&self, package: &Package) -> Result<bool> {
        match package.package_type {
            PackageType::Tap => {
                let output = self.run_brew(&["tap"])?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(stdout.lines().any(|t| t.trim() == package.name))
            }
            PackageType::Brew | PackageType::Cask => {
                let type_flag = if package.package_type == PackageType::Cask {
                    "--cask"
                } else {
                    "--formula"
                };

                let output = self.run_brew(&["info", "--json=v2", type_flag, &package.name])?;

                if !output.status.success() {
                    return Ok(false);
                }

                let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;

                let installed = if package.package_type == PackageType::Cask {
                    json["casks"]
                        .as_array()
                        .and_then(|arr| arr.first())
                        .and_then(|c| c["installed"].as_str())
                        .is_some()
                } else {
                    json["formulae"]
                        .as_array()
                        .and_then(|arr| arr.first())
                        .and_then(|f| f["installed"].as_array())
                        .map(|arr| !arr.is_empty())
                        .unwrap_or(false)
                };

                Ok(installed)
            }
            PackageType::Mas => {
                let id = package.mas_id().ok_or_else(|| Error::Other(
                    "mas package missing id".to_string()
                ))?;
                is_mas_installed(id)
            }
            PackageType::Vscode => is_vscode_installed(&package.name),
        }
    }

    fn list_installed(&self, package_type: PackageType) -> Result<Vec<InstalledPackage>> {
        match package_type {
            PackageType::Tap => {
                let output = self.run_brew_checked(&["tap"], None)?;
                Ok(output
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| InstalledPackage {
                        name: l.trim().to_string(),
                        package_type: PackageType::Tap,
                        version: String::new(),
                        installed_on_request: true,
                    })
                    .collect())
            }
            PackageType::Brew => {
                let output = self.run_brew(&["info", "--json=v2", "--installed"])?;
                if !output.status.success() {
                    return Ok(Vec::new());
                }
                let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
                parse_installed_formulas(&json)
            }
            PackageType::Cask => {
                let output = self.run_brew(&["info", "--json=v2", "--cask", "--installed"])?;
                if !output.status.success() {
                    return Ok(Vec::new());
                }
                let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
                parse_installed_casks(&json)
            }
            PackageType::Mas => list_mas_installed(),
            PackageType::Vscode => list_vscode_installed(),
        }
    }

    fn get_version(&self, package: &Package) -> Result<Option<String>> {
        match package.package_type {
            PackageType::Brew | PackageType::Cask => {
                let type_flag = if package.package_type == PackageType::Cask {
                    "--cask"
                } else {
                    "--formula"
                };

                let output = self.run_brew(&["info", "--json=v2", type_flag, &package.name])?;

                if !output.status.success() {
                    return Ok(None);
                }

                let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;

                let version = if package.package_type == PackageType::Cask {
                    json["casks"]
                        .as_array()
                        .and_then(|arr| arr.first())
                        .and_then(|c| c["installed"].as_str())
                        .map(|s| s.to_string())
                } else {
                    json["formulae"]
                        .as_array()
                        .and_then(|arr| arr.first())
                        .and_then(|f| f["installed"].as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|i| i["version"].as_str())
                        .map(|s| s.to_string())
                };

                Ok(version)
            }
            _ => Ok(None),
        }
    }

    fn bundle(&self, brewfile_path: &Path, verbose: bool) -> Result<BundleResult> {
        let mut args = vec!["bundle", "--file", brewfile_path.to_str().unwrap_or("")];
        if verbose {
            args.push("--verbose");
        }

        let output = self.run_brew(&args)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        parse_bundle_output(&stdout, &stderr, output.status.success())
    }

    fn update(&self) -> Result<()> {
        self.run_brew_checked(&["update"], None)?;
        Ok(())
    }

    fn upgrade(&self, package: Option<&Package>) -> Result<()> {
        let args = match package {
            Some(p) => match p.package_type {
                PackageType::Brew => vec!["upgrade", "--formula", p.name.as_str()],
                PackageType::Cask => vec!["upgrade", "--cask", p.name.as_str()],
                _ => return Ok(()),
            },
            None => vec!["upgrade"],
        };

        self.run_brew_checked(&args, package.map(|p| p.name.as_str()))?;
        Ok(())
    }
}

/// Find the brew executable path.
fn find_brew() -> Result<String> {
    // Check common locations
    let paths = [
        "/opt/homebrew/bin/brew", // Apple Silicon
        "/usr/local/bin/brew",    // Intel
        "/home/linuxbrew/.linuxbrew/bin/brew", // Linux
    ];

    for path in &paths {
        if std::path::Path::new(path).exists() {
            return Ok(path.to_string());
        }
    }

    // Try which
    let output = Command::new("which")
        .arg("brew")
        .output()
        .map_err(|_| Error::BrewNotFound)?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(path);
        }
    }

    Err(Error::BrewNotFound)
}

/// Parse installed formulas from brew info JSON.
fn parse_installed_formulas(json: &serde_json::Value) -> Result<Vec<InstalledPackage>> {
    let empty = Vec::new();
    let formulas = json["formulae"].as_array().unwrap_or(&empty);

    let mut installed = Vec::new();
    for formula in formulas {
        let name = formula["name"].as_str().unwrap_or_default();
        let installed_versions = formula["installed"].as_array();

        if let Some(versions) = installed_versions {
            if let Some(first) = versions.first() {
                let version = first["version"].as_str().unwrap_or_default();
                let on_request = first["installed_on_request"].as_bool().unwrap_or(false);

                installed.push(InstalledPackage {
                    name: name.to_string(),
                    package_type: PackageType::Brew,
                    version: version.to_string(),
                    installed_on_request: on_request,
                });
            }
        }
    }

    Ok(installed)
}

/// Parse installed casks from brew info JSON.
fn parse_installed_casks(json: &serde_json::Value) -> Result<Vec<InstalledPackage>> {
    let empty = Vec::new();
    let casks = json["casks"].as_array().unwrap_or(&empty);

    let mut installed = Vec::new();
    for cask in casks {
        let name = cask["token"].as_str().unwrap_or_default();
        let version = cask["installed"].as_str();

        if let Some(ver) = version {
            installed.push(InstalledPackage {
                name: name.to_string(),
                package_type: PackageType::Cask,
                version: ver.to_string(),
                installed_on_request: true, // Casks are always explicit
            });
        }
    }

    Ok(installed)
}

/// Parse bundle output to extract results.
fn parse_bundle_output(stdout: &str, stderr: &str, success: bool) -> Result<BundleResult> {
    let mut result = BundleResult::default();
    let combined = format!("{}\n{}", stdout, stderr);

    for line in combined.lines() {
        let line = line.trim();

        // Patterns for success
        if line.starts_with("Installing ") || line.starts_with("Tapping ") {
            if let Some(name) = extract_package_name(line) {
                result.installed.push(name);
            }
        } else if line.contains("already installed") || line.starts_with("Using ") {
            if let Some(name) = extract_package_name(line) {
                result.skipped.push(name);
            }
        } else if line.starts_with("Upgrading ") {
            if let Some(name) = extract_package_name(line) {
                result.upgraded.push(name);
            }
        } else if line.contains("Error:") || line.contains("failed") {
            if let Some(name) = extract_package_name(line) {
                let error_msg = line.to_string();
                result.failed.push((name, error_msg));
            }
        }
    }

    // If we couldn't parse anything but the command failed, report generic error
    if !success && result.failed.is_empty() && result.installed.is_empty() {
        result.failed.push(("bundle".to_string(), stderr.to_string()));
    }

    Ok(result)
}

/// Extract package name from a brew output line.
fn extract_package_name(line: &str) -> Option<String> {
    // Patterns:
    // "Installing git"
    // "Installing git 2.40.0"
    // "Tapping homebrew/cask"
    // "Using git"
    // "Error: git: ..."

    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() >= 2 {
        let name = words[1].trim_end_matches(':');
        // Skip if it's not a package name (contains special chars)
        if !name.is_empty()
            && !name.starts_with('-')
            && !name.contains('=')
        {
            return Some(name.to_string());
        }
    }
    None
}

// =============================================================================
// mas (Mac App Store) helpers
// =============================================================================

fn run_mas_install(app_id: &str) -> Result<()> {
    let output = Command::new("mas")
        .args(["install", app_id])
        .output()
        .map_err(|e| Error::CommandFailed {
            message: format!("failed to execute mas: {}", e),
            stderr: String::new(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::CommandFailed {
            message: format!("mas install failed for {}", app_id),
            stderr: stderr.to_string(),
        });
    }

    Ok(())
}

fn is_mas_installed(app_id: &str) -> Result<bool> {
    let output = Command::new("mas")
        .args(["list"])
        .output()
        .map_err(|_| Error::Other("mas not available".to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().any(|l| l.starts_with(app_id)))
}

fn list_mas_installed() -> Result<Vec<InstalledPackage>> {
    let output = Command::new("mas")
        .args(["list"])
        .output()
        .map_err(|_| Error::Other("mas not available".to_string()))?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut installed = Vec::new();

    for line in stdout.lines() {
        // Format: "497799835 Xcode (14.3)"
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() >= 2 {
            let id = parts[0];
            let rest = parts[1];

            // Extract name and version
            let (name, version) = if let Some(paren_pos) = rest.rfind('(') {
                let name = rest[..paren_pos].trim();
                let version = rest[paren_pos + 1..].trim_end_matches(')');
                (name, version)
            } else {
                (rest.trim(), "")
            };

            installed.push(InstalledPackage {
                name: format!("{} ({})", name, id),
                package_type: PackageType::Mas,
                version: version.to_string(),
                installed_on_request: true,
            });
        }
    }

    Ok(installed)
}

// =============================================================================
// VS Code extension helpers
// =============================================================================

fn run_vscode_install(extension: &str) -> Result<()> {
    let output = Command::new("code")
        .args(["--install-extension", extension])
        .output()
        .map_err(|e| Error::CommandFailed {
            message: format!("failed to execute code: {}", e),
            stderr: String::new(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::CommandFailed {
            message: format!("vscode install failed for {}", extension),
            stderr: stderr.to_string(),
        });
    }

    Ok(())
}

fn run_vscode_uninstall(extension: &str) -> Result<()> {
    let output = Command::new("code")
        .args(["--uninstall-extension", extension])
        .output()
        .map_err(|e| Error::CommandFailed {
            message: format!("failed to execute code: {}", e),
            stderr: String::new(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::CommandFailed {
            message: format!("vscode uninstall failed for {}", extension),
            stderr: stderr.to_string(),
        });
    }

    Ok(())
}

fn is_vscode_installed(extension: &str) -> Result<bool> {
    let output = Command::new("code")
        .args(["--list-extensions"])
        .output()
        .map_err(|_| Error::Other("code not available".to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let extension_lower = extension.to_lowercase();
    Ok(stdout.lines().any(|l| l.to_lowercase() == extension_lower))
}

fn list_vscode_installed() -> Result<Vec<InstalledPackage>> {
    let output = Command::new("code")
        .args(["--list-extensions", "--show-versions"])
        .output()
        .map_err(|_| Error::Other("code not available".to_string()))?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut installed = Vec::new();

    for line in stdout.lines() {
        // Format: "ms-python.python@2024.0.1"
        let (name, version) = if let Some(at_pos) = line.rfind('@') {
            (&line[..at_pos], &line[at_pos + 1..])
        } else {
            (line, "")
        };

        installed.push(InstalledPackage {
            name: name.to_string(),
            package_type: PackageType::Vscode,
            version: version.to_string(),
            installed_on_request: true,
        });
    }

    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_package_name() {
        assert_eq!(extract_package_name("Installing git"), Some("git".to_string()));
        assert_eq!(extract_package_name("Installing git 2.40.0"), Some("git".to_string()));
        assert_eq!(extract_package_name("Tapping homebrew/cask"), Some("homebrew/cask".to_string()));
        assert_eq!(extract_package_name("Using curl"), Some("curl".to_string()));
    }

    #[test]
    fn test_parse_bundle_output_success() {
        let stdout = "Installing git\nInstalling curl\nUsing wget";
        let result = parse_bundle_output(stdout, "", true).unwrap();

        assert_eq!(result.installed, vec!["git", "curl"]);
        assert_eq!(result.skipped, vec!["wget"]);
        assert!(result.failed.is_empty());
    }

    #[test]
    fn test_parse_bundle_output_with_errors() {
        let stderr = "Error: foo: not found";
        let result = parse_bundle_output("", stderr, false).unwrap();

        assert!(!result.failed.is_empty());
    }
}
