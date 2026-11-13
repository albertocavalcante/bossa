use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Environment inputs captured before dispatch, or supplied explicitly by a caller.
///
/// Values are private and deliberately have no Debug implementation: they may
/// contain credentials. Constructing this value never modifies process globals.
#[derive(Clone)]
pub struct AppEnvironment {
    home: Option<PathBuf>,
    variables: BTreeMap<OsString, OsString>,
    #[cfg(windows)]
    platform_config: Option<PathBuf>,
    #[cfg(windows)]
    platform_state: Option<PathBuf>,
}

impl AppEnvironment {
    /// Supply inputs without reading the live environment.
    pub fn new(
        home: Option<PathBuf>,
        variables: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Self {
        Self {
            home,
            variables: variables.into_iter().collect(),
            #[cfg(windows)]
            platform_config: None,
            #[cfg(windows)]
            platform_state: None,
        }
    }

    /// Capture native inputs once at the application's outer boundary.
    pub fn capture() -> Self {
        let value = Self::new(dirs::home_dir(), std::env::vars_os());
        #[cfg(windows)]
        let value = Self {
            platform_config: dirs::config_dir(),
            platform_state: dirs::data_local_dir(),
            ..value
        };
        value
    }

    pub fn home_dir(&self) -> Result<&Path> {
        self.home
            .as_deref()
            .context("Could not determine home directory")
    }

    pub fn variables(&self) -> &BTreeMap<OsString, OsString> {
        &self.variables
    }

    pub fn variable(&self, name: &str) -> Option<&OsStr> {
        lookup_variable(&self.variables, name)
    }

    /// Expand shell-style string paths using only captured values.
    /// Unknown variables preserve the original input, matching the legacy helper.
    pub fn expand(&self, path: &str) -> PathBuf {
        let expanded = shellexpand::full_with_context(
            path,
            || self.home.as_deref().and_then(Path::to_str),
            |name| {
                self.variable(name)
                    .and_then(OsStr::to_str)
                    .map(Some)
                    .ok_or(())
            },
        );
        PathBuf::from(expanded.as_deref().unwrap_or(path))
    }

    fn override_path(&self, name: &str) -> Option<PathBuf> {
        self.variable(name).map(|value| {
            // Preserve OS-native literal paths. Shell expansion applies to UTF-8
            // strings, as in the existing configuration path convention.
            value
                .to_str()
                .map_or_else(|| PathBuf::from(value), |value| self.expand(value))
        })
    }

    /// Preserve the existing config-directory precedence, without creating it.
    pub fn config_dir(&self) -> Result<PathBuf> {
        if let Some(path) = self.override_path("BOSSA_CONFIG_DIR") {
            return Ok(path);
        }
        let existing = self.home.as_ref().map(|home| home.join(".config/bossa"));
        if let Some(path) = existing {
            match std::fs::symlink_metadata(&path) {
                Ok(_) => {
                    std::fs::metadata(&path).with_context(|| {
                        format!("Could not inspect config directory {}", path.display())
                    })?;
                    return Ok(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    crate::confirm_missing_parent(&path)
                        .context("Could not inspect the home config directory")?;
                }
                Err(error) => {
                    return Err(error).context("Could not inspect the home config directory");
                }
            }
        }
        if let Some(path) = self.variable("XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(path).join("bossa"));
        }
        #[cfg(windows)]
        if let Some(path) = &self.platform_config {
            return Ok(path.join("bossa"));
        }
        Ok(self.home_dir()?.join(".config/bossa"))
    }

    pub fn state_dir(&self) -> Result<PathBuf> {
        if let Some(path) = self.override_path("BOSSA_STATE_DIR") {
            return Ok(path);
        }
        if let Some(path) = self.variable("XDG_STATE_HOME") {
            return Ok(PathBuf::from(path).join("bossa"));
        }
        #[cfg(windows)]
        if let Some(path) = &self.platform_state {
            return Ok(path.join("bossa"));
        }
        Ok(self.home_dir()?.join(".local/state/bossa"))
    }

    pub fn workspaces_dir(&self) -> Result<PathBuf> {
        self.override_path("BOSSA_WORKSPACES_DIR")
            .map_or_else(|| Ok(self.home_dir()?.join("dev/ws")), Ok)
    }
}

pub(crate) fn lookup_variable<'a>(
    variables: &'a BTreeMap<OsString, OsString>,
    name: &str,
) -> Option<&'a OsStr> {
    #[cfg(windows)]
    {
        // Standard Windows environment names are ASCII and case insensitive;
        // retain original OS-native spelling and values for child processes.
        variables.iter().find_map(|(key, value)| {
            key.to_str()
                .is_some_and(|key| key.eq_ignore_ascii_case(name))
                .then_some(value.as_os_str())
        })
    }
    #[cfg(not(windows))]
    {
        variables.get(OsStr::new(name)).map(OsString::as_os_str)
    }
}

