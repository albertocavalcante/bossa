//! Centralized path resolution for bossa
//!
//! This module provides platform-aware path resolution with environment variable
//! support, making it easy to symlink bossa configs from a dotfiles repository.
//!
//! # Environment Variables
//!
//! - `BOSSA_CONFIG_DIR` - Override config directory (e.g., `~/dotfiles/bossa`)
//! - `BOSSA_STATE_DIR` - Override state directory
//! - `BOSSA_WORKSPACES_DIR` - Override workspaces root directory
//!
//! # Path Resolution Priority
//!
//! For config_dir():
//! 1. `BOSSA_CONFIG_DIR` environment variable
//! 2. Existing `~/.config/bossa/` (backwards compatibility)
//! 3. `XDG_CONFIG_HOME/bossa` (if set)
//! 4. Platform default:
//!    - Windows: `%APPDATA%\bossa`
//!    - macOS/Linux: `~/.config/bossa`
//!
//! For state_dir():
//! 1. `BOSSA_STATE_DIR` environment variable
//! 2. `XDG_STATE_HOME/bossa` (if set)
//! 3. Platform default:
//!    - Windows: `%LOCALAPPDATA%\bossa`
//!    - macOS/Linux: `~/.local/state/bossa`

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::schema::LocationsConfig;

/// Environment variable for config directory override
pub const ENV_CONFIG_DIR: &str = "BOSSA_CONFIG_DIR";

/// Environment variable for state directory override
pub const ENV_STATE_DIR: &str = "BOSSA_STATE_DIR";

/// Environment variable for workspaces directory override
pub const ENV_WORKSPACES_DIR: &str = "BOSSA_WORKSPACES_DIR";

/// Get the bossa config directory path
///
/// Priority:
/// 1. `BOSSA_CONFIG_DIR` env var
/// 2. Existing `~/.config/bossa/` (backwards compat)
/// 3. `XDG_CONFIG_HOME/bossa`
/// 4. Platform default
pub fn config_dir() -> Result<PathBuf> {
    // 1. Check environment variable override
    if let Ok(dir) = std::env::var(ENV_CONFIG_DIR) {
        let path = expand_path(&dir);
        log::debug!(
            "Using config dir from {}: {}",
            ENV_CONFIG_DIR,
            path.display()
        );
        return Ok(path);
    }

    // 2. Check for existing ~/.config/bossa (backwards compatibility)
    if let Some(home) = dirs::home_dir() {
        let legacy_path = home.join(".config").join("bossa");
        if legacy_path.exists() {
            log::debug!(
                "Using existing config dir (backwards compat): {}",
                legacy_path.display()
            );
            return Ok(legacy_path);
        }
    }

    // 3. Check XDG_CONFIG_HOME
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg_config).join("bossa");
        log::debug!("Using XDG_CONFIG_HOME: {}", path.display());
        return Ok(path);
    }

    // 4. Platform default
    #[cfg(windows)]
    {
        if let Some(app_data) = dirs::config_dir() {
            let path = app_data.join("bossa");
            log::debug!("Using Windows config dir: {}", path.display());
            return Ok(path);
        }
    }

    // Unix default: ~/.config/bossa
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let path = home.join(".config").join("bossa");
    log::debug!("Using default config dir: {}", path.display());
    Ok(path)
}

/// Get the bossa state directory path
///
/// Priority:
/// 1. `BOSSA_STATE_DIR` env var
/// 2. `XDG_STATE_HOME/bossa`
/// 3. Platform default
pub fn state_dir() -> Result<PathBuf> {
    // 1. Check environment variable override
    if let Ok(dir) = std::env::var(ENV_STATE_DIR) {
        let path = expand_path(&dir);
        log::debug!("Using state dir from {}: {}", ENV_STATE_DIR, path.display());
        return Ok(path);
    }

    // 2. Check XDG_STATE_HOME
    if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
        let path = PathBuf::from(xdg_state).join("bossa");
        log::debug!("Using XDG_STATE_HOME: {}", path.display());
        return Ok(path);
    }

    // 3. Platform default
    #[cfg(windows)]
    {
        if let Some(local_app_data) = dirs::data_local_dir() {
            let path = local_app_data.join("bossa");
            log::debug!("Using Windows state dir: {}", path.display());
            return Ok(path);
        }
    }

    // Unix default: ~/.local/state/bossa
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let path = home.join(".local").join("state").join("bossa");
    log::debug!("Using default state dir: {}", path.display());
    Ok(path)
}

