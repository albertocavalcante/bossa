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
                message: format!("failed to execute brew: {e}"),
                stderr: String::new(),
            })?;
        Ok(output)
    }

    /// Check command status without interpreting output from mutations.
    fn run_brew_success(
        &self,
        args: &[&str],
        package_name: Option<&str>,
    ) -> Result<std::process::Output> {
        let output = self.run_brew(args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::from_brew_output(&stderr, package_name));
        }
        Ok(output)
    }

    /// Decode successful observation output strictly.
    fn run_brew_checked(&self, args: &[&str], package_name: Option<&str>) -> Result<String> {
        let output = self.run_brew_success(args, package_name)?;
        String::from_utf8(output.stdout)
            .map_err(|e| Error::Other(format!("invalid UTF-8 in brew output: {e}")))
    }
}

impl Backend for BrewBackend {
    fn is_available(&self) -> bool {
        self.run_brew(&["--version"])
            .is_ok_and(|output| output.status.success())
    }

    fn install(&self, package: &Package) -> Result<()> {
        let args = match package.package_type {
            PackageType::Tap => vec!["tap", package.name.as_str()],
            PackageType::Brew => vec!["install", "--formula", package.name.as_str()],
            PackageType::Cask => vec!["install", "--cask", package.name.as_str()],
            PackageType::Mas => {
                // mas install <app_id>
                let id = package
                    .mas_id()
                    .ok_or_else(|| Error::Other("mas package missing id".to_string()))?;
                return run_mas_install(id);
            }
            PackageType::Vscode => {
                // code --install-extension <ext_id>
                return run_vscode_install(&package.name);
            }
        };

        self.run_brew_success(&args, Some(&package.name))?;
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

        self.run_brew_success(&args, Some(&package.name))?;
        Ok(())
    }

    fn is_installed(&self, package: &Package) -> Result<bool> {
        match package.package_type {
            PackageType::Tap => {
                let stdout = self.run_brew_checked(&["tap"], None)?;
                Ok(stdout.lines().any(|t| t.trim() == package.name))
            }
            PackageType::Brew | PackageType::Cask => Ok(self.get_version(package)?.is_some()),
            PackageType::Mas => {
                let id = package
                    .mas_id()
                    .ok_or_else(|| Error::Other("mas package missing id".to_string()))?;
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
                let output = self.run_brew_checked(&["info", "--json=v2", "--installed"], None)?;
                let json: serde_json::Value = serde_json::from_str(&output)?;
                parse_installed_formulas(&json)
            }
            PackageType::Cask => {
                let output =
                    self.run_brew_checked(&["info", "--json=v2", "--cask", "--installed"], None)?;
                let json: serde_json::Value = serde_json::from_str(&output)?;
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

                let output = self.run_brew_checked(
                    &["info", "--json=v2", type_flag, &package.name],
                    Some(&package.name),
                )?;
                let json: serde_json::Value = serde_json::from_str(&output)?;
                parse_package_version(&json, package.package_type)
            }
            _ => Err(Error::Other(
                "version observation is not supported for this package type".to_string(),
            )),
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
        self.run_brew_success(&["update"], None)?;
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

        self.run_brew_success(&args, package.map(|p| p.name.as_str()))?;
        Ok(())
    }
}

/// Find the brew executable path.
fn find_brew() -> Result<String> {
    // Check common locations
    let paths = [
        "/opt/homebrew/bin/brew",              // Apple Silicon
        "/usr/local/bin/brew",                 // Intel
        "/home/linuxbrew/.linuxbrew/bin/brew", // Linux
    ];

    for path in &paths {
        if std::path::Path::new(path).exists() {
            return Ok((*path).to_string());
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

fn field_array<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a Vec<serde_json::Value>> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Error::Other(format!("missing or invalid {field} in brew output")))
}

fn required_string<'a>(value: &'a serde_json::Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Other(format!("missing or invalid {field} in brew output")))
}

fn parse_package_version(json: &serde_json::Value, kind: PackageType) -> Result<Option<String>> {
    let field = if kind == PackageType::Cask {
        "casks"
    } else {
        "formulae"
    };
    let entries = field_array(json, field)?;
    if entries.len() != 1 {
        return Err(Error::Other(format!(
            "expected one {field} entry in brew output"
        )));
    }
    let entry = &entries[0];
    if kind == PackageType::Cask {
        match entry.get("installed") {
            Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(version)) if !version.is_empty() => {
                Ok(Some(version.clone()))
            }
            _ => Err(Error::Other(
                "missing or invalid cask installed field".to_string(),
            )),
        }
    } else {
        let versions = field_array(entry, "installed")?;
        versions
            .first()
            .map(|v| required_string(v, "version").map(str::to_owned))
            .transpose()
    }
}

