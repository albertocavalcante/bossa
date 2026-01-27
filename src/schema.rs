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

    /// Tools configuration (declarative tool definitions)
    #[serde(default)]
    pub tools: ToolsSection,
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
// Tools Section (Declarative Definitions)
// ============================================================================

/// Declarative tools configuration
///
/// Example config:
/// ```toml
/// [tools]
/// install_dir = "~/.local/bin"
///
/// [tools.rg]
/// source = "http"
/// description = "ripgrep - fast grep alternative"
/// url = "https://github.com/BurntSushi/ripgrep/releases/download/{version}/ripgrep-{version}-x86_64-apple-darwin.tar.gz"
/// version = "14.1.0"
/// binary = "rg"
/// archive_path = "ripgrep-{version}-x86_64-apple-darwin"
///
/// [tools.jq]
/// source = "container"
/// description = "jq - JSON processor"
/// image = "fedora:latest"
/// package = "jq"
/// binary_path = "/usr/bin/jq"
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ToolsSection {
    /// Default installation directory (defaults to ~/.local/bin)
    #[serde(default = "default_install_dir")]
    pub install_dir: String,

    /// Default container runtime (podman or docker)
    #[serde(default = "default_runtime")]
    pub runtime: String,

    /// Tool definitions (keyed by tool name)
    #[serde(flatten)]
    pub definitions: HashMap<String, ToolDefinition>,
}

fn default_install_dir() -> String {
    "~/.local/bin".to_string()
}

