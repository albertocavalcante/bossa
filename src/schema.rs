#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ============================================================================
// Main Config Schema
// ============================================================================

/// The unified bossa configuration structure
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BossaConfig {
    /// Collections of git repositories (e.g., reference repos)
    #[serde(default)]
    pub collections: HashMap<String, Collection>,

    /// Workspace configuration (bare repos with worktrees)
    #[serde(default)]
    pub workspaces: WorkspacesConfig,

    /// Storage volumes and symlink management
    #[serde(default)]
    pub storage: HashMap<String, Storage>,

    /// Nova bootstrap configuration
    #[serde(default)]
    pub nova: NovaConfig,

    /// Sudo allowlist configuration
    #[serde(default)]
    pub sudo: SudoConfig,

    /// macOS defaults configuration
    #[serde(default)]
    pub defaults: DefaultsConfig,

    /// Packages configuration (aggregates all package managers)
    #[serde(default)]
    pub packages: PackagesConfig,

    /// Symlinks configuration (replaces stow)
    #[serde(default)]
    pub symlinks: Option<SymlinksConfig>,

    /// Dock configuration
    #[serde(default)]
    pub dock: DockConfig,

    /// File handlers
    #[serde(default)]
    pub handlers: HandlersConfig,
}

impl BossaConfig {
    /// Load the unified bossa config from ~/.config/bossa/config.toml
    pub fn load() -> Result<Self> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        let config_path = home.join(".config").join("bossa").join("config.toml");

        if !config_path.exists() {
            // Return default config if file doesn't exist
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Could not read config file: {}", config_path.display()))?;

        toml::from_str(&content).context("Invalid TOML format in bossa config")
    }

    /// Save the config to ~/.config/bossa/config.toml
    pub fn save(&self) -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        let config_dir = home.join(".config").join("bossa");
        std::fs::create_dir_all(&config_dir)?;

        let config_path = config_dir.join("config.toml");
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        std::fs::write(&config_path, content)?;

        Ok(config_path)
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate collections
        for (name, collection) in &self.collections {
            collection
                .validate()
                .with_context(|| format!("Invalid collection '{}'", name))?;
        }

        // Validate workspaces
        self.workspaces.validate()?;

        // Validate storage
        for (name, storage) in &self.storage {
            storage
                .validate()
                .with_context(|| format!("Invalid storage '{}'", name))?;
        }

        // Validate nova
        self.nova.validate()?;

        Ok(())
    }

    /// Find a collection by name
    pub fn find_collection(&self, name: &str) -> Option<&Collection> {
        self.collections.get(name)
    }

    /// Find a collection by name (mutable)
    pub fn find_collection_mut(&mut self, name: &str) -> Option<&mut Collection> {
        self.collections.get_mut(name)
    }

    /// Find a workspace repo by name
    pub fn find_workspace_repo(&self, name: &str) -> Option<&WorkspaceRepo> {
        self.workspaces.repos.iter().find(|repo| repo.name == name)
    }

    /// Find storage by name
    pub fn find_storage(&self, name: &str) -> Option<&Storage> {
        self.storage.get(name)
    }
}

// ============================================================================
// Collection - Group of git repos in a folder
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Collection {
    /// Path to the collection directory
    pub path: String,

    /// Human-readable description
    #[serde(default)]
    pub description: String,

    /// Clone settings for repositories in this collection
    #[serde(default)]
    pub clone: CloneSettings,

    /// Optional reference to a storage volume
    #[serde(default)]
    pub storage: Option<String>,

    /// List of repositories in this collection
    #[serde(default)]
    pub repos: Vec<CollectionRepo>,
}

impl Collection {
    /// Get the expanded path
    pub fn expanded_path(&self) -> Result<PathBuf> {
        let expanded = shellexpand::tilde(&self.path);
        Ok(PathBuf::from(expanded.as_ref()))
    }

    /// Validate the collection
    pub fn validate(&self) -> Result<()> {
        if self.path.is_empty() {
            anyhow::bail!("Collection path cannot be empty");
        }

        for repo in &self.repos {
            repo.validate()?;
        }

        Ok(())
    }

    /// Find a repo by name
    pub fn find_repo(&self, name: &str) -> Option<&CollectionRepo> {
        self.repos.iter().find(|r| r.name == name)
    }

