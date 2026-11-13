#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::AppEnvironment;

// ============================================================================
// Main Config Schema
// ============================================================================

/// The unified bossa configuration structure
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
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

    /// Themes configuration (GNOME/GTK theme presets)
    #[serde(default)]
    pub themes: ThemesSection,

    /// Network configuration (proxies, registries)
    #[serde(default)]
    pub network: NetworkConfig,

    /// Logical locations for path management
    #[serde(default)]
    pub locations: LocationsConfig,

    /// Generated config files
    #[serde(default)]
    pub configs: ConfigsSection,

    /// Dotfiles repository management
    #[serde(default)]
    pub dotfiles: Option<DotfilesConfig>,

    /// Dotfiles reconciliation across multiple sources
    #[serde(default)]
    pub dotfiles_reconcile: Option<DotfilesReconcileConfig>,

    /// Homebrew cellar warehousing on external SSD
    #[serde(default)]
    pub cellar: CellarConfig,
}

impl BossaConfig {
    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate collections
        for (name, collection) in &self.collections {
            collection
                .validate()
                .with_context(|| format!("Invalid collection '{name}'"))?;
        }

        // Validate workspaces
        self.workspaces.validate()?;

        // Validate storage
        for (name, storage) in &self.storage {
            storage
                .validate()
                .with_context(|| format!("Invalid storage '{name}'"))?;
        }

        // Validate nova
        self.nova.validate()?;

        // Validate locations
        self.locations.validate()?;

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
#[serde(deny_unknown_fields)]
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
    pub fn expanded_path(&self, environment: &AppEnvironment) -> Result<PathBuf> {
        Ok(environment.expand(&self.path))
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
#[serde(deny_unknown_fields)]
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

impl CloneSettings {
    pub fn validate(&self) -> Result<()> {
        for option in &self.options {
            let flag = option.split('=').next().unwrap_or(option);
            if option == "--"
                || [
                    "--depth",
                    "--no-depth",
                    "--single-branch",
                    "--no-single-branch",
                    "--branch",
                    "--no-branch",
                ]
                .iter()
                .any(|protected| protected.starts_with(flag))
                || option.starts_with("-b")
            {
                anyhow::bail!(
                    "Clone options cannot override depth, single_branch, default_branch, or the argument boundary"
                );
            }
        }
        Ok(())
    }
}

/// A repository in a collection
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
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
        validate_relative_path(&self.name, false, "Repository name")?;
        validate_relative_path(&self.default_branch, true, "Repository default branch")?;
        Ok(())
    }
}

fn default_branch() -> String {
    "main".to_string()
}

// ============================================================================
// Workspaces - Bare repo + worktree structure
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
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

impl Default for WorkspacesConfig {
    fn default() -> Self {
        Self {
            root: default_workspaces_root(),
            structure: default_structure(),
            repos: Vec::new(),
        }
    }
}

impl WorkspacesConfig {
    /// Get the expanded root path
    pub fn expanded_root(&self, environment: &AppEnvironment) -> Result<PathBuf> {
        if self.is_historical_empty_default() {
            return environment.workspaces_dir();
        }
        Ok(environment.expand(&self.root))
    }

    fn is_historical_empty_default(&self) -> bool {
        self.root.is_empty() && self.structure.is_empty() && self.repos.is_empty()
    }

