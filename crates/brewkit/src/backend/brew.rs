//! Real Homebrew CLI backend using `brew` commands.

use crate::backend::Backend;
use crate::error::{Error, Result};
use crate::types::{BundleResult, InstalledPackage, Package, PackageType};
use bossa_process::{CommandRequest, IoMode, ProcessLimits, ProcessRunner, SystemRunner};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

/// Query and mutation budgets. Both cap combined stdout and stderr at 32 MiB.
#[derive(Clone, Copy, Debug)]
pub struct BrewLimits {
    /// Inventory observations default to 60 seconds.
    pub query: ProcessLimits,
    /// Mutations default to 30 minutes; timeout does not undo effects.
    pub mutation: ProcessLimits,
}
impl Default for BrewLimits {
    fn default() -> Self {
        Self {
            query: ProcessLimits {
                timeout: Duration::from_secs(60),
                max_output_bytes: 32 * 1024 * 1024,
            },
            mutation: ProcessLimits {
                timeout: Duration::from_secs(30 * 60),
                max_output_bytes: 32 * 1024 * 1024,
            },
        }
    }
}

/// Backend using a captured environment and explicitly bounded native execution.
pub struct BrewBackend {
    brew_path: PathBuf,
    cwd: PathBuf,
    environment: BTreeMap<OsString, OsString>,
    runner: SystemRunner,
    limits: BrewLimits,
}

impl BrewBackend {
    /// Capture environment and working directory once; find Homebrew in standard locations or PATH.
    pub fn new() -> Result<Self> {
        let cwd = std::env::current_dir()?;
        let environment = std::env::vars_os().collect();
        let mut found = None;
        for path in [
            "/opt/homebrew/bin/brew",
            "/usr/local/bin/brew",
            "/home/linuxbrew/.linuxbrew/bin/brew",
        ] {
            match std::fs::symlink_metadata(path) {
                Ok(_) => {
                    found = Some(PathBuf::from(path));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::Io(error)),
            }
        }
        let brew_path = match found {
            Some(path) => path,
            None => {
                resolve_program(OsStr::new("brew"), &cwd, &environment).map_err(
                    |error| match error {
                        Error::ExecutableNotFound { .. } => Error::BrewNotFound,
                        other => other,
                    },
                )?
            }
        };
        Ok(Self::with_context(brew_path, cwd, environment))
    }

    /// Supply an executable, working directory and environment without reading process globals.
    pub fn with_context(
        brew_path: PathBuf,
        cwd: PathBuf,
        environment: BTreeMap<OsString, OsString>,
    ) -> Self {
        let runner = SystemRunner::new(environment.clone());
        Self {
            brew_path,
            cwd,
            environment,
            runner,
            limits: BrewLimits::default(),
        }
    }