/// A declarative tool definition
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolDefinition {
    /// Source type: http, container, github-release
    pub source: ToolSource,

    /// Human-readable description
    #[serde(default)]
    pub description: String,

    // === HTTP source fields ===
    /// Download URL (supports {version}, {platform} placeholders)
    /// If not provided, will be built from base_url + path + url_pattern
    #[serde(default)]
    pub url: Option<String>,

    /// Base URL for artifact repositories (e.g., "https://releases.example.com/artifacts")
    #[serde(default)]
    pub base_url: Option<String>,

    /// Path within the artifact repository (e.g., "org/mytool")
    #[serde(default)]
    pub path: Option<String>,

    /// Custom URL pattern with placeholders: {base_url}, {path}, {version}, {platform}, {binary}, {archive_name}, {ext}
    /// Default: "{base_url}/{path}/{version}/{archive_name}-{version}-{platform}.{ext}"
    #[serde(default)]
    pub url_pattern: Option<String>,

    /// Archive name if different from binary name (e.g., "mytool-cli" when binary is "mytool")
    #[serde(default)]
    pub archive_name: Option<String>,

    /// Platform string style: "long" (linux-amd64), "short" (linux_amd64), "go" (linux_amd64), or custom pattern
    #[serde(default)]
    pub platform_style: Option<String>,

    /// Path inside archive where binary is located (supports {version}, {platform})
    #[serde(default)]
    pub archive_path: Option<String>,

    // === GitHub Release source fields ===
    /// GitHub repository (e.g., "BurntSushi/ripgrep")
    #[serde(default)]
    pub repo: Option<String>,

    /// Asset name pattern (supports {version})
    #[serde(default)]
    pub asset: Option<String>,

    // === Container source fields ===
    /// Container image (e.g., "fedora:latest", "ubi9/ubi-minimal")
    #[serde(default)]
    pub image: Option<String>,

    /// Package to install in container
    #[serde(default)]
    pub package: Option<String>,

    /// Additional packages/dependencies to install
    #[serde(default)]
    pub packages: Vec<String>,

    /// Package manager override (dnf, microdnf, apt, apk, yum)
    #[serde(default)]
    pub package_manager: Option<String>,

    /// Path to binary inside container (e.g., /usr/bin/jq)
    #[serde(default)]
    pub binary_path: Option<String>,

    /// Command to run before package install (e.g., enable repos)
    #[serde(default)]
    pub pre_install: Option<String>,

    // === Cargo source fields ===
    /// Crate name on crates.io (e.g., "ripgrep", "fd-find")
    #[serde(default, rename = "crate")]
    pub crate_name: Option<String>,

    /// Git URL for cargo install --git (alternative to crate)
    #[serde(default)]
    pub git: Option<String>,

    /// Git branch to use with --git
    #[serde(default)]
    pub branch: Option<String>,

    /// Git tag to use with --git
    #[serde(default)]
    pub tag: Option<String>,

    /// Git revision to use with --git
    #[serde(default)]
    pub rev: Option<String>,

    /// Cargo features to enable (comma-separated or list)
    #[serde(default)]
    pub features: Vec<String>,

    /// Enable all features
    #[serde(default)]
    pub all_features: bool,

    /// Use --locked flag for reproducible builds
    #[serde(default)]
    pub locked: bool,

    /// Additional cargo install arguments
    #[serde(default)]
    pub cargo_args: Vec<String>,

    // === Common fields ===
    /// Version to install (supports "latest" for github-release)
    #[serde(default)]
    pub version: Option<String>,

    /// Binary name (defaults to tool name)
    #[serde(default)]
    pub binary: Option<String>,

    /// Custom installation directory (overrides global)
    #[serde(default)]
    pub install_dir: Option<String>,

    /// Container runtime override (podman or docker)
    #[serde(default)]
    pub runtime: Option<String>,

    /// Archive type hint: tar.gz, zip, binary (auto-detected if not set)
    #[serde(default)]
    pub archive_type: Option<String>,

    /// Post-install message or script to display
    #[serde(default)]
    pub post_install: Option<String>,

    /// Whether this tool is enabled (default: true)
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Tool source type
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ToolSource {
    /// HTTP URL to tar.gz archive
    Http,
    /// Container image (podman/docker)
    Container,
    /// GitHub release
    GithubRelease,
    /// Cargo install (from crates.io or git)
    Cargo,
}

impl ToolDefinition {
    /// Expand {version} placeholders in a string
    pub fn expand_version(&self, template: &str) -> String {
        match &self.version {
            Some(v) => template.replace("{version}", v),
            None => template.to_string(),
        }
    }

    /// Expand all placeholders in a template string
    /// Supports: {base_url}, {path}, {version}, {platform}, {binary}, {archive_name}, {ext}
    pub fn expand_template(&self, template: &str, tool_name: &str) -> String {
        let binary = self.effective_binary(tool_name);
        let archive_name = self.archive_name.as_deref().unwrap_or(&binary);
        let ext = self.effective_extension();
        let platform = self.get_platform_string();

        let mut result = template.to_string();
        if let Some(ref base_url) = self.base_url {
            result = result.replace("{base_url}", base_url.trim_end_matches('/'));
        }
        if let Some(ref path) = self.path {
            result = result.replace("{path}", path.trim_matches('/'));
        }
        if let Some(ref version) = self.version {
            result = result.replace("{version}", version);
        }
        result = result.replace("{platform}", &platform);
        result = result.replace("{binary}", &binary);
        result = result.replace("{archive_name}", archive_name);
        result = result.replace("{ext}", &ext);
        result
    }

    /// Get the platform string based on platform_style
    /// Returns format like "linux-amd64", "darwin-arm64", etc.
    pub fn get_platform_string(&self) -> String {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        // Normalize OS names
        let os_name = match os {
            "macos" => "darwin",
            other => other,
        };

        // Normalize arch names (Rust uses different names than Go)
        let arch_name = match arch {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            "x86" => "386",
            other => other,
        };

        let style = self.platform_style.as_deref().unwrap_or("long");

        match style {
            "long" => format!("{}-{}", os_name, arch_name),
            "short" | "go" => format!("{}_{}", os_name, arch_name),
            "os-only" => os_name.to_string(),
            "arch-only" => arch_name.to_string(),
            // Custom pattern: use as-is with {os} and {arch} placeholders
            custom => custom
                .replace("{os}", os_name)
                .replace("{arch}", arch_name),
        }
    }

    /// Get the effective archive extension
    pub fn effective_extension(&self) -> String {
        self.archive_type
            .as_deref()
            .unwrap_or("tar.gz")
            .to_string()
    }

    /// Build the download URL from pattern or direct URL
    pub fn build_url(&self, tool_name: &str) -> Option<String> {
        // If direct URL is provided, use it (with expansion)
        if let Some(ref url) = self.url {
            return Some(self.expand_template(url, tool_name));
        }

        // If base_url and path are provided, build from pattern
        if self.base_url.is_some() && self.path.is_some() {
            let pattern = self.url_pattern.as_deref().unwrap_or(
                "{base_url}/{path}/{version}/{archive_name}-{version}-{platform}.{ext}",
            );
            return Some(self.expand_template(pattern, tool_name));
        }

        None
    }

    /// Get the effective binary name
    pub fn effective_binary(&self, tool_name: &str) -> String {
        self.binary.clone().unwrap_or_else(|| tool_name.to_string())
    }

    /// Check if this is a direct binary download (no archive extraction)
    pub fn is_binary_download(&self) -> bool {
        self.archive_type.as_deref() == Some("binary")
    }

    /// Validate the tool definition
    pub fn validate(&self, name: &str) -> Result<()> {
        match self.source {
            ToolSource::Http => {
                // Either direct url OR base_url+path must be provided
                let has_direct_url = self.url.is_some();
                let has_pattern_url = self.base_url.is_some() && self.path.is_some();
                if !has_direct_url && !has_pattern_url {
                    anyhow::bail!(
                        "Tool '{}': HTTP source requires either 'url' or 'base_url' + 'path'",
                        name
                    );
                }
            }
            ToolSource::Container => {
                if self.image.is_none() {
                    anyhow::bail!("Tool '{}': Container source requires 'image' field", name);
                }
                if self.binary_path.is_none() {
                    anyhow::bail!(
                        "Tool '{}': Container source requires 'binary_path' field",
                        name
                    );
                }
            }
            ToolSource::GithubRelease => {
                if self.repo.is_none() {
                    anyhow::bail!("Tool '{}': GitHub release source requires 'repo' field", name);
                }
            }
            ToolSource::Cargo => {
                if self.crate_name.is_none() && self.git.is_none() {
                    anyhow::bail!(
                        "Tool '{}': Cargo source requires 'crate' or 'git' field",
                        name
                    );
                }
            }
        }
        Ok(())
    }
}

impl Default for ToolDefinition {
    fn default() -> Self {
        Self {
            source: ToolSource::Http,
            description: String::new(),
            url: None,
            base_url: None,
            path: None,
            url_pattern: None,
            archive_name: None,
            platform_style: None,
            archive_path: None,
            repo: None,
            asset: None,
            image: None,
            package: None,
            packages: Vec::new(),
            package_manager: None,
            binary_path: None,
            pre_install: None,
            crate_name: None,
            git: None,
            branch: None,
            tag: None,
            rev: None,
            features: Vec::new(),
            all_features: false,
            locked: false,
            cargo_args: Vec::new(),
            version: None,
            binary: None,
            install_dir: None,
            runtime: None,
            archive_type: None,
            post_install: None,
            enabled: true,
        }
    }
}

impl ToolsSection {
    /// Get all enabled tool definitions
    pub fn enabled_tools(&self) -> impl Iterator<Item = (&String, &ToolDefinition)> {
        self.definitions.iter().filter(|(_, def)| def.enabled)
    }

    /// Get a specific tool definition
    pub fn get(&self, name: &str) -> Option<&ToolDefinition> {
        self.definitions.get(name)
    }

    /// Validate all tool definitions
    pub fn validate(&self) -> Result<()> {
        for (name, def) in &self.definitions {
            def.validate(name)?;
        }
        Ok(())
    }
}

// ============================================================================
// Tools State (Installation Tracking)
// ============================================================================

/// Tool installation tracking (stored separately from main config)
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ToolsConfig {
    /// Installed tools
    #[serde(default)]
    pub tools: HashMap<String, InstalledTool>,
}

impl ToolsConfig {
    /// Load the tools config from ~/.config/bossa/tools.toml
    pub fn load() -> Result<Self> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        let config_path = home.join(".config").join("bossa").join("tools.toml");

        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Could not read tools config: {}", config_path.display()))?;

        toml::from_str(&content).context("Invalid TOML format in tools config")
    }

    /// Save the tools config to ~/.config/bossa/tools.toml
    pub fn save(&self) -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        let config_dir = home.join(".config").join("bossa");
        std::fs::create_dir_all(&config_dir)?;

        let config_path = config_dir.join("tools.toml");
        let content = toml::to_string_pretty(self).context("Failed to serialize tools config")?;
        std::fs::write(&config_path, content)?;

        Ok(config_path)
    }

    /// Get an installed tool by name
    pub fn get(&self, name: &str) -> Option<&InstalledTool> {
        self.tools.get(name)
    }

    /// Add or update an installed tool
    pub fn insert(&mut self, name: String, tool: InstalledTool) {
        self.tools.insert(name, tool);
    }

    /// Remove an installed tool
    pub fn remove(&mut self, name: &str) -> Option<InstalledTool> {
        self.tools.remove(name)
    }
}

