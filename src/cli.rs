#![allow(dead_code)]

use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use std::fmt;

#[derive(Parser)]
#[command(name = "bossa")]
#[command(author = "Alberto Cavalcante")]
#[command(version)]
#[command(about = "Unified CLI for managing your dev environment", long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Verbosity level
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Bootstrap a new machine (bossa nova!)
    Nova(NovaArgs),

    /// Show current vs desired state
    Status(StatusArgs),

    /// Apply desired state
    Apply(ApplyArgs),

    /// Preview what would change
    Diff(DiffArgs),

    /// Add resources to config
    #[command(subcommand)]
    Add(AddCommand),

    /// Remove resources from config
    #[command(subcommand)]
    Rm(RmCommand),

    /// List resources
    List(ListArgs),

    /// Show detailed resource info
    Show(ShowArgs),

    /// System health check
    Doctor,

    /// Migrate old config format to new unified format
    Migrate {
        /// Preview changes without writing
        #[arg(long, short = 'n')]
        dry_run: bool,
    },

    /// Manage cache locations on external drive
    #[command(subcommand)]
    Caches(CachesCommand),

    /// Manage collections (generic repos)
    #[command(subcommand)]
    Collections(CollectionsCommand),

    /// Content manifest - hash files, find duplicates
    #[command(subcommand)]
    Manifest(ManifestCommand),

    /// iCloud Drive storage management
    #[command(subcommand, name = "icloud")]
    ICloud(ICloudCommand),

    /// Unified storage overview (SSD, iCloud, external drives)
    #[command(subcommand, name = "storage")]
    Storage(StorageCommand),

    /// Homebrew package management
    #[command(subcommand)]
    Brew(BrewCommand),

    /// [DEPRECATED] Manage reference repositories (use 'collections' instead)
    #[command(subcommand)]
    Refs(RefsCommand),

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

// ============================================================================
// Command Arguments
// ============================================================================

#[derive(Parser)]
pub struct StatusArgs {
    /// Target resource (e.g., "collections", "collections.refs", "workspaces", "storage.t9")
    pub target: Option<String>,
}

#[derive(Parser)]
pub struct ApplyArgs {
    /// Target resource (e.g., "collections", "collections.refs", "workspaces", "storage.t9")
    pub target: Option<String>,

    /// Dry run - show what would be done
    #[arg(short, long)]
    pub dry_run: bool,

    /// Number of parallel jobs (max 128)
    #[arg(short, long, default_value = "4", value_parser = clap::value_parser!(u16).range(1..=128))]
    pub jobs: u16,
}

#[derive(Parser)]
pub struct DiffArgs {
    /// Target resource (e.g., "collections", "collections.refs", "workspaces", "storage.t9")
    pub target: Option<String>,
}

#[derive(Parser)]
pub struct ListArgs {
    /// Resource type to list
    #[arg(value_enum)]
    pub resource_type: ResourceType,
}

#[derive(Parser)]
pub struct ShowArgs {
    /// Target resource (e.g., "collections.refs", "workspaces.dotfiles", "storage.t9")
    pub target: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ResourceType {
    /// List collections
    Collections,
    /// List repositories
    Repos,
    /// List workspaces
    Workspaces,
    /// List storage
    Storage,
}

// ============================================================================
// Add Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum AddCommand {
    /// Add a new collection
    Collection {
        /// Collection name
        name: String,

        /// Collection path
        path: String,

        /// Collection description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Add a repository to a collection
    Repo {
        /// Collection name
        collection: String,

        /// Repository URL
        url: String,

        /// Repository name (defaults to repo name from URL)
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Add a workspace
    Workspace {
        /// Repository URL
        url: String,

        /// Workspace name (defaults to repo name from URL)
        #[arg(short, long)]
        name: Option<String>,

        /// Workspace category
        #[arg(short, long)]
        category: Option<String>,
    },

    /// Add storage
    Storage {
        /// Storage name
        name: String,

        /// Mount point
        mount: String,

        /// Storage type
        #[arg(short = 't', long)]
        storage_type: Option<String>,
    },
}

// ============================================================================
// Remove Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum RmCommand {
    /// Remove a collection
    Collection {
        /// Collection name
        name: String,
    },

    /// Remove a repository from a collection
    Repo {
        /// Collection name
        collection: String,

        /// Repository name
        name: String,
    },

    /// Remove a workspace
    Workspace {
        /// Workspace name
        name: String,
    },

    /// Remove storage
    Storage {
        /// Storage name
        name: String,
    },
}

// ============================================================================
// Caches Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum CachesCommand {
    /// Show cache status and symlink health
    Status,