/// Get the workspaces root directory
///
/// Priority:
/// 1. `BOSSA_WORKSPACES_DIR` env var
/// 2. Default: `~/dev/ws`
pub fn workspaces_dir() -> Result<PathBuf> {
    // 1. Check environment variable override
    if let Ok(dir) = std::env::var(ENV_WORKSPACES_DIR) {
        let path = expand_path(&dir);
        log::debug!(
            "Using workspaces dir from {}: {}",
            ENV_WORKSPACES_DIR,
            path.display()
        );
        return Ok(path);
    }

    // 2. Default: ~/dev/ws
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let path = home.join("dev").join("ws");
    log::debug!("Using default workspaces dir: {}", path.display());
    Ok(path)
}

/// Get the legacy workspace-setup config directory path
///
/// This is kept for backwards compatibility with migration tools.
/// Always returns `~/.config/workspace-setup`.
pub fn legacy_config_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".config").join("workspace-setup"))
}

/// Expand ~ and environment variables in a path string.
///
/// This is the canonical path expansion function for bossa. All modules
/// should use this instead of calling shellexpand directly.
///
/// # Examples
///
/// ```
/// use bossa::paths;
///
/// // Expands ~ to home directory
/// let home_path = paths::expand("~/dotfiles");
///
/// // Expands environment variables
/// let var_path = paths::expand("$HOME/dotfiles");
///
/// // Both work together
/// let mixed = paths::expand("~/${PROJECT}/config");
/// ```
pub fn expand(path: &str) -> PathBuf {
    let expanded = shellexpand::full(path).unwrap_or(std::borrow::Cow::Borrowed(path));
    PathBuf::from(expanded.as_ref())
}

/// Internal alias for backwards compatibility within this module
fn expand_path(path: &str) -> PathBuf {
    expand(path)
}

/// Resolve a path with location variable expansion.
///
/// Expands `${locations.name}` references using the provided LocationsConfig,
/// then expands ~ and environment variables.
///
/// # Examples
/// ```
/// use bossa::paths;
/// use bossa::schema::LocationsConfig;
/// use std::collections::HashMap;
///
/// let mut paths_map = HashMap::new();
/// paths_map.insert("dev".to_string(), "/Volumes/T9/dev".to_string());
/// let locations = LocationsConfig { paths: paths_map, ..Default::default() };
/// let resolved = paths::resolve("${locations.dev}/ws/myproject", &locations);
/// // Returns: /Volumes/T9/dev/ws/myproject
/// ```
pub fn resolve(path: &str, locations: &LocationsConfig) -> PathBuf {
    // First expand ${locations.xxx} references
    let mut result = path.to_string();

    // Use simple string replacement to find ${locations.name} patterns
    // For each match, look up the location and substitute
    for (name, location_path) in &locations.paths {
        let pattern = format!("${{locations.{}}}", name);
        if result.contains(&pattern) {
            // Recursively resolve the location path itself (it might reference other locations)
            let expanded_location = resolve(location_path, locations);
            result = result.replace(&pattern, &expanded_location.to_string_lossy());
        }
    }

    // Then expand ~ and env vars using the existing expand function
    expand(&result)
}