/// Information about an installed tool
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InstalledTool {
    /// Original download URL or container image
    pub url: String,
    /// Binary name
    pub binary: String,
    /// Installation path
    pub install_path: String,
    /// Installation timestamp (ISO 8601)
    pub installed_at: String,
    /// Source type (http, container, brew, npm, etc.)
    #[serde(default = "default_source")]
    pub source: String,
    /// Container-specific metadata (only for source=container)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerMeta>,
}

/// Metadata for tools installed from containers
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContainerMeta {
    /// Container image used
    pub image: String,
    /// Package installed (if any)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Path inside container where binary was located
    pub binary_path: String,
    /// Package manager used
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<String>,
    /// Container runtime (podman/docker)
    #[serde(default = "default_runtime")]
    pub runtime: String,
}

fn default_source() -> String {
    "http".to_string()
}

fn default_runtime() -> String {
    "podman".to_string()
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

    #[test]
    fn test_tool_definition_build_url_direct() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            url: Some("https://example.com/{version}/tool-{platform}.tar.gz".to_string()),
            version: Some("1.2.3".to_string()),
            ..Default::default()
        };

        let url = def.build_url("mytool").unwrap();
        assert!(url.contains("1.2.3"));
        // Platform depends on the current system
        assert!(url.contains("https://example.com/1.2.3/tool-"));
    }

    #[test]
    fn test_tool_definition_build_url_pattern() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            base_url: Some("https://releases.example.com/artifacts".to_string()),
            path: Some("org/mytool".to_string()),
            version: Some("0.9.2".to_string()),
            archive_name: Some("mytool".to_string()),
            ..Default::default()
        };

        let url = def.build_url("mytool").unwrap();
        assert!(url.starts_with("https://releases.example.com/artifacts/org/mytool/0.9.2/mytool-0.9.2-"));
    }

    #[test]
    fn test_tool_definition_build_url_custom_pattern() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            base_url: Some("https://downloads.example.com/bin".to_string()),
            path: Some("tools/mycli".to_string()),
            version: Some("20250910".to_string()),
            url_pattern: Some("{base_url}/{path}/{version}/mycli-{platform}".to_string()),
            archive_type: Some("binary".to_string()),
            ..Default::default()
        };

        let url = def.build_url("mycli").unwrap();
        assert!(url.starts_with("https://downloads.example.com/bin/tools/mycli/20250910/mycli-"));
        assert!(!url.ends_with(".tar.gz")); // Binary download, no extension in pattern
    }

    #[test]
    fn test_tool_definition_platform_style_long() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            platform_style: Some("long".to_string()),
            ..Default::default()
        };

        let platform = def.get_platform_string();
        // Format should be os-arch (e.g., darwin-arm64, linux-amd64)
        assert!(platform.contains('-'));
    }

    #[test]
    fn test_tool_definition_platform_style_short() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            platform_style: Some("short".to_string()),
            ..Default::default()
        };

        let platform = def.get_platform_string();
        // Format should be os_arch (e.g., darwin_arm64, linux_amd64)
        assert!(platform.contains('_'));
    }

    #[test]
    fn test_tool_definition_platform_style_custom() {
        let def = ToolDefinition {
            source: ToolSource::Http,
            platform_style: Some("{os}_{arch}_custom".to_string()),
            ..Default::default()
        };

        let platform = def.get_platform_string();
        assert!(platform.ends_with("_custom"));
    }

    #[test]
    fn test_tool_definition_is_binary_download() {
        let binary_def = ToolDefinition {
            source: ToolSource::Http,
            archive_type: Some("binary".to_string()),
            ..Default::default()
        };
        assert!(binary_def.is_binary_download());

        let targz_def = ToolDefinition {
            source: ToolSource::Http,
            archive_type: Some("tar.gz".to_string()),
            ..Default::default()
        };
        assert!(!targz_def.is_binary_download());

        let default_def = ToolDefinition {
            source: ToolSource::Http,
            ..Default::default()
        };
        assert!(!default_def.is_binary_download());
    }

    #[test]
    fn test_tool_definition_effective_extension() {
        let def_default = ToolDefinition {
            source: ToolSource::Http,
            ..Default::default()
        };
        assert_eq!(def_default.effective_extension(), "tar.gz");

        let def_zip = ToolDefinition {
            source: ToolSource::Http,
            archive_type: Some("zip".to_string()),
            ..Default::default()
        };
        assert_eq!(def_zip.effective_extension(), "zip");
    }

    #[test]
    fn test_tool_definition_validation_http_base_url_path() {
        // Valid: base_url + path
        let def = ToolDefinition {
            source: ToolSource::Http,
            base_url: Some("https://example.com".to_string()),
            path: Some("tools/mytool".to_string()),
            ..Default::default()
        };
        assert!(def.validate("test").is_ok());

        // Invalid: only base_url without path
        let def_invalid = ToolDefinition {
            source: ToolSource::Http,
            base_url: Some("https://example.com".to_string()),
            ..Default::default()
        };
        assert!(def_invalid.validate("test").is_err());
    }
}
