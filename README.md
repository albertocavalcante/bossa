# bossa

Unified CLI for managing your development environment.

## Features

- **refs** - Manage reference repositories with parallel cloning and retry logic
- **brew** - Homebrew package management (apply, capture, audit)
- **workspace** - Workspace management (bare repos + worktrees)
- **worktree** - Git worktree worker pool model
- **t9** - External drive management for exFAT repos
- **doctor** - Health checks for all systems
- **nova** - Full machine bootstrap (15 stages)
- **completions** - Shell completions (bash/zsh/fish/powershell)

## Installation

```bash
# From source
cargo install --path .

# Or build manually
cargo build --release
cp target/release/bossa ~/bin/
```

## Usage

```bash
# Show status dashboard
bossa status

# Sync everything (workspaces + refs)
bossa sync

# Manage reference repos
bossa refs sync              # Clone missing repos (parallel)
bossa refs snapshot          # Capture current state to refs.json
bossa refs audit             # Detect drift

# Manage brew packages
bossa brew apply             # Install from Brewfile
bossa brew capture           # Update Brewfile with current packages
bossa brew audit             # Detect drift

# Health check
bossa doctor

# Bootstrap new machine
bossa nova                   # Run all stages
bossa nova --list-stages     # Show available stages
bossa nova --only=stow,refs  # Run specific stages
bossa nova --skip=brew       # Skip specific stages

# Shell completions
bossa completions bash >> ~/.bashrc
bossa completions zsh >> ~/.zshrc
bossa completions fish > ~/.config/fish/completions/bossa.fish
```

## Configuration

Bossa reads configuration from `~/.config/workspace-setup/`:

- `refs.json` - Reference repositories
- `workspaces.json` - Workspace definitions

## Global Flags

```
-v, --verbose    Increase verbosity (can be repeated: -vv, -vvv)
-q, --quiet      Suppress non-essential output
```

## Nova Stages

The `nova` command bootstraps a new machine with 15 stages:

| Stage | Description |
|-------|-------------|
| defaults | macOS system defaults |
| terminal | Terminal font setup |
| git-signing | Git signing key setup |
| homebrew | Homebrew installation |
| bash | Bash 4+ bootstrap |
| essential | Essential packages (stow, jq, gh, etc.) |
| brew | Full Brewfile packages |
| pnpm | Node packages via pnpm |
| dock | Dock configuration |
| ecosystem | Ecosystem extensions |
| handlers | File handlers (duti) |
| stow | Symlinks via GNU Stow |
| mcp | MCP server configuration |
| refs | Reference repositories |
| workspaces | Developer workspaces |

## License

MIT
