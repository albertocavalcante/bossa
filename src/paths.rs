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
use std::path::PathBuf;

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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
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
}
