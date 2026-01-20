use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
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
            ConfigFormat::Json => {
                serde_json::from_str(content).context("Invalid JSON format")
            }
            ConfigFormat::Toml => {
                toml::from_str(content).context("Invalid TOML format")
            }
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
pub fn find_config_file(dir: &PathBuf, base_name: &str) -> Option<(PathBuf, ConfigFormat)> {
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
pub fn load_config<T: DeserializeOwned>(dir: &PathBuf, base_name: &str) -> Result<(T, ConfigFormat)> {
    let (path, format) = find_config_file(dir, base_name)
        .with_context(|| format!("Config file not found: {}.toml or {}.json", base_name, base_name))?;

    let content = fs::read_to_string(&path)
        .with_context(|| format!("Could not read {}", path.display()))?;

    let config = format.parse(&content)?;
    Ok((config, format))
}

/// Save a config file in the specified format
pub fn save_config<T: Serialize>(
    dir: &PathBuf,
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

/// Get the config directory path
pub fn config_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".config").join("workspace-setup"))
}

/// Get the workspaces root directory
pub fn workspaces_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join("dev").join("ws"))
}

// ============================================================================
// Refs Config
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct RefsConfig {
    pub root_directory: String,
    pub repositories: Vec<RefsRepo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RefsRepo {
    pub name: String,
    pub url: String,
    #[serde(default = "default_branch")]
    pub default_branch: String,
}

fn default_branch() -> String {
    "main".to_string()
}

impl RefsConfig {
    /// Load refs config (tries .toml first, then .json)
    pub fn load() -> Result<Self> {
        let dir = config_dir()?;
        let (config, _format) = load_config(&dir, "refs")?;
        Ok(config)
    }

    /// Load refs config and return the format it was loaded from
    pub fn load_with_format() -> Result<(Self, ConfigFormat)> {
        let dir = config_dir()?;
        load_config(&dir, "refs")
    }

    /// Save refs config (preserves format, defaults to TOML for new files)
    #[allow(dead_code)]
    pub fn save(&self) -> Result<()> {
        self.save_as(ConfigFormat::Toml)
    }

    /// Save refs config in specific format
    pub fn save_as(&self, format: ConfigFormat) -> Result<()> {
        let dir = config_dir()?;
        save_config(&dir, "refs", self, format)?;
        Ok(())
    }

    /// Get expanded root directory path
    pub fn root_path(&self) -> Result<PathBuf> {
        let expanded = shellexpand::tilde(&self.root_directory);
        Ok(PathBuf::from(expanded.as_ref()))
    }

    /// Find a repo by name
    pub fn find_repo(&self, name: &str) -> Option<&RefsRepo> {
        self.repositories.iter().find(|r| r.name == name)
    }

    /// Add a new repo
    pub fn add_repo(&mut self, repo: RefsRepo) {
        // Remove if exists (update)
        self.repositories.retain(|r| r.name != repo.name);
        self.repositories.push(repo);
        // Sort by name
        self.repositories.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Remove a repo by name
    pub fn remove_repo(&mut self, name: &str) -> bool {
        let len_before = self.repositories.len();
        self.repositories.retain(|r| r.name != name);
        self.repositories.len() < len_before
    }
}

// ============================================================================
// Workspaces Config
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspacesConfig {
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub bare_dir: Option<String>,
    #[serde(default)]
    pub worktrees: Vec<WorktreeConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorktreeConfig {
    pub branch: String,
    pub path: String,
}

impl WorkspacesConfig {
    /// Load workspaces config (tries .toml first, then .json)
    pub fn load() -> Result<Self> {
        let dir = config_dir()?;
        let (config, _format) = load_config(&dir, "workspaces")?;
        Ok(config)
    }

    /// Load workspaces config and return the format it was loaded from
    pub fn load_with_format() -> Result<(Self, ConfigFormat)> {
        let dir = config_dir()?;
        load_config(&dir, "workspaces")
    }

    /// Save workspaces config (defaults to TOML)
    #[allow(dead_code)]
    pub fn save(&self) -> Result<()> {
        self.save_as(ConfigFormat::Toml)
    }

    /// Save workspaces config in specific format
    pub fn save_as(&self, format: ConfigFormat) -> Result<()> {
        let dir = config_dir()?;
        save_config(&dir, "workspaces", self, format)?;
        Ok(())
    }
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