    /// Apply cache configuration (create symlinks, configs)
    Apply {
        /// Dry run - show what would be done
        #[arg(long)]
        dry_run: bool,
    },

    /// Detect drift from expected configuration
    Audit,

    /// Health check for cache system
    Doctor,

    /// Initialize cache configuration with defaults
    Init {
        /// Overwrite existing config
        #[arg(short, long)]
        force: bool,
    },
}

// ============================================================================
// Collections Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum CollectionsCommand {
    /// List all collections
    List,

    /// Show collection status (repos, clone state)
    Status {
        /// Collection name
        name: String,
    },

    /// Sync collection (clone missing repos)
    Sync {
        /// Collection name
        name: String,

        /// Parallel jobs
        #[arg(short, long, default_value = "4")]
        jobs: usize,

        /// Retry attempts for failed clones
        #[arg(short, long, default_value = "3")]
        retries: usize,

        /// Dry run
        #[arg(long)]
        dry_run: bool,
    },

    /// Audit for drift (repos on disk not in config)
    Audit {
        /// Collection name
        name: String,

        /// Auto-fix by adding untracked repos to config
        #[arg(long)]
        fix: bool,
    },

    /// Snapshot collection from disk (regenerate config)
    Snapshot {
        /// Collection name
        name: String,
    },

    /// Add repo to collection
    Add {
        /// Collection name
        collection: String,

        /// Repository URL
        url: String,

        /// Override repo name
        #[arg(short, long)]
        name: Option<String>,

        /// Clone immediately after adding
        #[arg(long)]
        clone: bool,
    },

    /// Remove repo from collection
    Rm {
        /// Collection name
        collection: String,

        /// Repository name
        repo: String,

        /// Delete local clone
        #[arg(long)]
        delete: bool,
    },

    /// Clean collection (delete clones from disk, preserve config)
    Clean {
        /// Collection name
        name: String,

        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,

        /// Show what would be deleted without deleting
        #[arg(long)]
        dry_run: bool,
    },
}

// ============================================================================
// Manifest Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum ManifestCommand {
    /// Scan directory and build content hash manifest
    Scan {
        /// Path to scan (e.g., /Volumes/T9, ~/dev/refs)
        path: String,

        /// Force re-scan all files (ignore cached hashes)
        #[arg(short, long)]
        force: bool,
    },

    /// Show manifest statistics
    Stats {
        /// Path to show stats for
        path: String,
    },

    /// Find and optionally delete duplicate files
    Duplicates {
        /// Path to check for duplicates
        path: String,

        /// Minimum file size to consider (bytes, default 1KB)
        #[arg(long, default_value = "1024")]
        min_size: u64,

        /// Interactively delete duplicates (keeps first, deletes rest)
        #[arg(long)]
        delete: bool,
    },
}

// ============================================================================
// Storage Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum StorageCommand {
    /// Show unified storage overview
    Status,

    /// Find duplicates across storage locations (requires scanned manifests)
    ///
    /// Compare files across different storage locations to find duplicates.
    /// Requires manifests to be scanned first with `bossa manifest scan <path>`.
    ///
    /// Examples:
    ///   bossa storage duplicates              # Compare all manifests
    ///   bossa storage duplicates icloud t9    # Compare only iCloud vs T9
    ///   bossa storage duplicates --list       # Show available manifests
    Duplicates {
        /// Manifest names to compare (e.g., "icloud t9"). If empty, compares all.
        /// Use --list to see available manifest names.
        #[arg(value_name = "MANIFEST")]
        manifests: Vec<String>,

        /// List available manifests and exit (don't compare)
        #[arg(long, short)]
        list: bool,

        /// Minimum file size to consider (e.g., 1048576 for 1MB)
        #[arg(long, default_value = "1048576")]
        min_size: u64,

        /// Maximum duplicates to show per comparison (0 = unlimited)
        #[arg(long, default_value = "10")]
        limit: usize,
    },
}

// ============================================================================
// Brew Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum BrewCommand {
    /// Apply Brewfile - install missing packages
    Apply {
        /// Only install essential packages (taps + core formulas, no casks/mas/vscode)
        #[arg(long)]
        essential: bool,

        /// Preview what would be installed without doing it
        #[arg(long)]
        dry_run: bool,

        /// Path to Brewfile (defaults to ~/dotfiles/Brewfile)
        #[arg(long, short)]
        file: Option<String>,
    },

    /// Capture installed packages to Brewfile
    Capture {
        /// Output path (defaults to ~/dotfiles/Brewfile)
        #[arg(long)]
        output: Option<String>,
    },

    /// Detect drift between installed packages and Brewfile
    Audit {
        /// Path to Brewfile (defaults to ~/dotfiles/Brewfile)
        #[arg(long, short)]
        file: Option<String>,
    },

    /// List installed Homebrew packages
    List {
        /// Filter by package type (tap, brew, cask, mas, vscode)
        #[arg(long, short)]
        r#type: Option<String>,
    },
}

