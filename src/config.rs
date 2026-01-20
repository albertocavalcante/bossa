use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Get the config directory path
pub fn config_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".config").join("workspace-setup"))
}

/// Get the refs root directory
pub fn refs_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join("dev").join("refs"))
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
    /// Load refs.json config
    pub fn load() -> Result<Self> {
        let path = config_dir()?.join("refs.json");
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Could not read {}", path.display()))?;
        serde_json::from_str(&content).context("Invalid refs.json format")
    }

    /// Save refs.json config
    pub fn save(&self) -> Result<()> {
        let dir = config_dir()?;
        fs::create_dir_all(&dir)?;
        let path = dir.join("refs.json");
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content)?;
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
    /// Load workspaces.json config
    pub fn load() -> Result<Self> {
        let path = config_dir()?.join("workspaces.json");
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Could not read {}", path.display()))?;
        serde_json::from_str(&content).context("Invalid workspaces.json format")
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