    /// Validate the workspaces config
    pub fn validate(&self) -> Result<()> {
        // Earlier CRUD serialized this exact unused default. It carries no
        // workspace intent and must remain readable without rewriting it.
        if self.is_historical_empty_default() {
            return Ok(());
        }
        if self.root.is_empty() {
            anyhow::bail!("Workspaces root cannot be empty");
        }
        if self.structure != "bare-worktree" {
            anyhow::bail!("Unsupported workspace structure: {}", self.structure);
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
        if self.is_historical_empty_default() {
            *self = Self::default();
        }
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
#[serde(deny_unknown_fields)]
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
        validate_relative_path(&self.name, false, "Repository name")?;
        validate_relative_path(&self.category, false, "Repository category")?;
        for branch in &self.worktrees {
            validate_relative_path(branch, true, "Worktree branch")?;
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

fn validate_relative_path(value: &str, nested: bool, label: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.starts_with('-')
        || value.contains(['\\', ':'])
        || value.chars().any(char::is_control)
        || (!nested && value.contains('/'))
        || value.split('/').any(|part| matches!(part, "" | "." | ".."))
    {
        anyhow::bail!(
            "{label} must be a safe relative {}",
            if nested { "path" } else { "name" }
        );
    }
    Ok(())
}

fn default_category() -> String {
    "default".to_string()
}

// ============================================================================
// Storage - External volumes with symlinks
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
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
    pub fn expanded_mount(&self, environment: &AppEnvironment) -> Result<PathBuf> {
        Ok(environment.expand(&self.mount))
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
#[serde(deny_unknown_fields)]
pub struct Symlink {
    /// Source path (the symlink location)
    pub from: String,

    /// Target path (what the symlink points to)
    /// Can contain {mount} placeholder
    pub to: String,
}

impl Symlink {
    /// Get the expanded from path
    pub fn expanded_from(&self, environment: &AppEnvironment) -> Result<PathBuf> {
        Ok(environment.expand(&self.from))
    }

    /// Get the expanded to path, replacing {mount} placeholder
    pub fn expanded_to(&self, mount_point: &str, environment: &AppEnvironment) -> Result<PathBuf> {
        let replaced = self.to.replace("{mount}", mount_point);
        Ok(environment.expand(&replaced))
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
#[serde(deny_unknown_fields)]
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
                anyhow::bail!("Duplicate stage: {stage}");
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
    Array(Vec<Self>),
}

impl DefaultValue {
    /// Convert to defaults command argument format
    pub fn to_defaults_args(&self) -> Vec<String> {
        match self {
            Self::Bool(b) => vec!["-bool".to_string(), b.to_string()],
            Self::Int(i) => vec!["-int".to_string(), i.to_string()],
            Self::Float(f) => vec!["-float".to_string(), f.to_string()],
            Self::String(s) => vec!["-string".to_string(), s.clone()],
            Self::Array(arr) => {
                let mut args = vec!["-array".to_string()];
                for item in arr {
                    match item {
                        Self::String(s) => args.push(s.clone()),
                        _ => args.push(format!("{item:?}")),
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct PnpmConfig {
    #[serde(default)]
    pub globals: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GhConfig {
    #[serde(default)]
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VscodeConfig {
    #[serde(default)]
    pub extensions: Vec<String>,
}

// ============================================================================
// Symlinks - Dotfile symlinks configuration (replaces stow)
// ============================================================================

/// Symlinks configuration (replaces stow)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct DockConfig {
    /// Auto-hide the dock
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autohide: Option<bool>,

    /// Tile size in pixels
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilesize: Option<u32>,

    /// Minimize effect ("scale" or "genie")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimize_effect: Option<String>,

    /// Show recent applications
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_recents: Option<bool>,

    /// Applications to pin to dock (in order)
    #[serde(default)]
    pub apps: Vec<String>,

    /// Folders to add to dock
    #[serde(default)]
    pub folders: Vec<DockFolder>,
}

/// Dock folder configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
// Themes Configuration (GNOME/GTK)
// ============================================================================

/// Themes configuration section
///
/// Example config:
/// ```toml
/// [themes.whitesur]
/// description = "macOS Big Sur style (dark)"
/// gtk = "WhiteSur-Dark"
/// shell = "WhiteSur-Dark"
/// wm = "WhiteSur-Dark"
/// wm_buttons = "close,minimize,maximize:"
/// icons = "WhiteSur-dark"
/// cursor = "WhiteSur-cursors"
/// terminal = "whitesur"
/// requires = ["whitesur-gtk", "whitesur-icons", "whitesur-cursors"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemesSection {
    /// Theme definitions (keyed by theme name)
    #[serde(flatten)]
    pub themes: HashMap<String, ThemeDefinition>,
}

/// A theme preset definition
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeDefinition {
    /// Human-readable description
    #[serde(default)]
    pub description: String,

    /// GTK theme name (apps like Nautilus, Settings)
    #[serde(default)]
    pub gtk: Option<String>,

    /// GNOME Shell theme (panel, overview, notifications)
    #[serde(default)]
    pub shell: Option<String>,

    /// Window manager theme (title bars, window decorations)
    #[serde(default)]
    pub wm: Option<String>,

    /// Window button layout (e.g., "close,minimize,maximize:" for left/macOS style)
    #[serde(default)]
    pub wm_buttons: Option<String>,

    /// Icon theme name
    #[serde(default)]
    pub icons: Option<String>,

    /// Cursor theme name
    #[serde(default)]
    pub cursor: Option<String>,

    /// Terminal color scheme (for gnome-terminal or similar)
    #[serde(default)]
    pub terminal: Option<String>,

    /// Tools/packages that must be installed first (from tools section)
    #[serde(default)]
    pub requires: Vec<String>,

    /// Whether this theme is enabled (default: true)
    #[serde(default = "default_theme_enabled")]
    pub enabled: bool,
}

fn default_theme_enabled() -> bool {
    true
}

impl ThemesSection {
    /// Get all enabled theme definitions
    pub fn enabled_themes(&self) -> impl Iterator<Item = (&String, &ThemeDefinition)> {
        self.themes.iter().filter(|(_, def)| def.enabled)
    }

    /// Get a specific theme definition
    pub fn get(&self, name: &str) -> Option<&ThemeDefinition> {
        self.themes.get(name)
    }
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
#[serde(deny_unknown_fields)]
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

    /// Base URL for artifact repositories (e.g., `https://releases.example.com/artifacts`)
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

    /// Platform availability configuration
    #[serde(default)]
    pub platforms: Option<PlatformsConfig>,

    // === Dependencies ===
    /// Tools that must be installed before this one (e.g., ["pnpm"] for bun)
    #[serde(default)]
    pub depends: Vec<String>,

    // === Npm source fields ===
    /// npm package name (defaults to tool name)
    #[serde(default)]
    pub npm_package: Option<String>,

    /// Whether to allow postinstall scripts (required by some packages like bun)
    #[serde(default)]
    pub needs_scripts: bool,
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
    /// npm/pnpm global install
    Npm,
}

/// Platform availability configuration
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct PlatformsConfig {
    /// Linux platform configurations
    #[serde(default)]
    pub linux: Option<HashMap<String, PlatformArch>>,

    /// macOS/Darwin platform configurations
    #[serde(default)]
    pub darwin: Option<HashMap<String, PlatformArch>>,

    /// Windows platform configurations
    #[serde(default)]
    pub windows: Option<HashMap<String, PlatformArch>>,
}

/// Architecture-specific platform configuration
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct PlatformArch {
    /// Whether this platform/arch combo is available
    #[serde(default = "default_available")]
    pub available: bool,

    /// Override archive type for this platform (e.g., "zip" for Windows)
    #[serde(default)]
    pub archive_type: Option<String>,

    /// Override URL for this platform
    #[serde(default)]
    pub url: Option<String>,

    /// Override archive path for this platform
    #[serde(default)]
    pub archive_path: Option<String>,
}

fn default_available() -> bool {
    true
}

impl PlatformsConfig {
    /// Check if the tool is available for the current platform
    pub fn is_available_for_current(&self) -> bool {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        // Normalize arch names
        let arch_name = match arch {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            "x86" => "386",
            other => other,
        };

        // Normalize OS names
        let os_name = match os {
            "macos" => "darwin",
            other => other,
        };

        self.is_available(os_name, arch_name)
    }

    /// Check if the tool is available for a specific OS and arch
    pub fn is_available(&self, os: &str, arch: &str) -> bool {
        let platform_map = match os {
            "linux" => &self.linux,
            "darwin" | "macos" => &self.darwin,
            "windows" => &self.windows,
            _ => return true, // Unknown OS, assume available
        };

        match platform_map {
            Some(archs) => {
                // If platform is specified, check if arch is available
                archs.get(arch).is_some_and(|p| p.available)
            }
            None => {
                // If platform not specified, assume available on all archs
                true
            }
        }
    }

    /// Get platform-specific overrides for current platform
    pub fn get_current_overrides(&self) -> Option<&PlatformArch> {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        let arch_name = match arch {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            "x86" => "386",
            other => other,
        };

        let os_name = match os {
            "macos" => "darwin",
            other => other,
        };

        let platform_map = match os_name {
            "linux" => &self.linux,
            "darwin" => &self.darwin,
            "windows" => &self.windows,
            _ => return None,
        };

        platform_map.as_ref().and_then(|archs| archs.get(arch_name))
    }
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
            "long" => format!("{os_name}-{arch_name}"),
            "short" | "go" => format!("{os_name}_{arch_name}"),
            "os-only" => os_name.to_string(),
            "arch-only" => arch_name.to_string(),
            // Custom pattern: use as-is with {os} and {arch} placeholders
            custom => custom.replace("{os}", os_name).replace("{arch}", arch_name),
        }
    }

    /// Get the effective archive extension
    pub fn effective_extension(&self) -> String {
        self.archive_type.as_deref().unwrap_or("tar.gz").to_string()
    }

    /// Build the download URL from pattern or direct URL
    pub fn build_url(&self, tool_name: &str) -> Option<String> {
        // If direct URL is provided, use it (with expansion)
        if let Some(ref url) = self.url {
            return Some(self.expand_template(url, tool_name));
        }

        // If base_url and path are provided, build from pattern
        if self.base_url.is_some() && self.path.is_some() {
            let pattern = self
                .url_pattern
                .as_deref()
                .unwrap_or("{base_url}/{path}/{version}/{archive_name}-{version}-{platform}.{ext}");
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

    /// Check if this tool is available for the current platform
    pub fn is_available_for_current_platform(&self) -> bool {
        match &self.platforms {
            Some(platforms) => platforms.is_available_for_current(),
            None => true, // No platforms specified means available everywhere
        }
    }

    /// Get platform-specific archive type override, if any
    pub fn get_effective_archive_type(&self) -> String {
        if let Some(ref platforms) = self.platforms
            && let Some(overrides) = platforms.get_current_overrides()
            && let Some(ref archive_type) = overrides.archive_type
        {
            return archive_type.clone();
        }
        self.archive_type
            .clone()
            .unwrap_or_else(|| "tar.gz".to_string())
    }

    /// Get platform-specific URL override, if any
    pub fn get_effective_url(&self, tool_name: &str) -> Option<String> {
        if let Some(ref platforms) = self.platforms
            && let Some(overrides) = platforms.get_current_overrides()
            && let Some(ref url) = overrides.url
        {
            return Some(self.expand_template(url, tool_name));
        }
        self.build_url(tool_name)
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
                        "Tool '{name}': HTTP source requires either 'url' or 'base_url' + 'path'"
                    );
                }
            }
            ToolSource::Container => {
                if self.image.is_none() {
                    anyhow::bail!("Tool '{name}': Container source requires 'image' field");
                }
                if self.binary_path.is_none() {
                    anyhow::bail!("Tool '{name}': Container source requires 'binary_path' field");
                }
            }
            ToolSource::GithubRelease => {
                if self.repo.is_none() {
                    anyhow::bail!("Tool '{name}': GitHub release source requires 'repo' field");
                }
            }
            ToolSource::Cargo => {
                if self.crate_name.is_none() && self.git.is_none() {
                    anyhow::bail!("Tool '{name}': Cargo source requires 'crate' or 'git' field");
                }
            }
            ToolSource::Npm => {
                // npm_package is optional, defaults to tool name
                // No required fields, but we should have npm or pnpm available
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
            platforms: None,
            depends: Vec::new(),
            npm_package: None,
            needs_scripts: false,
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
// Network Configuration
// ============================================================================

/// Network configuration for proxies and package registries
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    /// HTTP proxy URL
    #[serde(default)]
    pub http_proxy: Option<String>,

    /// HTTPS proxy URL
    #[serde(default)]
    pub https_proxy: Option<String>,

    /// Comma-separated list of hosts to bypass proxy
    #[serde(default)]
    pub no_proxy: Option<String>,

    /// Go-specific network configuration
    #[serde(default)]
    pub go: Option<GoNetworkConfig>,

    /// npm/pnpm/bun registry configuration
    #[serde(default)]
    pub npm: Option<NpmNetworkConfig>,

    /// Python/pip registry configuration
    #[serde(default)]
    pub python: Option<PythonNetworkConfig>,

    /// Cargo/Rust registry configuration
    #[serde(default)]
    pub cargo: Option<CargoNetworkConfig>,
}

/// Go-specific network configuration
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct GoNetworkConfig {
    /// GOPROXY value
    #[serde(default)]
    pub goproxy: Option<String>,

    /// GOSUMDB value
    #[serde(default)]
    pub gosumdb: Option<String>,

    /// GOPRIVATE value (comma-separated module paths)
    #[serde(default)]
    pub goprivate: Option<String>,

    /// GONOSUMDB value
    #[serde(default)]
    pub gonosumdb: Option<String>,

    /// Preferred Go version
    #[serde(default)]
    pub goversion: Option<String>,
}

/// npm/pnpm/bun network configuration
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct NpmNetworkConfig {
    /// npm registry URL
    #[serde(default)]
    pub registry: Option<String>,

    /// Scoped registries (e.g., "@myorg" -> "https://...")
    #[serde(default)]
    pub scoped: HashMap<String, String>,
}

/// Python/pip network configuration
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct PythonNetworkConfig {
    /// Primary index URL (pip --index-url)
    #[serde(default)]
    pub index_url: Option<String>,

    /// Extra index URLs (pip --extra-index-url)
    #[serde(default)]
    pub extra_index_urls: Vec<String>,

    /// Trusted hosts (pip --trusted-host)
    #[serde(default)]
    pub trusted_hosts: Vec<String>,
}

/// Cargo/Rust network configuration
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CargoNetworkConfig {
    /// Alternative registry name and URL
    #[serde(default)]
    pub registries: HashMap<String, String>,
}

impl NetworkConfig {
    /// Get environment variables to set for network configuration
    pub fn to_env_vars(&self) -> Vec<(String, String)> {
        let mut vars = Vec::new();

        if let Some(ref proxy) = self.http_proxy {
            vars.push(("HTTP_PROXY".to_string(), proxy.clone()));
            vars.push(("http_proxy".to_string(), proxy.clone()));
        }

        if let Some(ref proxy) = self.https_proxy {
            vars.push(("HTTPS_PROXY".to_string(), proxy.clone()));
            vars.push(("https_proxy".to_string(), proxy.clone()));
        }

        if let Some(ref no_proxy) = self.no_proxy {
            vars.push(("NO_PROXY".to_string(), no_proxy.clone()));
            vars.push(("no_proxy".to_string(), no_proxy.clone()));
        }

        if let Some(ref go) = self.go {
            if let Some(ref goproxy) = go.goproxy {
                vars.push(("GOPROXY".to_string(), goproxy.clone()));
            }
            if let Some(ref gosumdb) = go.gosumdb {
                vars.push(("GOSUMDB".to_string(), gosumdb.clone()));
            }
            if let Some(ref goprivate) = go.goprivate {
                vars.push(("GOPRIVATE".to_string(), goprivate.clone()));
            }
            if let Some(ref gonosumdb) = go.gonosumdb {
                vars.push(("GONOSUMDB".to_string(), gonosumdb.clone()));
            }
        }

        vars
    }

    /// Check if any proxy is configured
    pub fn has_proxy(&self) -> bool {
        self.http_proxy.is_some() || self.https_proxy.is_some()
    }
}

// ============================================================================
// Locations Configuration (Path Aliases)
// ============================================================================

/// Configuration for logical locations (path aliases)
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct LocationsConfig {
    /// Named location mappings (e.g., "dev" -> "/Volumes/T9/dev")
    #[serde(default)]
    pub paths: HashMap<String, String>,

    /// Aliases for historical paths that redirect to locations
    /// e.g., "~/dev" -> "dev" (meaning ~/dev should resolve to locations.dev)
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

impl LocationsConfig {
    /// Validate the locations config
    pub fn validate(&self) -> Result<()> {
        for name in self.paths.keys() {
            // Validate name is alphanumeric + underscore
            if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                anyhow::bail!("Invalid location name '{name}': must be alphanumeric or underscore");
            }
        }
        let environment = AppEnvironment::new(None, []);
        for path in self.paths.values() {
            crate::resolve_location_path(path, self, &environment)?;
        }
        Ok(())
    }

    /// Get a location path by name
    pub fn get(&self, name: &str) -> Option<&str> {
        self.paths.get(name).map(std::string::String::as_str)
    }

    /// Check if a path matches any alias and return the location name
    pub fn resolve_alias(&self, path: &str) -> Option<&str> {
        self.aliases.get(path).map(std::string::String::as_str)
    }
}

// ============================================================================
// Config Generation (Git)
// ============================================================================

/// Configuration for generated config files
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct ConfigsSection {
    /// Git configuration
    #[serde(default)]
    pub git: Option<GitConfig>,
}

/// Git configuration that will be generated to ~/.gitconfig
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GitConfig {
    /// Target file path (default: ~/.gitconfig)
    #[serde(default = "GitConfig::default_target")]
    pub target: String,

    /// User identity
    #[serde(default)]
    pub user: GitUserConfig,

    /// Core settings
    #[serde(default)]
    pub core: HashMap<String, toml::Value>,

    /// Init settings
    #[serde(default)]
    pub init: HashMap<String, toml::Value>,

    /// Pull settings
    #[serde(default)]
    pub pull: HashMap<String, toml::Value>,

    /// Push settings
    #[serde(default)]
    pub push: HashMap<String, toml::Value>,

    /// Merge settings
    #[serde(default)]
    pub merge: HashMap<String, toml::Value>,

    /// Diff settings
    #[serde(default)]
    pub diff: HashMap<String, toml::Value>,

    /// Aliases
    #[serde(default)]
    pub alias: HashMap<String, String>,

    /// Additional sections (for any other git config sections)
    #[serde(flatten)]
    pub extra: HashMap<String, HashMap<String, toml::Value>>,
}

impl GitConfig {
    fn default_target() -> String {
        "~/.gitconfig".to_string()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct GitUserConfig {
    pub name: Option<String>,
    pub email: Option<String>,
    pub signingkey: Option<String>,
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
// Dotfiles - Repository management
// ============================================================================

/// Dotfiles repository configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DotfilesConfig {
    /// Git repository URL
    pub repo: String,

    /// Local clone path (e.g., "~/.dotfiles")
    pub path: String,

    /// Branch to track
    #[serde(default = "default_branch")]
    pub branch: String,

    /// Public submodules to always initialize
    #[serde(default)]
    pub public_submodules: Vec<String>,

    /// Private submodule configuration (requires auth)
    #[serde(default)]
    pub private: Option<DotfilesPrivateConfig>,

    /// Submodules to skip (e.g., dev-only tools)
    #[serde(default)]
    pub skip_submodules: Vec<String>,
}

impl DotfilesConfig {
    /// Get the expanded local path
    pub fn expanded_path(&self, environment: &AppEnvironment) -> Result<PathBuf> {
        if self.path == "~" {
            return environment.home_dir().map(PathBuf::from);
        }
        if let Some(suffix) = self.path.strip_prefix("~/") {
            return Ok(environment.home_dir()?.join(suffix));
        }
        Ok(PathBuf::from(&self.path))
    }

    /// Validate the dotfiles config
    pub fn validate(&self) -> Result<()> {
        if self.repo.is_empty() {
            anyhow::bail!("Dotfiles repo URL cannot be empty");
        }
        if self.path.is_empty() {
            anyhow::bail!("Dotfiles path cannot be empty");
        }
        if self.branch.is_empty() {
            anyhow::bail!("Dotfiles branch cannot be empty");
        }
        if let Some(ref private) = self.private {
            private.validate()?;
        }
        Ok(())
    }
}

/// Private dotfiles submodule configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DotfilesPrivateConfig {
    /// Relative path within the dotfiles repo
    pub path: String,

    /// Git URL for the private submodule
    pub url: String,

    /// Whether GitHub auth is required
    #[serde(default)]
    pub requires_auth: bool,

    /// Script to run after initializing (relative to private submodule)
    #[serde(default)]
    pub setup_script: Option<String>,
}

impl DotfilesPrivateConfig {
    /// Validate the private config
    pub fn validate(&self) -> Result<()> {
        if self.path.is_empty() {
            anyhow::bail!("Private submodule path cannot be empty");
        }
        if self.url.is_empty() {
            anyhow::bail!("Private submodule URL cannot be empty");
        }
        Ok(())
    }
}

// ============================================================================
// Dotfiles Reconciliation Configuration
// ============================================================================

/// Configuration for reconciling dotfiles across multiple sources
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DotfilesReconcileConfig {
    /// Primary source (e.g., "~/.dotfiles")
    pub source_a: String,
    /// Secondary source (e.g., "/Volumes/T9/dev/ws/utils/dotfiles")
    pub source_b: String,
    /// Target directory where files are deployed (e.g., "~")
    pub target: String,
    /// Packages/subdirectories to compare (e.g., ["zsh", "git", "tmux"])
    /// If empty, compares all shared subdirectories
    #[serde(default)]
    pub packages: Vec<String>,
    /// File/directory patterns to ignore during comparison
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Default reconciliation strategy
    #[serde(default = "default_reconcile_strategy")]
    pub strategy: String,
}

fn default_reconcile_strategy() -> String {
    "interactive".to_string()
}

// ============================================================================
// Cellar Config (Homebrew warehouse on external SSD)
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct CellarConfig {
    /// Path to external cellar root (e.g., "/Volumes/T9/homebrew")
    #[serde(default)]
    pub path: String,
    /// Packages that must stay installed locally (plus their transitive deps)
    #[serde(default)]
    pub local: Vec<String>,
}

// ============================================================================
// Tests
// ============================================================================