// ============================================================================
// iCloud Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum ICloudCommand {
    /// Show status of iCloud files
    Status {
        /// Path to check (defaults to iCloud Drive root)
        path: Option<String>,
    },

    /// List files in iCloud directory with their status
    List {
        /// Path to list (defaults to iCloud Drive root)
        path: Option<String>,

        /// Show only local files (downloaded)
        #[arg(long)]
        local: bool,

        /// Show only cloud-only files (evicted)
        #[arg(long)]
        cloud: bool,
    },

    /// Find large local files that could be evicted
    FindEvictable {
        /// Path to search (defaults to iCloud Drive root)
        path: Option<String>,

        /// Minimum file size to consider (e.g., "100MB", "1GB")
        #[arg(long, short, default_value = "100MB")]
        min_size: String,
    },

    /// Evict files to free local space (keeps files in iCloud)
    Evict {
        /// Path to evict (file or directory)
        path: String,

        /// Recursively evict directory contents
        #[arg(long, short)]
        recursive: bool,

        /// Only evict files larger than this size (e.g., "50MB")
        #[arg(long)]
        min_size: Option<String>,

        /// Preview what would be evicted without doing it
        #[arg(long)]
        dry_run: bool,
    },

    /// Download files from iCloud to local storage
    Download {
        /// Path to download (file or directory)
        path: String,

        /// Recursively download directory contents
        #[arg(long, short)]
        recursive: bool,
    },
}

// ============================================================================
// Refs Commands (DEPRECATED - forwards to Collections)
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum RefsCommand {
    /// Sync reference repos (clone missing)
    Sync(RefsSyncArgs),

    /// List reference repos
    List {
        /// Filter by name pattern
        filter: Option<String>,

        /// Show only missing repos
        #[arg(long)]
        missing: bool,
    },

    /// Snapshot - regenerate refs.json from disk
    Snapshot,

    /// Audit - detect drift (untracked repos)
    Audit {
        /// Auto-fix by adding untracked repos
        #[arg(long)]
        fix: bool,
    },

    /// Add a new reference repo
    Add {
        /// Repository URL
        url: String,

        /// Override repo name (defaults to repo name from URL)
        #[arg(short, long)]
        name: Option<String>,

        /// Clone immediately after adding
        #[arg(long)]
        clone: bool,
    },

    /// Remove a reference repo
    Remove {
        /// Repository name
        name: String,

        /// Delete local clone
        #[arg(long)]
        delete: bool,
    },
}

#[derive(Debug, Parser)]
pub struct RefsSyncArgs {
    /// Specific repo name to sync (if not provided, syncs all missing)
    pub name: Option<String>,

    /// Number of parallel jobs
    #[arg(short, long, default_value = "4")]
    pub jobs: usize,

    /// Retry attempts for failed clones
    #[arg(short, long, default_value = "3")]
    pub retries: usize,

    /// Dry run - show what would be done
    #[arg(long)]
    pub dry_run: bool,
}

// ============================================================================
// Nova (Bootstrap) Commands
// ============================================================================

#[derive(Parser)]
pub struct NovaArgs {
    /// Skip specific stages (comma-separated)
    #[arg(long)]
    pub skip: Option<String>,

    /// Only run specific stages (comma-separated)
    #[arg(long)]
    pub only: Option<String>,

    /// List all available stages
    #[arg(long)]
    pub list_stages: bool,

    /// Dry run - show what would be done
    #[arg(long)]
    pub dry_run: bool,

    /// Number of parallel jobs (max 128)
    #[arg(short, long, value_parser = clap::value_parser!(u16).range(1..=128))]
    pub jobs: Option<u16>,
}

/// Nova bootstrap stages
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum NovaStage {
    Defaults,
    Terminal,
    GitSigning,
    Homebrew,
    Bash,
    Essential,
    Brew,
    Pnpm,
    Dock,
    Ecosystem,
    Handlers,
    Stow,
    Caches,
    Mcp,
    Refs,
    Workspaces,
}