    /// Override execution budgets. Failures after launch may leave partial mutations.
    pub fn with_limits(mut self, limits: BrewLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Share the runner cancellation domain with a caller.
    pub fn runner(&self) -> SystemRunner {
        self.runner.clone()
    }

    fn execute(&self, program: &OsStr, args: &[OsString], limits: ProcessLimits) -> Result<Output> {
        if !self.cwd.is_absolute() {
            return Err(Error::Other(
                "working directory must be absolute".to_owned(),
            ));
        }
        let path = resolve_program(program, &self.cwd, &self.environment)?;
        let request = CommandRequest {
            program: path.into_os_string(),
            args: args.to_vec(),
            working_dir: self.cwd.clone(),
        };
        self.runner.clone().with_limits(limits).run(&request, IoMode::Capture)
            .map_err(|error| Error::CommandFailed {
                message: format!("process execution failed: {error}; any started mutation may have partially completed"),
                stderr: String::new(),
            })
    }

    fn run_brew(&self, args: &[&str], limits: ProcessLimits) -> Result<Output> {
        self.execute(
            self.brew_path.as_os_str(),
            &args.iter().map(OsString::from).collect::<Vec<_>>(),
            limits,
        )
    }

    fn run_brew_success(&self, args: &[&str], package_name: Option<&str>) -> Result<Output> {
        let output = self.run_brew(args, self.limits.mutation)?;
        if !output.status.success() {
            return Err(Error::from_brew_output(
                &String::from_utf8_lossy(&output.stderr),
                package_name,
            ));
        }
        Ok(output)
    }

    fn run_brew_checked(&self, args: &[&str], package_name: Option<&str>) -> Result<String> {
        let output = self.run_brew(args, self.limits.query)?;
        if !output.status.success() {
            return Err(Error::from_brew_output(
                &String::from_utf8_lossy(&output.stderr),
                package_name,
            ));
        }
        String::from_utf8(output.stdout)
            .map_err(|e| Error::Other(format!("invalid UTF-8 in brew output: {e}")))
    }

    fn run_query(&self, program: &str, args: &[&str]) -> Result<String> {
        let output = self.execute(
            OsStr::new(program),
            &args.iter().map(OsString::from).collect::<Vec<_>>(),
            self.limits.query,
        )?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                message: format!("{program} query failed ({})", output.status),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        String::from_utf8(output.stdout)
            .map_err(|e| Error::Other(format!("invalid UTF-8 in {program} output: {e}")))
    }

    fn mutate_aux(&self, program: &str, args: &[&str]) -> Result<()> {
        let output = self.execute(
            OsStr::new(program),
            &args.iter().map(OsString::from).collect::<Vec<_>>(),
            self.limits.mutation,
        )?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                message: format!("{program} command failed ({})", output.status),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(())
    }
}

// Resolve against only the captured PATH and working directory. A present but
// broken candidate is never treated as proof that the tool is absent.
fn candidate_exists(candidate: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(candidate) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A missing entry is not proof of absence if an ancestor is a
            // dangling symlink or cannot be inspected.
            for ancestor in candidate.ancestors().skip(1) {
                match std::fs::symlink_metadata(ancestor) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        std::fs::metadata(ancestor)?;
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn resolve_program(
    program: &OsStr,
    cwd: &Path,
    environment: &BTreeMap<OsString, OsString>,
) -> Result<PathBuf> {
    let path = Path::new(program);
    if path.components().count() > 1 || path.is_absolute() {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        return if candidate_exists(&candidate)? {
            Ok(candidate)
        } else {
            Err(Error::ExecutableNotFound {
                program: program.to_string_lossy().into_owned(),
            })
        };
    }
    let search = environment
        .get(OsStr::new("PATH"))
        .ok_or_else(|| Error::Other("executable search requires an explicit PATH".to_owned()))?;
    let mut uncertain = None;
    for dir in std::env::split_paths(search) {
        let dir = if dir.is_absolute() {
            dir
        } else {
            cwd.join(dir)
        };
        let candidate = dir.join(path);
        match candidate_exists(&candidate) {
            Ok(true) => return Ok(candidate),
            Ok(false) => {}
            Err(error) => uncertain = Some(error),
        }
    }
    if let Some(error) = uncertain {
        return Err(Error::Io(error));
    }
    Err(Error::ExecutableNotFound {
        program: program.to_string_lossy().into_owned(),
    })
}

impl Backend for BrewBackend {
    fn is_available(&self) -> Result<bool> {
        match self.run_brew_checked(&["--version"], None) {
            Ok(_) => Ok(true),
            Err(Error::ExecutableNotFound { .. }) => Ok(false),
            Err(error) => Err(error),
        }
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
                return self.mutate_aux("mas", &["install", id]);
            }
            PackageType::Vscode => {
                // code --install-extension <ext_id>
                return self.mutate_aux("code", &["--install-extension", &package.name]);
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
                return self.mutate_aux("code", &["--uninstall-extension", &package.name]);
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
                self.is_mas_installed(id)
            }
            PackageType::Vscode => self.is_vscode_installed(&package.name),
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
            PackageType::Mas => self.list_mas_installed(),
            PackageType::Vscode => self.list_vscode_installed(),
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
        let mut args = vec![
            OsString::from("bundle"),
            OsString::from("--file"),
            brewfile_path.as_os_str().to_os_string(),
        ];
        if verbose {
            args.push("--verbose".into());
        }
        let output = self.execute(self.brew_path.as_os_str(), &args, self.limits.mutation)?;
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

    // A nonzero exit is a failure even after success-looking output.
    if !success && result.failed.is_empty() {
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

impl BrewBackend {
    fn is_mas_installed(&self, app_id: &str) -> Result<bool> {
        Ok(parse_mas_list(&self.run_query("mas", &["list"])?)?
            .iter()
            .any(|(id, _)| id == app_id))
    }
    fn list_mas_installed(&self) -> Result<Vec<InstalledPackage>> {
        Ok(parse_mas_list(&self.run_query("mas", &["list"])?)?
            .into_iter()
            .map(|(_, package)| package)
            .collect())
    }
    fn is_vscode_installed(&self, extension: &str) -> Result<bool> {
        Ok(
            parse_vscode_list(&self.run_query("code", &["--list-extensions"])?, false)?
                .iter()
                .any(|p| p.name.eq_ignore_ascii_case(extension)),
        )
    }
    fn list_vscode_installed(&self) -> Result<Vec<InstalledPackage>> {
        parse_vscode_list(
            &self.run_query("code", &["--list-extensions", "--show-versions"])?,
            true,
        )
    }
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

    // Another test can fork while a fixture is being written. The forked
    // child temporarily inherits the writable descriptor until exec, causing
    // ETXTBSY if a sibling test executes that inode in the meantime.
    #[cfg(unix)]
    fn process_fixture_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(unix)]
    fn fake_brew(script: &str) -> (tempfile::TempDir, BrewBackend) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brew");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let environment =
            BTreeMap::from([(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))]);
        let backend = BrewBackend::with_context(path, dir.path().to_path_buf(), environment);
        (dir, backend)
    }

    #[cfg(unix)]
    #[test]
    fn injected_context_uses_only_its_path_and_working_directory() {
        let _guard = process_fixture_guard();
        use std::os::unix::fs::PermissionsExt;
        let (dir, _) = fake_brew("exit 0");
        let code = dir.path().join("code");
        std::fs::write(&code, "#!/bin/sh\nprintf '%s' \"$PWD\"\n").unwrap();
        std::fs::set_permissions(&code, std::fs::Permissions::from_mode(0o700)).unwrap();
        let backend = BrewBackend::with_context(
            dir.path().join("brew"),
            dir.path().to_path_buf(),
            BTreeMap::from([("PATH".into(), ".".into())]),
        );
        assert_eq!(
            std::fs::canonicalize(backend.run_query("code", &[]).unwrap()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
        assert!(matches!(
            backend.run_query("mas", &[]),
            Err(Error::ExecutableNotFound { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn dangling_path_is_not_optional_tool_absence() {
        let _guard = process_fixture_guard();
        use std::os::unix::fs::PermissionsExt;
        let (dir, _) = fake_brew("exit 0");
        let broken = dir.path().join("broken");
        std::os::unix::fs::symlink(dir.path().join("missing"), &broken).unwrap();
        let environment = BTreeMap::from([("PATH".into(), broken.clone().into_os_string())]);
        let backend = BrewBackend::with_context(
            dir.path().join("brew"),
            dir.path().to_path_buf(),
            environment,
        );
        assert!(matches!(backend.run_query("mas", &[]), Err(Error::Io(_))));
        let tool = dir.path().join("mas");
        std::fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = std::env::join_paths([broken.as_path(), dir.path()]).unwrap();
        let backend = BrewBackend::with_context(
            dir.path().join("brew"),
            dir.path().to_path_buf(),
            BTreeMap::from([("PATH".into(), path)]),
        );
        assert_eq!(backend.run_query("mas", &[]).unwrap(), "");
        let absent = BrewBackend::with_context(
            dir.path().join("missing-brew"),
            dir.path().to_path_buf(),
            BTreeMap::new(),
        );
        assert!(!absent.is_available().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn query_and_mutation_limits_and_cancellation_are_errors() {
        let _guard = process_fixture_guard();
        let (_dir, backend) = fake_brew("/bin/sleep 3");
        let short = ProcessLimits {
            timeout: Duration::from_millis(100),
            max_output_bytes: 32,
        };
        let backend = backend.with_limits(BrewLimits {
            query: short,
            mutation: short,
        });
        assert!(backend.is_installed(&Package::tap("org/tap")).is_err());
        assert!(backend.install(&Package::brew("git")).is_err());
        let runner = backend.runner();
        runner.cancel().unwrap();
        assert!(backend.install(&Package::brew("git")).is_err());
        let (_dir, backend) = fake_brew("printf 1234; printf 5678 >&2");
        let small = ProcessLimits {
            timeout: Duration::from_secs(3),
            max_output_bytes: 7,
        };
        let backend = backend.with_limits(BrewLimits {
            query: small,
            mutation: small,
        });
        assert!(backend.is_installed(&Package::tap("org/tap")).is_err());
        assert!(backend.install(&Package::brew("git")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn bundle_preserves_native_non_utf8_path() {
        let _guard = process_fixture_guard();
        use std::os::unix::ffi::OsStringExt;
        let (dir, mut backend) = fake_brew("printf '%s' \"$3\" > marker");
        let path = dir.path().join(OsString::from_vec(vec![b'f', 0xff]));
        backend.cwd = dir.path().to_path_buf();
        backend.bundle(&path, false).unwrap();
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            std::fs::read(dir.path().join("marker")).unwrap(),
            path.as_os_str().as_bytes()
        );
    }

    #[cfg(unix)]
    #[test]
    fn observation_failures_do_not_mean_absent() {
        let _guard = process_fixture_guard();
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
        assert!(backend.is_available().is_err());
        assert!(matches!(
            backend.run_query("/usr/bin/false", &[]),
            Err(Error::CommandFailed { .. })
        ));
        assert!(matches!(
            backend.run_query("bossa-test-missing-executable", &[]),
            Err(Error::ExecutableNotFound { .. })
        ));
        let (broken_dir, broken) = fake_brew("exit 0");
        std::fs::write(
            broken_dir.path().join("brew"),
            "#!/bossa-missing-interpreter\n",
        )
        .unwrap();
        assert!(matches!(
            broken.run_query(broken.brew_path.to_str().unwrap(), &[]),
            Err(Error::CommandFailed { .. })
        ));
        use std::os::unix::fs::PermissionsExt;
        let no_exec = dir.path().join("no-exec");
        std::fs::write(&no_exec, "not executable").unwrap();
        std::fs::set_permissions(&no_exec, std::fs::Permissions::from_mode(0o600)).unwrap();
        let denied = BrewBackend::with_context(no_exec, dir.path().to_path_buf(), BTreeMap::new());
        assert!(denied.is_installed(&Package::brew("git")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn decode_errors_do_not_mean_absent() {
        let _guard = process_fixture_guard();
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
        let _guard = process_fixture_guard();
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
        let _guard = process_fixture_guard();
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
    fn failed_bundle_after_install_output_is_not_success() {
        let result = parse_bundle_output("Installing git", "unrecognized failure", false).unwrap();
        assert_eq!(result.installed, ["git"]);
        assert!(!result.is_success());
        assert_eq!(result.failed.len(), 1);
    }

    #[test]
    fn test_parse_bundle_output_with_errors() {
        let stderr = "Error: foo: not found";
        let result = parse_bundle_output("", stderr, false).unwrap();

        assert_ne!(result.failed.len(), 0);
    }
}
