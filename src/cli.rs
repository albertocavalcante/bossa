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
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Show status dashboard of all systems
    Status,

    /// Sync everything (workspaces, refs, packages)
    Sync(SyncArgs),

    /// Manage reference repositories (~/dev/refs)
    #[command(subcommand)]
    Refs(RefsCommand),

    /// Manage Homebrew packages
    #[command(subcommand)]
    Brew(BrewCommand),

    /// Manage workspaces (bare repos + worktrees)
    #[command(subcommand)]
    Workspace(WorkspaceCommand),

    /// Manage git worktrees (worker pool model)
    #[command(subcommand)]
    Worktree(WorktreeCommand),

    /// Manage T9 external drive (exFAT repos)
    #[command(subcommand)]
    T9(T9Command),

    /// Run health checks on all systems
    Doctor,

    /// Bootstrap a new machine from scratch (like install.sh)
    Nova(NovaArgs),

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Manage configuration files
    #[command(subcommand)]
    Config(ConfigCommand),
}

// ============================================================================
// Config Commands
// ============================================================================

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Show config file locations and formats
    Show,

    /// Convert config files between formats
    Convert {
        /// Config to convert: refs, workspaces, or all
        #[arg(value_enum)]
        config: ConfigTarget,

        /// Target format
        #[arg(short, long, value_enum)]
        format: ConfigFormatArg,

        /// Keep original file after conversion
        #[arg(short, long)]
        keep: bool,
    },

    /// Validate config files
    Validate,

    /// Open config directory
    Dir,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ConfigTarget {
    Refs,
    Workspaces,
    All,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ConfigFormatArg {
    Json,
    Toml,
}

// ============================================================================
// Sync
// ============================================================================

#[derive(Parser)]
pub struct SyncArgs {
    /// Only sync specific targets (comma-separated): refs, workspace, brew
    #[arg(short, long)]
    pub only: Option<String>,

    /// Dry run - show what would be done
    #[arg(short, long)]
    pub dry_run: bool,

    /// Number of parallel jobs for cloning
    #[arg(short, long, default_value = "4")]
    pub jobs: usize,
}

// ============================================================================
// Refs Commands
// ============================================================================

#[derive(Subcommand)]
pub enum RefsCommand {
    /// Clone all missing repos from refs.json
    Sync(RefsSyncArgs),

    /// List all configured reference repos
    List {
        /// Filter by name pattern
        #[arg(short, long)]
        filter: Option<String>,

        /// Show only missing repos
        #[arg(long)]
        missing: bool,
    },

    /// Capture current ~/dev/refs state to refs.json
    Snapshot,

    /// Detect repos not tracked in refs.json (drift detection)
    Audit {
        /// Automatically add untracked repos to config
        #[arg(long)]
        fix: bool,
    },

    /// Add a new repo to refs.json (and optionally clone it)
    Add {
        /// Git URL to add
        url: String,

        /// Custom name (defaults to repo name from URL)
        #[arg(short, long)]
        name: Option<String>,

        /// Clone immediately after adding
        #[arg(short, long)]
        clone: bool,
    },

    /// Remove a repo from refs.json
    Remove {
        /// Repo name to remove
        name: String,

        /// Also delete the local clone
        #[arg(short, long)]
        delete: bool,
    },
}

#[derive(Parser)]
pub struct RefsSyncArgs {
    /// Specific repo name to sync
    pub name: Option<String>,

    /// Number of parallel clone jobs
    #[arg(short, long, default_value = "4")]
    pub jobs: usize,

    /// Number of retry attempts for failed clones
    #[arg(short, long, default_value = "3")]
    pub retries: usize,

    /// Dry run - show what would be cloned
    #[arg(long)]
    pub dry_run: bool,
}

// ============================================================================
// Brew Commands
// ============================================================================

#[derive(Subcommand)]
pub enum BrewCommand {
    /// Install all packages from Brewfile
    Apply {
        /// Install only essential packages
        #[arg(short, long)]
        essential: bool,
    },

    /// Capture current packages to Brewfile
    Capture,

    /// Detect packages not in Brewfile (drift detection)
    Audit,
}

// ============================================================================
// Workspace Commands
// ============================================================================

#[derive(Subcommand)]
pub enum WorkspaceCommand {
    /// Sync workspaces from config
    Sync {
        /// Specific workspace to sync
        target: Option<String>,
    },

    /// List configured workspaces
    List,
}

// ============================================================================
// Worktree Commands (wt wrapper)
// ============================================================================

#[derive(Subcommand)]
pub enum WorktreeCommand {
    /// Show worktree status (worker pool)
    Status,

    /// Create new worktree in available slot
    New {
        /// Branch name
        branch: String,

        /// Specific slot (w1-w10)
        #[arg(short, long)]
        slot: Option<String>,

        /// Push branch to remote
        #[arg(short, long)]
        push: bool,
    },

    /// Release a worktree slot
    Release {
        /// Slot to release (w1-w10)
        slot: String,

        /// Force release even if dirty
        #[arg(short, long)]
        force: bool,
    },

    /// Clean up merged worktrees
    Cleanup {
        /// Force cleanup
        #[arg(short, long)]
        force: bool,

        /// Dry run
        #[arg(short, long)]
        dry_run: bool,
    },
}

// ============================================================================
// T9 Commands
// ============================================================================

#[derive(Subcommand)]
pub enum T9Command {
    /// Check git status across all T9 repos
    Status,

    /// Configure repos for exFAT (fileMode=false)
    Config,

    /// Clean metadata files (._ and .DS_Store)
    Clean,

    /// Show T9 statistics
    Stats,

    /// Verify T9 mount and symlinks
    Verify,
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
