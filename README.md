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
- **config** - Manage configuration files (supports JSON and TOML)

## Installation

### Quick Install (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash
```

Or with options:

```bash
# Install specific version
BOSSA_VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash

# Install to custom directory
BOSSA_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash
```

### Homebrew (macOS)

```bash
brew install albertocavalcante/tap/bossa
```

> **Note:** Use the full tap path to avoid conflict with homebrew-core's `bossa` (a flash programmer).

### Download Binary

Download the latest release for your platform from [GitHub Releases](https://github.com/albertocavalcante/bossa/releases).

| Platform            | Asset                        |
| ------------------- | ---------------------------- |
| Linux x64           | `bossa-linux-amd64.tar.gz`   |
| Linux ARM64         | `bossa-linux-aarch64.tar.gz` |
| macOS Apple Silicon | `bossa-darwin-arm64.tar.gz`  |
| macOS Intel         | `bossa-darwin-amd64.tar.gz`  |
| Windows x64         | `bossa-windows-amd64.zip`    |

### From Source

```bash
# Using Cargo
cargo install --path .

# Using Bazel
bazel run //:install

# Using just
just install
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

- `refs.toml` or `refs.json` - Reference repositories
- `workspaces.toml` or `workspaces.json` - Workspace definitions

TOML format is preferred when both formats exist. Use `bossa config convert` to switch formats:

```bash
# Show current config files
bossa config show

# Convert to TOML
bossa config convert all --format toml

# Validate configs
bossa config validate
```

## Global Flags

```
-v, --verbose    Increase verbosity (can be repeated: -vv, -vvv)
-q, --quiet      Suppress non-essential output
```

## Nova Stages

The `nova` command bootstraps a new machine with 15 stages:

| Stage       | Description                             |
| ----------- | --------------------------------------- |
| defaults    | macOS system defaults                   |
| terminal    | Terminal font setup                     |
| git-signing | Git signing key setup                   |
| homebrew    | Homebrew installation                   |
| bash        | Bash 4+ bootstrap                       |
| essential   | Essential packages (stow, jq, gh, etc.) |
| brew        | Full Brewfile packages                  |
| pnpm        | Node packages via pnpm                  |
| dock        | Dock configuration                      |
| ecosystem   | Ecosystem extensions                    |
| handlers    | File handlers (duti)                    |
| stow        | Symlinks via GNU Stow                   |
| mcp         | MCP server configuration                |
| refs        | Reference repositories                  |
| workspaces  | Developer workspaces                    |

## License

MIT