/// Parse installed formulas from brew info JSON.
fn parse_installed_formulas(json: &serde_json::Value) -> Result<Vec<InstalledPackage>> {
    let mut installed = Vec::new();
    for formula in field_array(json, "formulae")? {
        let name = required_string(formula, "name")?;
        let versions = field_array(formula, "installed")?;
        let version = versions.first().ok_or_else(|| {
            Error::Other("installed formula has no installed version".to_string())
        })?;
        let version_string = required_string(version, "version")?;
        let on_request = version
            .get("installed_on_request")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                Error::Other("missing or invalid installed_on_request in brew output".to_string())
            })?;
        installed.push(InstalledPackage {
            name: name.to_owned(),
            package_type: PackageType::Brew,
            version: version_string.to_owned(),
            installed_on_request: on_request,
        });
    }
    Ok(installed)
}

/// Parse installed casks from brew info JSON.
fn parse_installed_casks(json: &serde_json::Value) -> Result<Vec<InstalledPackage>> {
    let mut installed = Vec::new();
    for cask in field_array(json, "casks")? {
        let name = required_string(cask, "token")?;
        match cask.get("installed") {
            Some(serde_json::Value::String(version)) if !version.is_empty() => {
                installed.push(InstalledPackage {
                    name: name.to_owned(),
                    package_type: PackageType::Cask,
                    version: version.clone(),
                    installed_on_request: true,
                });
            }
            _ => {
                return Err(Error::Other(
                    "missing or invalid cask installed field".to_string(),
                ));
            }
        }
    }
    Ok(installed)
}

/// Parse bundle output to extract results.
fn parse_bundle_output(stdout: &str, stderr: &str, success: bool) -> Result<BundleResult> {
    let mut result = BundleResult::default();
    let combined = format!("{stdout}\n{stderr}");

    for line in combined.lines() {
        let line = line.trim();

        // Patterns for success
        if line.starts_with("Installing ") || line.starts_with("Tapping ") {
            if let Some(name) = extract_package_name(line) {
                result.installed.push(name);
            }
        } else if (line.contains("already installed") || line.starts_with("Using "))
            && let Some(name) = extract_package_name(line)
        {
            result.skipped.push(name);
        } else if line.starts_with("Upgrading ")
            && let Some(name) = extract_package_name(line)
        {
            result.upgraded.push(name);
        } else if (line.contains("Error:") || line.contains("failed"))
            && let Some(name) = extract_package_name(line)
        {
            let error_msg = line.to_string();
            result.failed.push((name, error_msg));
        }
    }

    // If we couldn't parse anything but the command failed, report generic error
    if !success && result.failed.is_empty() && result.installed.is_empty() {
        result
            .failed
            .push(("bundle".to_string(), stderr.to_string()));
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
        if !name.is_empty() && !name.starts_with('-') && !name.contains('=') {
            return Some(name.to_string());
        }
    }
    None
}

fn run_query(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program).args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound && !executable_may_exist(program) {
            Error::ExecutableNotFound {
                program: program.to_owned(),
            }
        } else {
            Error::CommandFailed {
                message: format!("failed to execute {program}: {e}"),
                stderr: String::new(),
            }
        }
    })?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            message: format!("{program} query failed ({})", output.status),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    String::from_utf8(output.stdout)
        .map_err(|e| Error::Other(format!("invalid UTF-8 in {program} output: {e}")))
}

// ENOENT can also mean a present script has a missing interpreter. If PATH
// cannot be inspected, retain the failure rather than claiming absence.
fn executable_may_exist(program: &str) -> bool {
    fn may_exist(path: &Path) -> bool {
        match std::fs::symlink_metadata(path) {
            Ok(_) => true,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        }
    }
    if Path::new(program).components().count() > 1 {
        return may_exist(Path::new(program));
    }
    let Some(path) = std::env::var_os("PATH") else {
        return true;
    };
    std::env::split_paths(&path).any(|dir| may_exist(&dir.join(program)))
}

// =============================================================================
// mas (Mac App Store) helpers
// =============================================================================

