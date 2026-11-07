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

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::schema::LocationsConfig;

static ENVIRONMENT: std::sync::OnceLock<bossa::AppEnvironment> = std::sync::OnceLock::new();

/// Capture once before command dispatch. Tests use explicit AppEnvironment values.
pub fn environment() -> &'static bossa::AppEnvironment {
    ENVIRONMENT.get_or_init(bossa::AppEnvironment::capture)
}

pub fn config_dir() -> Result<PathBuf> {
    environment().config_dir()
}

pub fn state_dir() -> Result<PathBuf> {
    environment().state_dir()
}

pub fn workspaces_dir() -> Result<PathBuf> {
    environment().workspaces_dir()
}

/// Get the legacy workspace-setup config directory path
///
/// This is kept for backwards compatibility with migration tools.
/// Always returns `~/.config/workspace-setup`.
pub fn legacy_config_dir() -> Result<PathBuf> {
    let home = environment().home_dir()?;
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
    environment().expand(path)
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
    resolve_with_environment(path, locations, environment())
}

fn resolve_with_environment(
    path: &str,
    locations: &LocationsConfig,
    environment: &bossa::AppEnvironment,
) -> PathBuf {
    // First expand ${locations.xxx} references
    let mut result = path.to_string();

    // Use simple string replacement to find ${locations.name} patterns
    // For each match, look up the location and substitute
    for (name, location_path) in &locations.paths {
        let pattern = format!("${{locations.{name}}}");
        if result.contains(&pattern) {
            // Recursively resolve the location path itself (it might reference other locations)
            let expanded_location = resolve_with_environment(location_path, locations, environment);
            result = result.replace(&pattern, &expanded_location.to_string_lossy());
        }
    }

    // Then expand ~ and env vars using the existing expand function
    environment.expand(&result)
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
        let environment =
            bossa::AppEnvironment::new(None, [("BOSSA_TEST_LOCATION".into(), "/test/path".into())]);
        let locations = make_locations_config(vec![("test", "$BOSSA_TEST_LOCATION/data")]);
        let result =
            resolve_with_environment("${locations.test}/file.txt", &locations, &environment);
        assert_eq!(result, PathBuf::from("/test/path/data/file.txt"));
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