    /// Add or update a repo
    pub fn add_repo(&mut self, repo: CollectionRepo) {
        // Remove if exists (update)
        self.repos.retain(|r| r.name != repo.name);
        self.repos.push(repo);
        // Sort by name
        self.repos.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Remove a repo by name
    pub fn remove_repo(&mut self, name: &str) -> bool {
        let len_before = self.repos.len();
        self.repos.retain(|r| r.name != name);
        self.repos.len() < len_before
    }
}

/// Settings for cloning repositories
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CloneSettings {
    /// Shallow clone depth (0 for full clone)
    #[serde(default)]
    pub depth: u32,

    /// Clone only a single branch
    #[serde(default)]
    pub single_branch: bool,

    /// Additional git clone options
    #[serde(default)]
    pub options: Vec<String>,
}

/// A repository in a collection
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CollectionRepo {
    /// Repository name
    pub name: String,

    /// Git URL
    pub url: String,

    /// Default branch (e.g., "main", "master")
    #[serde(default = "default_branch")]
    pub default_branch: String,

    /// Optional description
    #[serde(default)]
    pub description: String,
}

impl CollectionRepo {
    /// Validate the repo
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("Repository name cannot be empty");
        }
        if self.name.contains("..") || self.name.contains('/') || self.name.contains('\\') {
            anyhow::bail!("Repository name cannot contain path separators or '..'");
        }
        if self.url.is_empty() {
            anyhow::bail!("Repository URL cannot be empty");
        }
        Ok(())
    }
}

fn default_branch() -> String {
    "main".to_string()
}

// ============================================================================
// Workspaces - Bare repo + worktree structure
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct WorkspacesConfig {
    /// Root directory for all workspaces
    #[serde(default = "default_workspaces_root")]
    pub root: String,

    /// Structure type (e.g., "bare-worktree")
    #[serde(default = "default_structure")]
    pub structure: String,

    /// List of workspace repositories
    #[serde(default)]
    pub repos: Vec<WorkspaceRepo>,
}

impl WorkspacesConfig {
    /// Get the expanded root path
    pub fn expanded_root(&self) -> Result<PathBuf> {
        let expanded = shellexpand::tilde(&self.root);
        Ok(PathBuf::from(expanded.as_ref()))
    }

    /// Validate the workspaces config
    pub fn validate(&self) -> Result<()> {
        if self.root.is_empty() {
            anyhow::bail!("Workspaces root cannot be empty");
        }

        for repo in &self.repos {
            repo.validate()?;
        }

        Ok(())
    }

    /// Find a repo by name
    pub fn find_repo(&self, name: &str) -> Option<&WorkspaceRepo> {
        self.repos.iter().find(|r| r.name == name)
    }

    /// Find a repo by name (mutable)
    pub fn find_repo_mut(&mut self, name: &str) -> Option<&mut WorkspaceRepo> {
        self.repos.iter_mut().find(|r| r.name == name)
    }

    /// Add or update a repo
    pub fn add_repo(&mut self, repo: WorkspaceRepo) {
        // Remove if exists (update)
        self.repos.retain(|r| r.name != repo.name);
        self.repos.push(repo);
        // Sort by category, then name
        self.repos.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| a.name.cmp(&b.name))
        });
    }

    /// Remove a repo by name
    pub fn remove_repo(&mut self, name: &str) -> bool {
        let len_before = self.repos.len();
        self.repos.retain(|r| r.name != name);
        self.repos.len() < len_before
    }

    /// Get all unique categories
    pub fn categories(&self) -> Vec<String> {
        let mut categories: Vec<String> = self
            .repos
            .iter()
            .map(|r| r.category.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        categories.sort();
        categories
    }

    /// Get repos by category
    pub fn repos_by_category(&self, category: &str) -> Vec<&WorkspaceRepo> {
        self.repos
            .iter()
            .filter(|r| r.category == category)
            .collect()
    }
}

fn default_workspaces_root() -> String {
    "~/dev/ws".to_string()
}

fn default_structure() -> String {
    "bare-worktree".to_string()
}

/// A workspace repository with worktrees
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkspaceRepo {
    /// Repository name
    pub name: String,

    /// Git URL
    pub url: String,

    /// Category (e.g., "utils", "projects", "work")
    #[serde(default = "default_category")]
    pub category: String,

    /// List of worktree branch names
    #[serde(default)]
    pub worktrees: Vec<String>,

    /// Optional description
    #[serde(default)]
    pub description: String,
}