fn run_mas_install(app_id: &str) -> Result<()> {
    let output = Command::new("mas")
        .args(["install", app_id])
        .output()
        .map_err(|e| Error::CommandFailed {
            message: format!("failed to execute mas: {e}"),
            stderr: String::new(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::CommandFailed {
            message: format!("mas install failed for {app_id}"),
            stderr: stderr.to_string(),
        });
    }

    Ok(())
}

fn is_mas_installed(app_id: &str) -> Result<bool> {
    let output = run_query("mas", &["list"])?;
    Ok(parse_mas_list(&output)?.iter().any(|(id, _)| id == app_id))
}

fn list_mas_installed() -> Result<Vec<InstalledPackage>> {
    let output = run_query("mas", &["list"])?;
    Ok(parse_mas_list(&output)?
        .into_iter()
        .map(|(_, package)| package)
        .collect())
}

fn parse_mas_list(output: &str) -> Result<Vec<(String, InstalledPackage)>> {
    let mut installed = Vec::new();
    for line in output.lines() {
        let (id, rest) = line
            .split_once(' ')
            .ok_or_else(|| Error::Other("invalid mas list output".to_string()))?;
        let rest = rest.trim();
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) || rest.is_empty() {
            return Err(Error::Other("invalid mas list output".to_string()));
        }
        let (name, version) = if let Some(paren_pos) = rest.rfind('(') {
            if !rest.ends_with(')') {
                return Err(Error::Other("invalid mas version".to_string()));
            }
            (
                rest[..paren_pos].trim(),
                &rest[paren_pos + 1..rest.len() - 1],
            )
        } else {
            (rest, "")
        };
        if name.is_empty() {
            return Err(Error::Other("missing mas app name".to_string()));
        }
        installed.push((
            id.to_owned(),
            InstalledPackage {
                name: format!("{name} ({id})"),
                package_type: PackageType::Mas,
                version: version.to_owned(),
                installed_on_request: true,
            },
        ));
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
            message: format!("failed to execute code: {e}"),
            stderr: String::new(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::CommandFailed {
            message: format!("vscode install failed for {extension}"),
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
            message: format!("failed to execute code: {e}"),
            stderr: String::new(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::CommandFailed {
            message: format!("vscode uninstall failed for {extension}"),
            stderr: stderr.to_string(),
        });
    }

    Ok(())
}

fn is_vscode_installed(extension: &str) -> Result<bool> {
    let output = run_query("code", &["--list-extensions"])?;
    Ok(parse_vscode_list(&output, false)?
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(extension)))
}

fn list_vscode_installed() -> Result<Vec<InstalledPackage>> {
    let output = run_query("code", &["--list-extensions", "--show-versions"])?;
    parse_vscode_list(&output, true)
}

fn parse_vscode_list(output: &str, with_versions: bool) -> Result<Vec<InstalledPackage>> {
    let mut installed = Vec::new();
    for line in output.lines() {
        let (name, version) = if with_versions {
            line.rsplit_once('@')
                .filter(|(_, version)| !version.is_empty())
                .ok_or_else(|| {
                    Error::Other("invalid VS Code extension version output".to_string())
                })?
        } else {
            (line, "")
        };
        let valid_name = name.split_once('.').is_some_and(|(publisher, extension)| {
            !publisher.is_empty() && !extension.is_empty() && !name.chars().any(char::is_whitespace)
        });
        if !valid_name {
            return Err(Error::Other("invalid VS Code extension output".to_string()));
        }
        installed.push(InstalledPackage {
            name: name.to_owned(),
            package_type: PackageType::Vscode,
            version: version.to_owned(),
            installed_on_request: true,
        });
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn fake_brew(script: &str) -> (tempfile::TempDir, BrewBackend) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brew");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let backend = BrewBackend {
            brew_path: path.to_str().unwrap().to_owned(),
        };
        (dir, backend)
    }

    #[cfg(unix)]
    #[test]
    fn observation_failures_do_not_mean_absent() {
        let (dir, backend) = fake_brew("echo permission-denied >&2; exit 1");
        for package in [
            Package::brew("git"),
            Package::cask("app"),
            Package::tap("org/tap"),
        ] {
            assert!(backend.is_installed(&package).is_err());
        }
        for kind in [PackageType::Brew, PackageType::Cask, PackageType::Tap] {
            assert!(backend.list_installed(kind).is_err());
        }
        assert!(backend.get_version(&Package::brew("git")).is_err());
        assert!(!backend.is_available());
        assert!(matches!(
            run_query("/usr/bin/false", &[]),
            Err(Error::CommandFailed { .. })
        ));
        assert!(matches!(
            run_query("/bossa-test-missing-executable", &[]),
            Err(Error::ExecutableNotFound { .. })
        ));
        let (broken_dir, broken) = fake_brew("exit 0");
        std::fs::write(
            broken_dir.path().join("brew"),
            "#!/bossa-missing-interpreter\n",
        )
        .unwrap();
        assert!(matches!(
            run_query(&broken.brew_path, &[]),
            Err(Error::CommandFailed { .. })
        ));
        use std::os::unix::fs::PermissionsExt;
        let no_exec = dir.path().join("no-exec");
        std::fs::write(&no_exec, "not executable").unwrap();
        std::fs::set_permissions(&no_exec, std::fs::Permissions::from_mode(0o600)).unwrap();
        let denied = BrewBackend {
            brew_path: no_exec.to_str().unwrap().to_owned(),
        };
        assert!(denied.is_installed(&Package::brew("git")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn decode_errors_do_not_mean_absent() {
        let (_dir, backend) = fake_brew("printf '\\377'");
        assert!(backend.is_installed(&Package::tap("org/tap")).is_err());
        assert!(backend.is_installed(&Package::brew("git")).is_err());
        let (_dir, malformed) = fake_brew("printf '{}' ");
        assert!(malformed.is_installed(&Package::brew("git")).is_err());
        assert!(malformed.list_installed(PackageType::Brew).is_err());
        assert!(parse_mas_list("malformed line").is_err());
        assert!(parse_vscode_list("warning text", false).is_err());
        assert!(parse_vscode_list("publisher.extension", true).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn successful_mutations_ignore_non_utf8_stdout() {
        let (_dir, backend) = fake_brew("printf '\\377'");
        let package = Package::brew("git");
        assert!(backend.install(&package).is_ok());
        assert!(backend.uninstall(&package).is_ok());
        assert!(backend.update().is_ok());
        assert!(backend.upgrade(Some(&package)).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn valid_absent_and_installed_observations_are_distinct() {
        let (_dir, absent) = fake_brew("printf '%s' '{\"formulae\":[{\"installed\":[]}]}'");
        assert!(!absent.is_installed(&Package::brew("git")).unwrap());
        let (_dir, present) =
            fake_brew("printf '%s' '{\"formulae\":[{\"installed\":[{\"version\":\"1.2\"}]}]}'");
        assert!(present.is_installed(&Package::brew("git")).unwrap());
        assert_eq!(
            present
                .get_version(&Package::brew("git"))
                .unwrap()
                .as_deref(),
            Some("1.2")
        );
        assert!(
            parse_installed_formulas(&serde_json::json!({"formulae":[]}))
                .unwrap()
                .is_empty()
        );
        assert!(parse_installed_casks(&serde_json::json!({})).is_err());
        assert!(
            parse_installed_formulas(
                &serde_json::json!({"formulae":[{"name":"git", "installed":[]}]})
            )
            .is_err()
        );
        assert!(
            parse_installed_casks(
                &serde_json::json!({"casks":[{"token":"app", "installed":null}]})
            )
            .is_err()
        );
        assert!(
            parse_installed_casks(&serde_json::json!({"casks":[]}))
                .unwrap()
                .is_empty()
        );
        let apps = parse_mas_list("123 App (1.0)\n").unwrap();
        assert_eq!(apps[0].0, "123");
    }

    #[test]
    fn test_extract_package_name() {
        assert_eq!(
            extract_package_name("Installing git"),
            Some("git".to_string())
        );
        assert_eq!(
            extract_package_name("Installing git 2.40.0"),
            Some("git".to_string())
        );
        assert_eq!(
            extract_package_name("Tapping homebrew/cask"),
            Some("homebrew/cask".to_string())
        );
        assert_eq!(extract_package_name("Using curl"), Some("curl".to_string()));
    }

    #[test]
    fn test_parse_bundle_output_success() {
        let stdout = "Installing git\nInstalling curl\nUsing wget";
        let result = parse_bundle_output(stdout, "", true).unwrap();

        assert_eq!(result.installed, vec!["git", "curl"]);
        assert_eq!(result.skipped, vec!["wget"]);
        assert_eq!(result.failed.len(), 0);
    }

    #[test]
    fn test_parse_bundle_output_with_errors() {
        let stderr = "Error: foo: not found";
        let result = parse_bundle_output("", stderr, false).unwrap();

        assert_ne!(result.failed.len(), 0);
    }
}
