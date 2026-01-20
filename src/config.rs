#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::fs;
use std::path::PathBuf;

// ============================================================================
// Config Format Support
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Json,
    Toml,
}

impl ConfigFormat {
    /// Get file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            ConfigFormat::Json => "json",
            ConfigFormat::Toml => "toml",
        }
    }

    /// Parse content in this format
    pub fn parse<T: DeserializeOwned>(&self, content: &str) -> Result<T> {
        match self {
            ConfigFormat::Json => serde_json::from_str(content).context("Invalid JSON format"),
            ConfigFormat::Toml => toml::from_str(content).context("Invalid TOML format"),
        }
    }

    /// Serialize to this format
    pub fn serialize<T: Serialize>(&self, value: &T) -> Result<String> {
        match self {
            ConfigFormat::Json => {
                serde_json::to_string_pretty(value).context("Failed to serialize to JSON")
            }
            ConfigFormat::Toml => {
                toml::to_string_pretty(value).context("Failed to serialize to TOML")
            }
        }
    }
}

/// Find config file, preferring TOML over JSON if both exist
pub fn find_config_file(dir: &std::path::Path, base_name: &str) -> Option<(PathBuf, ConfigFormat)> {
    // Prefer TOML
    let toml_path = dir.join(format!("{}.toml", base_name));
    if toml_path.exists() {
        return Some((toml_path, ConfigFormat::Toml));
    }

    // Fall back to JSON
    let json_path = dir.join(format!("{}.json", base_name));
    if json_path.exists() {
        return Some((json_path, ConfigFormat::Json));
    }

    None
}

/// Load a config file, trying TOML first then JSON
pub fn load_config<T: DeserializeOwned>(
    dir: &std::path::Path,
    base_name: &str,
) -> Result<(T, ConfigFormat)> {
    let (path, format) = find_config_file(dir, base_name).with_context(|| {
        format!(
            "Config file not found: {}.toml or {}.json",
            base_name, base_name
        )
    })?;

    let content =
        fs::read_to_string(&path).with_context(|| format!("Could not read {}", path.display()))?;

    let config = format.parse(&content)?;
    Ok((config, format))
}

/// Save a config file in the specified format
pub fn save_config<T: Serialize>(
    dir: &std::path::Path,
    base_name: &str,
    config: &T,
    format: ConfigFormat,
) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.{}", base_name, format.extension()));
    let content = format.serialize(config)?;
    fs::write(&path, &content)?;
    Ok(path)
}

/// Get the bossa config directory path
pub fn config_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".config").join("bossa"))
}

/// Get the legacy workspace-setup config directory path
/// This is kept for backwards compatibility with migration tools
pub fn legacy_config_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".config").join("workspace-setup"))
}

/// Get the workspaces root directory
pub fn workspaces_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join("dev").join("ws"))
}

// ============================================================================
// Helpers
// ============================================================================

/// Extract repo name from a git URL
pub fn repo_name_from_url(url: &str) -> Option<String> {
    let url = url.trim_end_matches('/').trim_end_matches(".git");
    url.rsplit('/').next().map(|s| s.to_string())
}

/// Detect default branch for a remote URL
pub fn detect_default_branch(url: &str) -> String {
    // Try to get remote HEAD
    if let Ok(output) = std::process::Command::new("git")
        .args(["ls-remote", "--symref", url, "HEAD"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Parse: ref: refs/heads/main	HEAD
        for line in stdout.lines() {
            if line.starts_with("ref:") && line.contains("refs/heads/") {
                if let Some(branch) = line.split("refs/heads/").nth(1) {
                    if let Some(branch) = branch.split_whitespace().next() {
                        return branch.to_string();
                    }
                }
            }
        }
    }
    "main".to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_config_format_extensions() {
        assert_eq!(ConfigFormat::Json.extension(), "json");
        assert_eq!(ConfigFormat::Toml.extension(), "toml");
    }

    #[test]
    fn test_parse_valid_json() {
        let json = r#"{"name": "test", "count": 42}"#;
        let result: Result<HashMap<String, serde_json::Value>> = ConfigFormat::Json.parse(json);
        assert!(result.is_ok());
        let map = result.unwrap();
        assert_eq!(map.get("name").and_then(|v| v.as_str()), Some("test"));
    }

    #[test]
    fn test_parse_invalid_json() {
        let invalid_json = r#"{"name": "test", broken"#;
        let result: Result<HashMap<String, String>> = ConfigFormat::Json.parse(invalid_json);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("JSON"));
    }

    #[test]
    fn test_parse_valid_toml() {
        let toml = r#"
name = "test"
count = 42
"#;
        let result: Result<HashMap<String, toml::Value>> = ConfigFormat::Toml.parse(toml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_invalid_toml() {
        let invalid_toml = r#"
name = "test"
broken =
"#;
        let result: Result<HashMap<String, String>> = ConfigFormat::Toml.parse(invalid_toml);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("TOML"));
    }

    #[test]
    fn test_parse_malformed_toml_missing_quotes() {
        let bad_toml = r#"name = test"#;
        let result: Result<HashMap<String, String>> = ConfigFormat::Toml.parse(bad_toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_string() {
        let result: Result<HashMap<String, String>> = ConfigFormat::Json.parse("");
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize_json() {
        let mut map = HashMap::new();
        map.insert("key".to_string(), "value".to_string());
        let result = ConfigFormat::Json.serialize(&map);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("key"));
    }

    #[test]
    fn test_serialize_toml() {
        let mut map = HashMap::new();
        map.insert("key".to_string(), "value".to_string());
        let result = ConfigFormat::Toml.serialize(&map);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("key"));
    }

    #[test]
    fn test_repo_name_from_url_https() {
        assert_eq!(
            repo_name_from_url("https://github.com/user/repo.git"),
            Some("repo".to_string())
        );
        assert_eq!(
            repo_name_from_url("https://github.com/user/repo"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn test_repo_name_from_url_ssh() {
        assert_eq!(
            repo_name_from_url("git@github.com:user/repo.git"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn test_repo_name_from_url_trailing_slash() {
        assert_eq!(
            repo_name_from_url("https://github.com/user/repo/"),
            Some("repo".to_string())
        );
    }

    #[test]
    fn test_repo_name_from_url_empty() {
        assert_eq!(repo_name_from_url(""), Some("".to_string()));
    }

    #[test]
    fn test_repo_name_from_url_no_slashes() {
        assert_eq!(
            repo_name_from_url("invalid-url"),
            Some("invalid-url".to_string())
        );
    }

    #[test]
    fn test_repo_name_from_url_special_chars() {
        assert_eq!(
            repo_name_from_url("https://github.com/user/my-repo_123.git"),
            Some("my-repo_123".to_string())
        );
    }

    #[test]
    fn test_repo_name_from_url_unicode() {
        assert_eq!(
            repo_name_from_url("https://github.com/user/プロジェクト.git"),
            Some("プロジェクト".to_string())
        );
    }

    #[test]
    fn test_repo_name_from_url_very_long() {
        let long_name = "a".repeat(500);
        let url = format!("https://github.com/user/{}.git", long_name);
        assert_eq!(repo_name_from_url(&url), Some(long_name));
    }
}