impl WorkspaceRepo {
    /// Validate the repo
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("Repository name cannot be empty");
        }
        if self.name.contains("..") || self.name.contains('/') || self.name.contains('\\') {
            anyhow::bail!("Repository name cannot contain path separators or '..'");
        }
        if self.url.is_empty() {
            anyhow::bail!("Repository URL cannot be empty");
        }
        if self.category.trim().is_empty() {
            anyhow::bail!("Repository category cannot be empty");
        }
        Ok(())
    }

    /// Get the bare repository path
    pub fn bare_path(&self, root: &std::path::Path) -> PathBuf {
        root.join(&self.category).join(format!("{}.git", self.name))
    }

    /// Get the worktree path for a specific branch
    pub fn worktree_path(&self, root: &std::path::Path, branch: &str) -> PathBuf {
        root.join(&self.category).join(&self.name).join(branch)
    }
}

fn default_category() -> String {
    "default".to_string()
}

// ============================================================================
// Storage - External volumes with symlinks
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Storage {
    /// Mount point path
    pub mount: String,

    /// Storage type (external, network, etc.)
    #[serde(rename = "type")]
    pub storage_type: StorageType,

    /// List of symlinks to create
    #[serde(default)]
    pub symlinks: Vec<Symlink>,

    /// Optional description
    #[serde(default)]
    pub description: String,
}

impl Storage {
    /// Get the expanded mount path
    pub fn expanded_mount(&self) -> Result<PathBuf> {
        let expanded = shellexpand::tilde(&self.mount);
        Ok(PathBuf::from(expanded.as_ref()))
    }

    /// Check if the storage is mounted
    pub fn is_mounted(&self) -> bool {
        self.expanded_mount().map(|p| p.exists()).unwrap_or(false)
    }

    /// Validate the storage config
    pub fn validate(&self) -> Result<()> {
        if self.mount.is_empty() {
            anyhow::bail!("Storage mount point cannot be empty");
        }

        for symlink in &self.symlinks {
            symlink.validate()?;
        }

        Ok(())
    }

    /// Find a symlink by from path
    pub fn find_symlink(&self, from: &str) -> Option<&Symlink> {
        self.symlinks.iter().find(|s| s.from == from)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StorageType {
    External,
    Network,
    Internal,
}

/// A symlink configuration
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Symlink {
    /// Source path (the symlink location)
    pub from: String,

    /// Target path (what the symlink points to)
    /// Can contain {mount} placeholder
    pub to: String,
}

impl Symlink {
    /// Get the expanded from path
    pub fn expanded_from(&self) -> Result<PathBuf> {
        let expanded = shellexpand::tilde(&self.from);
        Ok(PathBuf::from(expanded.as_ref()))
    }

    /// Get the expanded to path, replacing {mount} placeholder
    pub fn expanded_to(&self, mount_point: &str) -> Result<PathBuf> {
        let replaced = self.to.replace("{mount}", mount_point);
        let expanded = shellexpand::tilde(&replaced);
        Ok(PathBuf::from(expanded.as_ref()))
    }

    /// Validate the symlink
    pub fn validate(&self) -> Result<()> {
        if self.from.is_empty() {
            anyhow::bail!("Symlink 'from' path cannot be empty");
        }
        if self.to.is_empty() {
            anyhow::bail!("Symlink 'to' path cannot be empty");
        }
        Ok(())
    }
}

// ============================================================================
// Nova - Bootstrap stages for new machine setup
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct NovaConfig {
    /// Ordered list of bootstrap stages to run
    #[serde(default)]
    pub stages: Vec<String>,

    /// Optional custom stage configurations
    #[serde(default)]
    pub stage_config: HashMap<String, StageConfig>,
}

impl NovaConfig {
    /// Validate the nova config
    pub fn validate(&self) -> Result<()> {
        // Check for duplicate stages
        let mut seen = std::collections::HashSet::new();
        for stage in &self.stages {
            if !seen.insert(stage) {
                anyhow::bail!("Duplicate stage: {}", stage);
            }
        }

        Ok(())
    }

    /// Check if a stage is enabled
    pub fn has_stage(&self, stage: &str) -> bool {
        self.stages.contains(&stage.to_string())
    }

    /// Get stage configuration
    pub fn get_stage_config(&self, stage: &str) -> Option<&StageConfig> {
        self.stage_config.get(stage)
    }
}

/// Configuration for a specific bootstrap stage
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StageConfig {
    /// Whether the stage is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Stage-specific options
    #[serde(flatten)]
    pub options: HashMap<String, toml::Value>,
}

fn default_true() -> bool {
    true
}

// ============================================================================
// Sudo - Allowlist configuration for elevated operations
// ============================================================================

/// Sudo allowlist configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SudoConfig {
    #[serde(default)]
    pub casks: Vec<String>,
    #[serde(default)]
    pub defaults: Vec<String>,
    #[serde(default)]
    pub operations: Vec<String>,
}

// ============================================================================
// Defaults - macOS defaults configuration
// ============================================================================

/// macOS defaults configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefaultsConfig {
    /// Key-value pairs for defaults (domain.key = value)
    #[serde(flatten)]
    pub settings: HashMap<String, DefaultValue>,