impl NovaStage {
    pub fn all() -> &'static [NovaStage] {
        &[
            NovaStage::Defaults,
            NovaStage::Terminal,
            NovaStage::GitSigning,
            NovaStage::Homebrew,
            NovaStage::Bash,
            NovaStage::Essential,
            NovaStage::Brew,
            NovaStage::Pnpm,
            NovaStage::Dock,
            NovaStage::Ecosystem,
            NovaStage::Handlers,
            NovaStage::Stow,
            NovaStage::Caches,
            NovaStage::Mcp,
            NovaStage::Refs,
            NovaStage::Workspaces,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            NovaStage::Defaults => "defaults",
            NovaStage::Terminal => "terminal",
            NovaStage::GitSigning => "git-signing",
            NovaStage::Homebrew => "homebrew",
            NovaStage::Bash => "bash",
            NovaStage::Essential => "essential",
            NovaStage::Brew => "brew",
            NovaStage::Pnpm => "pnpm",
            NovaStage::Dock => "dock",
            NovaStage::Ecosystem => "ecosystem",
            NovaStage::Handlers => "handlers",
            NovaStage::Stow => "stow",
            NovaStage::Caches => "caches",
            NovaStage::Mcp => "mcp",
            NovaStage::Refs => "refs",
            NovaStage::Workspaces => "workspaces",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            NovaStage::Defaults => "macOS system defaults",
            NovaStage::Terminal => "Terminal font setup",
            NovaStage::GitSigning => "Git signing key setup",
            NovaStage::Homebrew => "Homebrew installation",
            NovaStage::Bash => "Bash 4+ bootstrap",
            NovaStage::Essential => "Essential packages (stow, jq, gh, etc.)",
            NovaStage::Brew => "Full Brewfile packages",
            NovaStage::Pnpm => "Node packages via pnpm",
            NovaStage::Dock => "Dock configuration",
            NovaStage::Ecosystem => "Ecosystem extensions",
            NovaStage::Handlers => "File handlers (duti)",
            NovaStage::Stow => "Symlinks via GNU Stow",
            NovaStage::Caches => "Cache symlinks to external drive",
            NovaStage::Mcp => "MCP server configuration",
            NovaStage::Refs => "Reference repositories",
            NovaStage::Workspaces => "Developer workspaces",
        }
    }

    pub fn from_name(name: &str) -> Option<NovaStage> {
        match name {
            "defaults" => Some(NovaStage::Defaults),
            "terminal" => Some(NovaStage::Terminal),
            "git-signing" => Some(NovaStage::GitSigning),
            "homebrew" => Some(NovaStage::Homebrew),
            "bash" => Some(NovaStage::Bash),
            "essential" => Some(NovaStage::Essential),
            "brew" => Some(NovaStage::Brew),
            "pnpm" => Some(NovaStage::Pnpm),
            "dock" => Some(NovaStage::Dock),
            "ecosystem" => Some(NovaStage::Ecosystem),
            "handlers" => Some(NovaStage::Handlers),
            "stow" => Some(NovaStage::Stow),
            "caches" => Some(NovaStage::Caches),
            "mcp" => Some(NovaStage::Mcp),
            "refs" => Some(NovaStage::Refs),
            "workspaces" => Some(NovaStage::Workspaces),
            _ => None,
        }
    }
}

// ============================================================================
// Target Parser
// ============================================================================

/// Represents a parsed target like "collections.refs" or "workspaces"
#[derive(Debug, Clone)]
pub struct Target {
    /// Resource type (e.g., "collections", "workspaces", "storage")
    pub resource_type: String,
    /// Optional specific resource name (e.g., "refs", "dotfiles", "t9")
    pub name: Option<String>,
}

impl Target {
    /// Parse a target string like "collections.refs" into (resource_type, name)
    pub fn parse(target: &str) -> Self {
        let target = target.trim();
        match target.split_once('.') {
            Some((resource_type, name)) => Self {
                resource_type: resource_type.trim().to_string(),
                name: Some(name.trim().to_string()),
            },
            None => Self {
                resource_type: target.to_string(),
                name: None,
            },
        }
    }

