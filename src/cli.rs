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

    /// Disk management (status, backup, repartition)
    #[command(subcommand)]
    Disk(DiskCommand),

    /// Homebrew package management
    #[command(subcommand)]
    Brew(BrewCommand),

    /// \[DEPRECATED\] Manage reference repositories (use 'collections' instead)
    #[command(subcommand)]
    Refs(RefsCommand),

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Install and manage development tools
    #[command(subcommand)]
    Tools(ToolsCommand),

    /// Manage dotfile symlinks (native stow replacement)
    #[command(subcommand)]
    Stow(StowCommand),

    /// Apply GNOME/GTK theme presets (Linux only)
    #[command(subcommand)]
    Theme(ThemeCommand),

    /// Manage macOS defaults (imperative)
    #[command(subcommand)]
    Defaults(DefaultsCommand),

    /// Manage logical locations for path abstraction
    #[command(subcommand)]
    Locations(LocationsCommand),
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
// Disk Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum DiskCommand {
    /// Show all disks with partitions, formats, and space usage
    ///
    /// Lists internal and external disks with their partitions,
    /// showing filesystem type (APFS, ExFAT, etc.), mount points,
    /// and used/free space.
    Status,

    /// Backup directory to another location with progress
    ///
    /// Copies files while skipping system files (.DS_Store, .Spotlight-V100, etc.).
    /// Shows a progress bar and validates destination has enough space.
    ///
    /// Examples:
    ///   bossa disk backup /Volumes/T9/photos /Volumes/Backup/photos
    ///   bossa disk backup ~/Documents /Volumes/External/Documents --dry-run
    Backup {
        /// Source directory to backup
        source: String,

        /// Destination directory
        destination: String,

        /// Preview what would be copied without copying
        #[arg(long, short = 'n')]
        dry_run: bool,
    },

    /// Repartition an external drive (DESTRUCTIVE)
    ///
    /// Interactive guided repartition for external drives.
    /// Will REFUSE to operate on internal or boot disks for safety.
    ///
    /// Examples:
    ///   bossa disk repartition disk2                    # Interactive, shows command only
    ///   bossa disk repartition disk2 --dry-run          # Preview mode
    ///   bossa disk repartition disk2 --confirm          # Actually execute
    ///
    /// SAFETY: Always shows the generated diskutil command before execution.
    /// Requires explicit --confirm flag to actually run the command.
    Repartition {
        /// Disk identifier (e.g., "disk2" or "/dev/disk2")
        disk: String,

        /// Preview what would happen without executing
        #[arg(long, short = 'n')]
        dry_run: bool,

        /// Actually execute the repartition (DESTRUCTIVE)
        #[arg(long)]
        confirm: bool,
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
            NovaStage::Stow => "Dotfile symlinks (native stow replacement)",
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
// Tools Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum ToolsCommand {
    /// Install a tool from URL (tar.gz)
    Install {
        /// Tool name
        name: String,

        /// Download URL (must be .tar.gz)
        #[arg(long)]
        url: String,

        /// Binary name inside archive (defaults to tool name)
        #[arg(long)]
        binary: Option<String>,

        /// Path inside archive where binary is located
        #[arg(long)]
        path: Option<String>,

        /// Installation directory (defaults to ~/.local/bin)
        #[arg(long)]
        install_dir: Option<String>,

        /// Force reinstall
        #[arg(long, short)]
        force: bool,
    },

    /// Install a tool from a container image (podman/docker)
    ///
    /// Extract binaries from container images. Useful for getting tools
    /// packaged for other distros (e.g., RPMs from Fedora on macOS).
    ///
    /// Examples:
    ///   # Install ripgrep from Fedora
    ///   bossa tools install-container rg --image fedora:latest \
    ///     --package ripgrep --binary-path /usr/bin/rg
    ///
    ///   # Install from UBI minimal with microdnf
    ///   bossa tools install-container jq --image registry.access.redhat.com/ubi9/ubi-minimal \
    ///     --package jq --binary-path /usr/bin/jq --package-manager microdnf
    ///
    ///   # Extract existing binary without installing package
    ///   bossa tools install-container bash --image alpine:latest \
    ///     --binary-path /bin/bash
    #[command(name = "install-container")]
    InstallContainer {
        /// Tool name (used for tracking)
        name: String,

        /// Container image to use (e.g., fedora:latest, ubi9/ubi-minimal)
        #[arg(long, short)]
        image: String,

        /// Package to install inside container (optional if binary exists in base image)
        #[arg(long, short)]
        package: Option<String>,

        /// Path to binary inside the container (e.g., /usr/bin/rg)
        #[arg(long, short = 'b')]
        binary_path: String,

        /// Package manager to use: dnf, microdnf, apt, apk, yum (auto-detected if not set)
        #[arg(long, short = 'm')]
        package_manager: Option<String>,

        /// Container runtime: podman or docker (default: podman)
        #[arg(long, short = 'r', default_value = "podman")]
        runtime: String,

        /// Additional packages to install (e.g., dependencies)
        #[arg(long, short = 'd')]
        dependencies: Option<Vec<String>>,

        /// Command to run before package installation (e.g., enable repos)
        #[arg(long)]
        pre_install: Option<String>,

        /// Keep the container after extraction (for debugging)
        #[arg(long)]
        keep_container: bool,

        /// Installation directory (defaults to ~/.local/bin)
        #[arg(long)]
        install_dir: Option<String>,

        /// Force reinstall
        #[arg(long, short)]
        force: bool,
    },

    /// Apply tools from config (install missing, update outdated)
    ///
    /// Reads tool definitions from ~/.config/bossa/config.toml and ensures
    /// all enabled tools are installed.
    Apply {
        /// Only apply specific tool(s)
        #[arg(value_name = "TOOL")]
        tools: Vec<String>,

        /// Preview what would be installed without doing it
        #[arg(long, short = 'n')]
        dry_run: bool,

        /// Force reinstall even if already installed
        #[arg(long, short)]
        force: bool,
    },

    /// List installed tools
    List {
        /// Also show tools defined in config but not installed
        #[arg(long, short)]
        all: bool,
    },

    /// Show tool status
    Status {
        /// Tool name
        name: String,
    },

    /// Uninstall a tool
    Uninstall {
        /// Tool name
        name: String,
    },

    /// Check for tool updates
    ///
    /// Checks installed tools against their sources to find newer versions.
    /// Supports multiple sources:
    /// - GitHub releases (checks latest release)
    /// - crates.io (checks latest version)
    /// - npm (checks latest version)
    /// - Go modules (checks latest version)
    ///
    /// Examples:
    ///   bossa tools outdated              # Check all installed tools
    ///   bossa tools outdated rg fd bat    # Check specific tools
    ///   bossa tools outdated --json       # Output as JSON
    Outdated {
        /// Only check specific tools (if empty, checks all)
        #[arg(value_name = "TOOL")]
        tools: Vec<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

// ============================================================================
// Stow Commands (Dotfile Management)
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum StowCommand {
    /// Show status of dotfile symlinks
    ///
    /// Displays which symlinks are:
    /// - Correct (pointing to the right place)
    /// - Missing (not yet created)
    /// - Wrong (pointing to wrong target)
    /// - Blocked (regular file exists)
    Status,

    /// Create/update dotfile symlinks
    ///
    /// Walks each package directory in your dotfiles source and
    /// creates symlinks in the target directory, preserving structure.
    ///
    /// Examples:
    ///   bossa stow sync              # Sync all configured packages
    ///   bossa stow sync zsh git      # Sync only zsh and git packages
    ///   bossa stow sync --dry-run    # Preview what would be done
    Sync {
        /// Only sync specific packages (if empty, syncs all)
        #[arg(value_name = "PACKAGE")]
        packages: Vec<String>,

        /// Preview changes without making them
        #[arg(long, short = 'n')]
        dry_run: bool,

        /// Force overwrite existing files (dangerous!)
        #[arg(long, short)]
        force: bool,
    },

    /// Preview what sync would do (alias for sync --dry-run)
    Diff {
        /// Only diff specific packages
        #[arg(value_name = "PACKAGE")]
        packages: Vec<String>,
    },

    /// List configured packages
    List,

    /// Add a package to the symlinks config
    Add {
        /// Package name (directory in source)
        package: String,
    },

    /// Remove a package from the symlinks config
    Rm {
        /// Package name
        package: String,

        /// Also delete the symlinks
        #[arg(long)]
        unlink: bool,
    },

    /// Remove all symlinks for a package (opposite of sync)
    Unlink {
        /// Package name (if empty, unlinks all)
        #[arg(value_name = "PACKAGE")]
        packages: Vec<String>,

        /// Preview changes without making them
        #[arg(long, short = 'n')]
        dry_run: bool,
    },

    /// Initialize symlinks config with auto-detected packages
    Init {
        /// Source directory (defaults to ~/dotfiles)
        #[arg(long, short)]
        source: Option<String>,

        /// Target directory (defaults to ~)
        #[arg(long, short)]
        target: Option<String>,

        /// Overwrite existing config
        #[arg(long)]
        force: bool,
    },
}

// ============================================================================
// Theme Commands (GNOME/GTK)
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum ThemeCommand {
    /// List available theme presets
    List,

    /// Show current theme status
    ///
    /// Displays the current GTK, shell, icons, cursor, and terminal themes
    /// along with which preset (if any) matches.
    Status,

    /// Apply a theme preset
    ///
    /// Sets GTK theme, GNOME Shell theme, icons, cursor, and optionally
    /// window button layout based on the preset definition.
    ///
    /// Examples:
    ///   bossa theme apply whitesur        # Apply WhiteSur dark theme
    ///   bossa theme apply whitesur-light  # Apply WhiteSur light theme
    ///   bossa theme apply --dry-run       # Preview what would change
    Apply {
        /// Theme preset name
        name: String,

        /// Preview changes without applying
        #[arg(long, short = 'n')]
        dry_run: bool,
    },

    /// Show details of a theme preset
    Show {
        /// Theme preset name
        name: String,
    },
}

// ============================================================================
// Defaults Commands
// ============================================================================

#[derive(Debug, Subcommand)]
pub enum DefaultsCommand {
    /// Set a macOS default value
    ///
    /// Sets a value in the macOS defaults system.
    ///
    /// Examples:
    ///   bossa defaults set com.apple.finder AppleShowAllFiles true
    ///   bossa defaults set NSGlobalDomain KeyRepeat -int 2
    Set {
        /// Domain (e.g., com.apple.finder, NSGlobalDomain)
        domain: String,

        /// Key (e.g., AppleShowAllFiles)
        key: String,

        /// Value
        value: String,

        /// Type (string, bool, int, float) - auto-detected if not provided
        #[arg(short = 't', long = "type")]
        r#type: Option<DefaultsType>,
    },

    /// Read a macOS default value
    Read {
        /// Domain (e.g., com.apple.finder)
        domain: String,

        /// Key (optional, reads entire domain if omitted)
        key: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DefaultsType {
    String,
    Bool,
    Int,
    Float,
}

// ============================================================================
// Locations Commands
// ============================================================================

/// Manage logical locations (path aliases)
#[derive(Debug, Subcommand)]
pub enum LocationsCommand {
    /// List all configured locations
    List,

    /// Add a new location
    Add {
        /// Location name (e.g., "dev", "workspaces")
        name: String,

        /// Path for this location
        path: String,
    },

    /// Remove a location
    Remove {
        /// Location name to remove
        name: String,
    },

    /// Show the resolved path for a location
    Show {
        /// Location name
        name: String,
    },

    /// Add an alias for a location
    Alias {
        /// Path pattern to alias (e.g., "~/dev")
        path: String,

        /// Location name this should resolve to
        location: String,
    },
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