    /// Services to restart after applying defaults
    #[serde(default)]
    pub restart: DefaultsRestartConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefaultsRestartConfig {
    #[serde(default)]
    pub services: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DefaultValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<DefaultValue>),
}

impl DefaultValue {
    /// Convert to defaults command argument format
    pub fn to_defaults_args(&self) -> Vec<String> {
        match self {
            DefaultValue::Bool(b) => vec!["-bool".to_string(), b.to_string()],
            DefaultValue::Int(i) => vec!["-int".to_string(), i.to_string()],
            DefaultValue::Float(f) => vec!["-float".to_string(), f.to_string()],
            DefaultValue::String(s) => vec!["-string".to_string(), s.clone()],
            DefaultValue::Array(arr) => {
                let mut args = vec!["-array".to_string()];
                for item in arr {
                    match item {
                        DefaultValue::String(s) => args.push(s.clone()),
                        _ => args.push(format!("{:?}", item)),
                    }
                }
                args
            }
        }
    }
}

// ============================================================================
// Packages - Package managers configuration
// ============================================================================

/// Packages configuration (aggregates all package managers)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackagesConfig {
    #[serde(default)]
    pub brew: BrewConfig,
    #[serde(default)]
    pub pnpm: PnpmConfig,
    #[serde(default)]
    pub gh: GhConfig,
    #[serde(default)]
    pub vscode: VscodeConfig,
}

/// Brew packages configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrewConfig {
    #[serde(default)]
    pub taps: Vec<String>,
    #[serde(default)]
    pub formulas: Vec<String>,
    #[serde(default)]
    pub casks: Vec<String>,
    #[serde(default)]
    pub fonts: Vec<String>,
    #[serde(default)]
    pub essential: BrewEssentialConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrewEssentialConfig {
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default = "default_retries")]
    pub retries: usize,
}

fn default_retries() -> usize {
    5
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PnpmConfig {
    #[serde(default)]
    pub globals: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GhConfig {
    #[serde(default)]
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VscodeConfig {
    #[serde(default)]
    pub extensions: Vec<String>,
}

// ============================================================================
// Symlinks - Dotfile symlinks configuration (replaces stow)
// ============================================================================

/// Symlinks configuration (replaces stow)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymlinksConfig {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub ignore: Vec<String>,
}

// ============================================================================
// Dock Configuration
// ============================================================================

/// Dock configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DockConfig {
    /// Auto-hide the dock
    #[serde(default)]
    pub autohide: bool,

    /// Tile size in pixels
    #[serde(default = "default_tilesize")]
    pub tilesize: u32,

    /// Minimize effect ("scale" or "genie")
    #[serde(default = "default_minimize_effect")]
    pub minimize_effect: String,

    /// Show recent applications
    #[serde(default)]
    pub show_recents: bool,

    /// Applications to pin to dock (in order)
    #[serde(default)]
    pub apps: Vec<String>,

    /// Folders to add to dock
    #[serde(default)]
    pub folders: Vec<DockFolder>,
}

fn default_tilesize() -> u32 {
    64
}

fn default_minimize_effect() -> String {
    "scale".to_string()
}

/// Dock folder configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockFolder {
    /// Path to folder
    pub path: String,

    /// View type ("grid", "list", "fan", "auto")
    #[serde(default = "default_dock_view")]
    pub view: String,

    /// Display type ("folder", "stack")
    #[serde(default = "default_dock_display")]
    pub display: String,

    /// Sort by ("name", "dateadded", "datemodified", "datecreated", "kind")
    #[serde(default = "default_dock_sort")]
    pub sort: String,
}