    /// Check if this target matches a specific resource type
    pub fn matches_type(&self, resource_type: &str) -> bool {
        self.resource_type == resource_type
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{}.{}", self.resource_type, name),
            None => write!(f, "{}", self.resource_type),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_parse_simple() {
        let target = Target::parse("collections");
        assert_eq!(target.resource_type, "collections");
        assert_eq!(target.name, None);
    }

    #[test]
    fn test_target_parse_with_name() {
        let target = Target::parse("collections.refs");
        assert_eq!(target.resource_type, "collections");
        assert_eq!(target.name, Some("refs".to_string()));
    }

    #[test]
    fn test_target_parse_multiple_dots() {
        let target = Target::parse("collections.refs.something");
        assert_eq!(target.resource_type, "collections");
        // Only splits on first dot
        assert_eq!(target.name, Some("refs.something".to_string()));
    }

    #[test]
    fn test_target_parse_empty_string() {
        let target = Target::parse("");
        assert_eq!(target.resource_type, "");
        assert_eq!(target.name, None);
    }

    #[test]
    fn test_target_parse_dot_only() {
        let target = Target::parse(".");
        assert_eq!(target.resource_type, "");
        assert_eq!(target.name, Some("".to_string()));
    }

    #[test]
    fn test_target_parse_trailing_dot() {
        let target = Target::parse("collections.");
        assert_eq!(target.resource_type, "collections");
        assert_eq!(target.name, Some("".to_string()));
    }

    #[test]
    fn test_target_parse_leading_dot() {
        let target = Target::parse(".refs");
        assert_eq!(target.resource_type, "");
        assert_eq!(target.name, Some("refs".to_string()));
    }

    #[test]
    fn test_target_parse_special_chars() {
        let target = Target::parse("coll-ect_ions.re-fs_123");
        assert_eq!(target.resource_type, "coll-ect_ions");
        assert_eq!(target.name, Some("re-fs_123".to_string()));
    }

    #[test]
    fn test_target_parse_path_traversal() {
        let target = Target::parse("../../etc.passwd");
        // split_once splits on first '.', so ".." is before the dot
        assert_eq!(target.resource_type, "");
        assert_eq!(target.name, Some("./../etc.passwd".to_string()));
    }

    #[test]
    fn test_target_parse_unicode() {
        let target = Target::parse("コレクション.レフ");
        assert_eq!(target.resource_type, "コレクション");
        assert_eq!(target.name, Some("レフ".to_string()));
    }

    #[test]
    fn test_target_matches_type() {
        let target = Target::parse("collections.refs");
        assert!(target.matches_type("collections"));
        assert!(!target.matches_type("workspaces"));
    }

    #[test]
    fn test_target_to_string() {
        let target1 = Target::parse("collections");
        assert_eq!(target1.to_string(), "collections");

        let target2 = Target::parse("collections.refs");
        assert_eq!(target2.to_string(), "collections.refs");
    }

    #[test]
    fn test_target_parse_roundtrip() {
        let inputs = vec![
            "collections",
            "collections.refs",
            "workspaces.dotfiles",
            "storage.t9",
        ];

        for input in inputs {
            let target = Target::parse(input);
            assert_eq!(target.to_string(), input);
        }
    }

    #[test]
    fn test_nova_stage_all() {
        let stages = NovaStage::all();
        assert_eq!(stages.len(), 16);
        assert_eq!(stages[0], NovaStage::Defaults);
        assert_eq!(stages[15], NovaStage::Workspaces);
    }

    #[test]
    fn test_nova_stage_name() {
        assert_eq!(NovaStage::Defaults.name(), "defaults");
        assert_eq!(NovaStage::GitSigning.name(), "git-signing");
        assert_eq!(NovaStage::Workspaces.name(), "workspaces");
    }

    #[test]
    fn test_nova_stage_from_name_valid() {
        assert_eq!(NovaStage::from_name("defaults"), Some(NovaStage::Defaults));
        assert_eq!(
            NovaStage::from_name("git-signing"),
            Some(NovaStage::GitSigning)
        );
        assert_eq!(
            NovaStage::from_name("workspaces"),
            Some(NovaStage::Workspaces)
        );
    }

    #[test]
    fn test_nova_stage_from_name_invalid() {
        assert_eq!(NovaStage::from_name("invalid"), None);
        assert_eq!(NovaStage::from_name(""), None);
        assert_eq!(NovaStage::from_name("Defaults"), None); // Case sensitive
    }

    #[test]
    fn test_nova_stage_description() {
        assert_eq!(NovaStage::Defaults.description(), "macOS system defaults");
        assert_eq!(NovaStage::Homebrew.description(), "Homebrew installation");
    }

    #[test]
    fn test_target_very_long_strings() {
        let long_type = "a".repeat(10000);
        let long_name = "b".repeat(10000);
        let target_str = format!("{}.{}", long_type, long_name);

        let target = Target::parse(&target_str);
        assert_eq!(target.resource_type.len(), 10000);
        assert_eq!(target.name.as_ref().unwrap().len(), 10000);
    }

    #[test]
    fn test_target_whitespace() {
        let target = Target::parse("  collections  .  refs  ");
        // Fixed: whitespace is now trimmed
        assert_eq!(target.resource_type, "collections");
        assert_eq!(target.name, Some("refs".to_string()));
    }
}