/// Resolved roots for one application invocation. Resolution performs reads only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config: PathBuf,
    pub state: PathBuf,
    pub workspaces: PathBuf,
}

impl AppPaths {
    pub fn resolve(environment: &AppEnvironment) -> Result<Self> {
        Ok(Self {
            config: environment.config_dir()?,
            state: environment.state_dir()?,
            workspaces: environment.workspaces_dir()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn environment(home: &Path, pairs: &[(&str, &str)]) -> AppEnvironment {
        AppEnvironment::new(
            Some(home.to_path_buf()),
            pairs
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into())),
        )
    }

    #[test]
    fn precedence_and_expansion_use_only_supplied_values() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();
        let env = environment(
            home,
            &[
                ("BOSSA_CONFIG_DIR", "~/config with spaces"),
                ("BOSSA_STATE_DIR", "$ROOT/state"),
                ("BOSSA_WORKSPACES_DIR", "${ROOT}/workspaces"),
                ("ROOT", "/fixture"),
                ("XDG_CONFIG_HOME", "/ignored"),
            ],
        );
        let paths = AppPaths::resolve(&env).unwrap();
        assert_eq!(paths.config, home.join("config with spaces"));
        assert_eq!(paths.state, PathBuf::from("/fixture/state"));
        assert_eq!(paths.workspaces, PathBuf::from("/fixture/workspaces"));
        assert!(!paths.config.exists());
        assert_eq!(env.expand("$UNKNOWN/file"), PathBuf::from("$UNKNOWN/file"));
    }

    #[test]
    fn existing_home_config_precedes_xdg_without_creating_other_roots() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();
        let env = environment(home, &[("XDG_CONFIG_HOME", "/xdg")]);
        assert_eq!(env.config_dir().unwrap(), PathBuf::from("/xdg/bossa"));
        std::fs::create_dir_all(home.join(".config/bossa")).unwrap();
        assert_eq!(env.config_dir().unwrap(), home.join(".config/bossa"));
        assert_eq!(env.state_dir().unwrap(), home.join(".local/state/bossa"));
        assert_eq!(env.workspaces_dir().unwrap(), home.join("dev/ws"));
        assert!(!home.join(".local").exists());
    }

    #[test]
    fn absent_home_is_an_error_unless_each_root_is_explicit() {
        let env = AppEnvironment::new(None, []);
        assert!(AppPaths::resolve(&env).is_err());
        let env = AppEnvironment::new(
            None,
            [
                ("BOSSA_CONFIG_DIR".into(), "/config".into()),
                ("BOSSA_STATE_DIR".into(), "/state".into()),
                ("BOSSA_WORKSPACES_DIR".into(), "/workspaces".into()),
            ],
        );
        assert!(AppPaths::resolve(&env).is_ok());
    }

    #[test]
    fn environment_key_lookup_matches_platform_case_rules() {
        let env = AppEnvironment::new(None, [("Path".into(), "fixture-bin".into())]);
        #[cfg(windows)]
        assert_eq!(env.variable("PATH"), Some(OsStr::new("fixture-bin")));
        #[cfg(not(windows))]
        assert!(env.variable("PATH").is_none());
        assert_eq!(env.variable("Path"), Some(OsStr::new("fixture-bin")));
    }

    #[cfg(unix)]
    #[test]
    fn unavailable_home_config_parent_does_not_fall_back_to_xdg() {
        let temp = TempDir::new().unwrap();
        std::os::unix::fs::symlink(temp.path().join("missing"), temp.path().join(".config"))
            .unwrap();
        let env = environment(temp.path(), &[("XDG_CONFIG_HOME", "/must-not-select")]);
        assert!(env.config_dir().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_literal_override_is_preserved() {
        use std::os::unix::ffi::OsStringExt;
        let value = OsString::from_vec(b"/fixture/\xff".to_vec());
        let env = AppEnvironment::new(None, [("BOSSA_CONFIG_DIR".into(), value.clone())]);
        assert_eq!(env.config_dir().unwrap().as_os_str(), value);
    }
}
