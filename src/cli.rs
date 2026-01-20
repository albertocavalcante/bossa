#![allow(dead_code)]
#![allow(clippy::inherent_to_string)]

use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

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

    /// Number of parallel jobs
    #[arg(short, long, default_value = "4")]
    pub jobs: usize,
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

    /// Number of parallel jobs
    #[arg(short, long)]
    pub jobs: Option<usize>,
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
        match target.split_once('.') {
            Some((resource_type, name)) => Self {
                resource_type: resource_type.to_string(),
                name: Some(name.to_string()),
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

    /// Get the full target string (e.g., "collections.refs")
    pub fn to_string(&self) -> String {
        match &self.name {
            Some(name) => format!("{}.{}", self.resource_type, name),
            None => self.resource_type.clone(),
        }
    }
}