fn default_dock_view() -> String {
    "grid".to_string()
}

fn default_dock_display() -> String {
    "stack".to_string()
}

fn default_dock_sort() -> String {
    "dateadded".to_string()
}

// ============================================================================
// File Handlers Configuration
// ============================================================================

/// File handlers configuration (duti)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandlersConfig {
    /// Handlers keyed by bundle ID, value is list of UTIs/extensions
    #[serde(flatten)]
    pub handlers: HashMap<String, Vec<String>>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_example_config() {
        let toml = r#"
[collections.refs]
path = "~/dev/refs"
description = "Reference repositories"

[collections.refs.clone]
depth = 1
single_branch = true

[[collections.refs.repos]]
name = "rust"
url = "https://github.com/rust-lang/rust"

[workspaces]
root = "~/dev/ws"
structure = "bare-worktree"

[[workspaces.repos]]
name = "dotfiles"
url = "git@github.com:user/dotfiles.git"
category = "utils"
worktrees = ["main"]

[storage.t9]
mount = "/Volumes/T9"
type = "external"

[[storage.t9.symlinks]]
from = "~/dev/refs"
to = "{mount}/refs"

[nova]
stages = ["defaults", "homebrew", "essential", "stow", "collections", "workspaces"]
"#;

        let config: BossaConfig = toml::from_str(toml).expect("Failed to parse config");

        // Test collections
        assert!(config.collections.contains_key("refs"));
        let refs = &config.collections["refs"];
        assert_eq!(refs.path, "~/dev/refs");
        assert_eq!(refs.clone.depth, 1);
        assert!(refs.clone.single_branch);
        assert_eq!(refs.repos.len(), 1);
        assert_eq!(refs.repos[0].name, "rust");

        // Test workspaces
        assert_eq!(config.workspaces.root, "~/dev/ws");
        assert_eq!(config.workspaces.structure, "bare-worktree");
        assert_eq!(config.workspaces.repos.len(), 1);
        assert_eq!(config.workspaces.repos[0].name, "dotfiles");
        assert_eq!(config.workspaces.repos[0].category, "utils");

        // Test storage
        assert!(config.storage.contains_key("t9"));
        let t9 = &config.storage["t9"];
        assert_eq!(t9.mount, "/Volumes/T9");
        assert_eq!(t9.storage_type, StorageType::External);
        assert_eq!(t9.symlinks.len(), 1);

        // Test nova
        assert_eq!(config.nova.stages.len(), 6);
        assert!(config.nova.has_stage("homebrew"));
    }

    #[test]
    fn test_collection_validation() {
        let mut collection = Collection {
            path: "~/test".to_string(),
            description: "Test".to_string(),
            clone: CloneSettings::default(),
            storage: None,
            repos: vec![],
        };

        assert!(collection.validate().is_ok());

        // Empty path should fail
        collection.path = "".to_string();
        assert!(collection.validate().is_err());
    }

    #[test]
    fn test_workspace_repo_paths() {
        let repo = WorkspaceRepo {
            name: "test-repo".to_string(),
            url: "git@github.com:user/test-repo.git".to_string(),
            category: "projects".to_string(),
            worktrees: vec!["main".to_string()],
            description: "".to_string(),
        };

        let root = PathBuf::from("/home/user/ws");
        let bare_path = repo.bare_path(&root);
        assert_eq!(
            bare_path,
            PathBuf::from("/home/user/ws/projects/test-repo.git")
        );

        let worktree_path = repo.worktree_path(&root, "main");
        assert_eq!(
            worktree_path,
            PathBuf::from("/home/user/ws/projects/test-repo/main")
        );
    }

    #[test]
    fn test_symlink_placeholder() {
        let symlink = Symlink {
            from: "~/dev/refs".to_string(),
            to: "{mount}/refs".to_string(),
        };

        let expanded = symlink.expanded_to("/Volumes/T9").unwrap();
        assert_eq!(expanded, PathBuf::from("/Volumes/T9/refs"));
    }

    // ====================================================================
    // Adversarial Tests - Edge Cases and Invalid Inputs
    // ====================================================================

    #[test]
    fn test_parse_empty_config() {
        let toml = "";
        let config: BossaConfig = toml::from_str(toml).unwrap();
        assert!(config.collections.is_empty());
        assert!(config.workspaces.repos.is_empty());
        assert!(config.storage.is_empty());
    }

    #[test]
    fn test_parse_config_with_unknown_fields() {
        let toml = r#"
[collections.refs]
path = "~/dev/refs"
unknown_field = "should_be_ignored"

[[collections.refs.repos]]
name = "rust"
url = "https://github.com/rust-lang/rust"
extra_field = 123
"#;
        // Serde should ignore unknown fields by default
        let result: Result<BossaConfig, _> = toml::from_str(toml);
        assert!(result.is_ok());
    }

    #[test]
    fn test_collection_empty_name() {
        let repo = CollectionRepo {
            name: "".to_string(),
            url: "https://github.com/user/repo".to_string(),
            default_branch: "main".to_string(),
            description: "".to_string(),
        };
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_collection_empty_url() {
        let repo = CollectionRepo {
            name: "test".to_string(),
            url: "".to_string(),
            default_branch: "main".to_string(),
            description: "".to_string(),
        };
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_collection_whitespace_only_name() {
        let repo = CollectionRepo {
            name: "   ".to_string(),
            url: "https://github.com/user/repo".to_string(),
            default_branch: "main".to_string(),
            description: "".to_string(),
        };
        // Fixed: whitespace-only names should now fail validation
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_workspace_empty_category() {
        let repo = WorkspaceRepo {
            name: "test".to_string(),
            url: "https://github.com/user/test".to_string(),
            category: "".to_string(),
            worktrees: vec![],
            description: "".to_string(),
        };
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_workspace_special_chars_in_name() {
        let repo = WorkspaceRepo {
            name: "test/../../../etc/passwd".to_string(),
            url: "https://github.com/user/test".to_string(),
            category: "projects".to_string(),
            worktrees: vec![],
            description: "".to_string(),
        };
        // Fixed: path traversal attempts should now fail validation
        assert!(repo.validate().is_err());
    }

    #[test]
    fn test_symlink_empty_from() {
        let symlink = Symlink {
            from: "".to_string(),
            to: "/somewhere".to_string(),
        };
        assert!(symlink.validate().is_err());
    }

    #[test]
    fn test_symlink_empty_to() {
        let symlink = Symlink {
            from: "~/test".to_string(),
            to: "".to_string(),
        };
        assert!(symlink.validate().is_err());
    }

    #[test]
    fn test_symlink_multiple_placeholders() {
        let symlink = Symlink {
            from: "~/dev/refs".to_string(),
            to: "{mount}/{mount}/refs".to_string(),
        };
        let expanded = symlink.expanded_to("/Volumes/T9").unwrap();
        assert_eq!(expanded, PathBuf::from("/Volumes/T9//Volumes/T9/refs"));
    }

    #[test]
    fn test_storage_empty_mount() {
        let storage = Storage {
            mount: "".to_string(),
            storage_type: StorageType::External,
            symlinks: vec![],
            description: "".to_string(),
        };
        assert!(storage.validate().is_err());
    }

    #[test]
    fn test_nova_duplicate_stages() {
        let config = NovaConfig {
            stages: vec![
                "defaults".to_string(),
                "homebrew".to_string(),
                "defaults".to_string(), // duplicate
            ],
            stage_config: HashMap::new(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_collection_add_duplicate_repo() {
        let mut collection = Collection {
            path: "~/test".to_string(),
            description: "Test".to_string(),
            clone: CloneSettings::default(),
            storage: None,
            repos: vec![],
        };

        let repo1 = CollectionRepo {
            name: "test-repo".to_string(),
            url: "https://github.com/user/test1".to_string(),
            default_branch: "main".to_string(),
            description: "".to_string(),
        };

        let repo2 = CollectionRepo {
            name: "test-repo".to_string(),                    // Same name
            url: "https://github.com/user/test2".to_string(), // Different URL
            default_branch: "main".to_string(),
            description: "".to_string(),
        };

        collection.add_repo(repo1);
        assert_eq!(collection.repos.len(), 1);

        collection.add_repo(repo2); // Should replace
        assert_eq!(collection.repos.len(), 1);
        assert_eq!(collection.repos[0].url, "https://github.com/user/test2");
    }

    #[test]
    fn test_workspace_extremely_long_name() {
        let long_name = "a".repeat(10000);
        let repo = WorkspaceRepo {
            name: long_name.clone(),
            url: "https://github.com/user/test".to_string(),
            category: "projects".to_string(),
            worktrees: vec![],
            description: "".to_string(),
        };
        assert!(repo.validate().is_ok());

        let root = PathBuf::from("/home/user/ws");
        let path = repo.bare_path(&root);
        // This could cause filesystem issues
        assert!(path.to_string_lossy().len() > 10000);
    }

    #[test]
    fn test_default_value_to_args() {
        let bool_val = DefaultValue::Bool(true);
        assert_eq!(bool_val.to_defaults_args(), vec!["-bool", "true"]);

        let int_val = DefaultValue::Int(42);
        assert_eq!(int_val.to_defaults_args(), vec!["-int", "42"]);

        let float_val = DefaultValue::Float(2.5);
        assert_eq!(float_val.to_defaults_args(), vec!["-float", "2.5"]);

        let string_val = DefaultValue::String("test".to_string());
        assert_eq!(string_val.to_defaults_args(), vec!["-string", "test"]);

        let array_val = DefaultValue::Array(vec![
            DefaultValue::String("a".to_string()),
            DefaultValue::String("b".to_string()),
        ]);
        let args = array_val.to_defaults_args();
        assert_eq!(args[0], "-array");
        assert_eq!(args[1], "a");
        assert_eq!(args[2], "b");
    }

    #[test]
    fn test_parse_config_missing_required_fields() {
        let toml = r#"
[[collections.refs.repos]]
name = "rust"
# Missing 'url' field
"#;
        let result: Result<BossaConfig, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_workspaces_categories() {
        let workspaces = WorkspacesConfig {
            repos: vec![
                WorkspaceRepo {
                    name: "repo1".to_string(),
                    url: "url1".to_string(),
                    category: "utils".to_string(),
                    worktrees: vec![],
                    description: "".to_string(),
                },
                WorkspaceRepo {
                    name: "repo2".to_string(),
                    url: "url2".to_string(),
                    category: "projects".to_string(),
                    worktrees: vec![],
                    description: "".to_string(),
                },
                WorkspaceRepo {
                    name: "repo3".to_string(),
                    url: "url3".to_string(),
                    category: "utils".to_string(),
                    worktrees: vec![],
                    description: "".to_string(),
                },
            ],
            ..Default::default()
        };

        let categories = workspaces.categories();
        assert_eq!(categories.len(), 2);
        assert!(categories.contains(&"utils".to_string()));
        assert!(categories.contains(&"projects".to_string()));
    }

    #[test]
    fn test_unicode_in_paths() {
        let collection = Collection {
            path: "~/dev/日本語/テスト".to_string(),
            description: "Test".to_string(),
            clone: CloneSettings::default(),
            storage: None,
            repos: vec![],
        };
        assert!(collection.validate().is_ok());
    }

    #[test]
    fn test_collection_repo_path_traversal_variants() {
        // Test various path traversal attempts
        let traversal_attempts = vec![
            "test/../etc",
            "../etc/passwd",
            "test/../../etc",
            "test\\etc",
            "test/subdir",
        ];

        for attempt in traversal_attempts {
            let repo = CollectionRepo {
                name: attempt.to_string(),
                url: "https://github.com/user/repo".to_string(),
                default_branch: "main".to_string(),
                description: "".to_string(),
            };
            assert!(
                repo.validate().is_err(),
                "Path traversal should be rejected: {}",
                attempt
            );
        }
    }

    #[test]
    fn test_workspace_repo_path_traversal_variants() {
        // Test various path traversal attempts
        let traversal_attempts = vec![
            "test/../etc",
            "../etc/passwd",
            "test/../../etc",
            "test\\etc",
            "test/subdir",
        ];

        for attempt in traversal_attempts {
            let repo = WorkspaceRepo {
                name: attempt.to_string(),
                url: "https://github.com/user/test".to_string(),
                category: "projects".to_string(),
                worktrees: vec![],
                description: "".to_string(),
            };
            assert!(
                repo.validate().is_err(),
                "Path traversal should be rejected: {}",
                attempt
            );
        }
    }

    #[test]
    fn test_workspace_whitespace_only_category() {
        let repo = WorkspaceRepo {
            name: "test".to_string(),
            url: "https://github.com/user/test".to_string(),
            category: "   ".to_string(),
            worktrees: vec![],
            description: "".to_string(),
        };
        // Fixed: whitespace-only categories should now fail validation
        assert!(repo.validate().is_err());
    }
}