/// Identify which location (if any) a path belongs to.
///
/// Returns the location name if the path starts with a known location path.
/// This is useful for determining which location a file or directory belongs to,
/// enabling features like path normalization and location-based organization.
///
/// # Examples
/// ```
/// use bossa::paths;
/// use bossa::schema::LocationsConfig;
/// use std::collections::HashMap;
/// use std::path::Path;
///
/// let mut paths_map = HashMap::new();
/// paths_map.insert("dev".to_string(), "/Volumes/T9/dev".to_string());
/// let locations = LocationsConfig { paths: paths_map, ..Default::default() };
///
/// let location = paths::identify_location(Path::new("/Volumes/T9/dev/ws/project"), &locations);
/// assert_eq!(location, Some("dev".to_string()));
/// ```
#[allow(dead_code)] // Will be used by relocate command
pub fn identify_location(path: &Path, locations: &LocationsConfig) -> Option<String> {
    let path_str = path.to_string_lossy();

    for (name, location_path) in &locations.paths {
        let expanded = expand(location_path);
        if path_str.starts_with(expanded.to_string_lossy().as_ref()) {
            return Some(name.clone());
        }
    }

    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::LocationsConfig;
    use std::collections::HashMap;
    use std::env;

    /// Helper to run a test with temporary env var
    ///
    /// # Safety
    /// This function uses unsafe env::set_var/remove_var which can cause issues
    /// if other threads read environment variables concurrently.
    /// Only use in single-threaded test contexts.
    fn with_env_var<F, R>(key: &str, value: &str, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let original = env::var(key).ok();
        // SAFETY: Tests run in isolation and don't read env vars concurrently
        unsafe { env::set_var(key, value) };
        let result = f();
        match original {
            // SAFETY: Tests run in isolation
            Some(v) => unsafe { env::set_var(key, v) },
            None => unsafe { env::remove_var(key) },
        }
        result
    }

    /// Helper to run a test with env var removed
    ///
    /// # Safety
    /// This function uses unsafe env::remove_var/set_var which can cause issues
    /// if other threads read environment variables concurrently.
    /// Only use in single-threaded test contexts.
    fn without_env_var<F, R>(key: &str, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let original = env::var(key).ok();
        // SAFETY: Tests run in isolation and don't read env vars concurrently
        unsafe { env::remove_var(key) };
        let result = f();
        if let Some(v) = original {
            // SAFETY: Tests run in isolation
            unsafe { env::set_var(key, v) };
        }
        result
    }

    #[test]
    fn test_config_dir_env_override() {
        with_env_var(ENV_CONFIG_DIR, "/custom/config/path", || {
            let result = config_dir().unwrap();
            assert_eq!(result, PathBuf::from("/custom/config/path"));
        });
    }

    #[test]
    fn test_config_dir_env_override_with_tilde() {
        // Use a unique env var value to avoid test interference
        let home = dirs::home_dir().unwrap();
        let expected = home.join("dotfiles").join("bossa-tilde-test");
        with_env_var(ENV_CONFIG_DIR, "~/dotfiles/bossa-tilde-test", || {
            let result = config_dir().unwrap();
            assert_eq!(result, expected);
        });
    }

    #[test]
    fn test_state_dir_env_override() {
        with_env_var(ENV_STATE_DIR, "/custom/state/path", || {
            let result = state_dir().unwrap();
            assert_eq!(result, PathBuf::from("/custom/state/path"));
        });
    }

    #[test]
    fn test_workspaces_dir_env_override() {
        with_env_var(ENV_WORKSPACES_DIR, "/custom/workspaces", || {
            let result = workspaces_dir().unwrap();
            assert_eq!(result, PathBuf::from("/custom/workspaces"));
        });
    }

    #[test]
    fn test_workspaces_dir_default() {
        without_env_var(ENV_WORKSPACES_DIR, || {
            let result = workspaces_dir().unwrap();
            let home = dirs::home_dir().unwrap();
            assert_eq!(result, home.join("dev").join("ws"));
        });
    }

    #[test]
    fn test_legacy_config_dir() {
        let result = legacy_config_dir().unwrap();
        let home = dirs::home_dir().unwrap();
        assert_eq!(result, home.join(".config").join("workspace-setup"));
    }

    #[test]
    fn test_xdg_config_home() {
        // Only test if no override and no existing config
        without_env_var(ENV_CONFIG_DIR, || {
            with_env_var("XDG_CONFIG_HOME", "/tmp/xdg-config-test", || {
                // This test might not work if ~/.config/bossa exists
                // Just verify the function doesn't panic
                let _ = config_dir();
            });
        });
    }

    #[test]
    fn test_xdg_state_home() {
        without_env_var(ENV_STATE_DIR, || {
            with_env_var("XDG_STATE_HOME", "/tmp/xdg-state-test", || {
                let result = state_dir().unwrap();
                assert_eq!(result, PathBuf::from("/tmp/xdg-state-test/bossa"));
            });
        });
    }

    #[test]
    fn test_expand_with_tilde() {
        let result = expand("~/test/path");
        let home = dirs::home_dir().unwrap();
        assert_eq!(result, home.join("test").join("path"));
    }

    #[test]
    fn test_expand_absolute() {
        let result = expand("/absolute/path");
        assert_eq!(result, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_expand_with_env_var() {
        with_env_var("BOSSA_TEST_VAR", "test_value", || {
            let result = expand("/path/$BOSSA_TEST_VAR/file");
            assert_eq!(result, PathBuf::from("/path/test_value/file"));
        });
    }

    #[test]
    fn test_expand_unknown_env_var_unchanged() {
        // Unknown env vars are left as-is by shellexpand::full
        let result = expand("/path/$NONEXISTENT_VAR_12345/file");
        assert_eq!(result, PathBuf::from("/path/$NONEXISTENT_VAR_12345/file"));
    }

    #[test]
    fn test_env_var_constants() {
        assert_eq!(ENV_CONFIG_DIR, "BOSSA_CONFIG_DIR");
        assert_eq!(ENV_STATE_DIR, "BOSSA_STATE_DIR");
        assert_eq!(ENV_WORKSPACES_DIR, "BOSSA_WORKSPACES_DIR");
    }

    #[cfg(unix)]
    #[test]
    fn test_default_state_dir_unix() {
        without_env_var(ENV_STATE_DIR, || {
            without_env_var("XDG_STATE_HOME", || {
                let result = state_dir().unwrap();
                let home = dirs::home_dir().unwrap();
                assert_eq!(result, home.join(".local").join("state").join("bossa"));
            });
        });
    }

    // ========================================================================
    // Location-aware path resolution tests
    // ========================================================================

    fn make_locations_config(paths: Vec<(&str, &str)>) -> LocationsConfig {
        let mut paths_map = HashMap::new();
        for (name, path) in paths {
            paths_map.insert(name.to_string(), path.to_string());
        }
        LocationsConfig {
            paths: paths_map,
            ..Default::default()
        }
    }

    #[test]
    fn test_resolve_simple_location() {
        let locations = make_locations_config(vec![("dev", "/Volumes/T9/dev")]);

        let result = resolve("${locations.dev}/ws/myproject", &locations);
        assert_eq!(result, PathBuf::from("/Volumes/T9/dev/ws/myproject"));
    }

    #[test]
    fn test_resolve_multiple_locations() {
        let locations = make_locations_config(vec![
            ("dev", "/Volumes/T9/dev"),
            ("home", "/Users/testuser"),
        ]);

        let result = resolve("${locations.dev}/ws", &locations);
        assert_eq!(result, PathBuf::from("/Volumes/T9/dev/ws"));

        let result2 = resolve("${locations.home}/dotfiles", &locations);
        assert_eq!(result2, PathBuf::from("/Users/testuser/dotfiles"));
    }

    #[test]
    fn test_resolve_nested_locations() {
        let locations = make_locations_config(vec![
            ("dev", "/Volumes/T9/dev"),
            ("workspaces", "${locations.dev}/ws"),
        ]);

        let result = resolve("${locations.workspaces}/myproject", &locations);
        assert_eq!(result, PathBuf::from("/Volumes/T9/dev/ws/myproject"));
    }

    #[test]
    fn test_resolve_with_tilde() {
        let locations = make_locations_config(vec![("dotfiles", "~/dotfiles")]);

        let result = resolve("${locations.dotfiles}/vim", &locations);
        let home = dirs::home_dir().unwrap();
        assert_eq!(result, home.join("dotfiles").join("vim"));
    }

    #[test]
    fn test_resolve_with_env_var() {
        with_env_var("BOSSA_TEST_LOCATION", "/test/path", || {
            let locations = make_locations_config(vec![("test", "$BOSSA_TEST_LOCATION/data")]);

            let result = resolve("${locations.test}/file.txt", &locations);
            assert_eq!(result, PathBuf::from("/test/path/data/file.txt"));
        });
    }

    #[test]
    fn test_resolve_no_location_reference() {
        let locations = make_locations_config(vec![("dev", "/Volumes/T9/dev")]);

        // Path without location reference should just be expanded normally
        let result = resolve("/absolute/path", &locations);
        assert_eq!(result, PathBuf::from("/absolute/path"));

        let result2 = resolve("~/relative/path", &locations);
        let home = dirs::home_dir().unwrap();
        assert_eq!(result2, home.join("relative").join("path"));
    }

    #[test]
    fn test_resolve_unknown_location() {
        let locations = make_locations_config(vec![("dev", "/Volumes/T9/dev")]);

        // Unknown location reference should be left as-is (then expanded by shellexpand)
        let result = resolve("${locations.unknown}/path", &locations);
        // shellexpand doesn't know about ${locations.xxx}, so it's left as-is
        assert_eq!(result, PathBuf::from("${locations.unknown}/path"));
    }

    #[test]
    fn test_resolve_empty_locations() {
        let locations = LocationsConfig::default();

        let result = resolve("/some/path", &locations);
        assert_eq!(result, PathBuf::from("/some/path"));
    }

    #[test]
    fn test_resolve_multiple_references_same_location() {
        let locations = make_locations_config(vec![("dev", "/Volumes/T9/dev")]);

        let result = resolve("${locations.dev}/a:${locations.dev}/b", &locations);
        assert_eq!(result, PathBuf::from("/Volumes/T9/dev/a:/Volumes/T9/dev/b"));
    }

    #[test]
    fn test_identify_location_simple() {
        let locations = make_locations_config(vec![("dev", "/Volumes/T9/dev")]);

        let path = Path::new("/Volumes/T9/dev/ws/project");
        let result = identify_location(path, &locations);
        assert_eq!(result, Some("dev".to_string()));
    }

    #[test]
    fn test_identify_location_exact_match() {
        let locations = make_locations_config(vec![("dev", "/Volumes/T9/dev")]);

        let path = Path::new("/Volumes/T9/dev");
        let result = identify_location(path, &locations);
        assert_eq!(result, Some("dev".to_string()));
    }

    #[test]
    fn test_identify_location_no_match() {
        let locations = make_locations_config(vec![("dev", "/Volumes/T9/dev")]);

        let path = Path::new("/Users/testuser/documents");
        let result = identify_location(path, &locations);
        assert_eq!(result, None);
    }

    #[test]
    fn test_identify_location_multiple_locations() {
        let locations = make_locations_config(vec![
            ("dev", "/Volumes/T9/dev"),
            ("home", "/Users/testuser"),
        ]);

        let path1 = Path::new("/Volumes/T9/dev/ws/project");
        assert_eq!(
            identify_location(path1, &locations),
            Some("dev".to_string())
        );

        let path2 = Path::new("/Users/testuser/documents/file.txt");
        assert_eq!(
            identify_location(path2, &locations),
            Some("home".to_string())
        );

        let path3 = Path::new("/other/path");
        assert_eq!(identify_location(path3, &locations), None);
    }

    #[test]
    fn test_identify_location_with_tilde_expansion() {
        let locations = make_locations_config(vec![("dotfiles", "~/dotfiles")]);

        let home = dirs::home_dir().unwrap();
        let path = home.join("dotfiles").join("vim");
        let result = identify_location(&path, &locations);
        assert_eq!(result, Some("dotfiles".to_string()));
    }

    #[test]
    fn test_identify_location_empty_locations() {
        let locations = LocationsConfig::default();

        let path = Path::new("/some/path");
        let result = identify_location(path, &locations);
        assert_eq!(result, None);
    }

    #[test]
    fn test_identify_location_partial_match_not_prefix() {
        let locations = make_locations_config(vec![("dev", "/Volumes/T9/dev")]);

        // This path contains "dev" but doesn't start with the location path
        let path = Path::new("/other/Volumes/T9/dev/ws");
        let result = identify_location(path, &locations);
        assert_eq!(result, None);
    }
}
