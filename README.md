<div align="center">

# bossa

**Unified CLI for managing your development environment.**

Bootstrap a new machine, manage dotfiles, install tools, sync repos — one command.

[![CI](https://github.com/albertocavalcante/bossa/actions/workflows/ci.yml/badge.svg)](https://github.com/albertocavalcante/bossa/actions/workflows/ci.yml)
[![Release](https://github.com/albertocavalcante/bossa/actions/workflows/release.yml/badge.svg)](https://github.com/albertocavalcante/bossa/actions/workflows/release.yml)
[![Nightly](https://github.com/albertocavalcante/bossa/actions/workflows/nightly.yml/badge.svg)](https://github.com/albertocavalcante/bossa/actions/workflows/nightly.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](https://www.rust-lang.org)

[Installation](#installation) · [Quick Start](#quick-start) · [Commands](#commands) · [Configuration](#configuration) · [Nova Stages](#nova-stages)

</div>

---

## Why Bossa?

Setting up a new dev machine means juggling dozens of tools: Homebrew, dotfile managers, git clones, system preferences, shell configs. Bossa replaces that patchwork with a single declarative CLI.

```bash
bossa nova        # bootstrap everything — from Homebrew to dotfiles to repos
bossa doctor      # verify your setup is healthy
bossa status      # see what's managed at a glance
```

## Installation

### Homebrew (macOS)

```bash
brew install albertocavalcante/tap/bossa
```

> Use the full tap path to avoid conflict with homebrew-core's `bossa` (a flash programmer).

### Quick Install (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash
```

Options:

```bash
# Nightly (latest features)
curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash -s -- nightly

# Specific version
curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash -s -- v0.1.0

# Custom directory
BOSSA_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash
```

### Quick Install (Windows PowerShell)

```powershell
irm https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.ps1 | iex
```

### Download Binary

Grab the latest release for your platform from [GitHub Releases](https://github.com/albertocavalcante/bossa/releases).

| Platform            | Asset                        |
| ------------------- | ---------------------------- |
| Linux x64           | `bossa-linux-amd64.tar.gz`   |
| Linux ARM64         | `bossa-linux-aarch64.tar.gz` |
| macOS Apple Silicon | `bossa-darwin-arm64.tar.gz`  |
| macOS Intel         | `bossa-darwin-amd64.tar.gz`  |
| Windows x64         | `bossa-windows-amd64.zip`    |

### From Source

```bash
cargo install --path .       # Cargo
bazel run //:install          # Bazel
just install                  # just
```

## Quick Start

```bash
# 1. Check system health
bossa doctor

# 2. See what bossa manages
bossa status

# 3. Bootstrap a new machine (runs all 16 stages)
bossa nova

# 4. Or pick specific stages
bossa nova --only=homebrew,brew,stow
```

## Commands

| Command       | Description                                                  |
| ------------- | ------------------------------------------------------------ |
| `nova`        | Full machine bootstrap (16 stages)                           |
| `doctor`      | Health checks for all managed systems                        |
| `status`      | Dashboard overview                                           |
| `sync`        | Sync workspaces + reference repos                            |
| `stow`        | Dotfile symlink management (GNU stow replacement)            |
| `tools`       | Install/manage dev tools (GitHub releases, cargo, npm, HTTP) |
| `brew`        | Homebrew package management (apply, capture, audit)          |
| `refs`        | Reference repository management with parallel cloning        |
| `workspace`   | Workspace management (bare repos + worktrees)                |
| `worktree`    | Git worktree worker pool model                               |
| `theme`       | GNOME/GTK theme presets (Linux)                              |
| `t9`          | External drive management for exFAT repos                    |
| `config`      | Manage configuration files (JSON / TOML)                     |
| `completions` | Shell completions (bash / zsh / fish / powershell)           |

### Stow (Dotfile Management)

Native replacement for GNU stow:

```bash
bossa stow status                # Show status of all packages
bossa stow diff                  # Preview what would be synced
bossa stow sync                  # Create/update all symlinks
bossa stow sync zsh git          # Sync specific packages
bossa stow sync --dry-run        # Preview only

bossa stow add nvim              # Add package to config
bossa stow rm nvim --unlink      # Remove package + delete symlinks

bossa stow init                  # Auto-detect from ~/dotfiles
bossa stow init --source ~/dots  # Specify source directory
```

Config (`config.toml`):

```toml
[symlinks]
source = "~/dotfiles"
target = "~"
packages = ["zsh", "git", "nvim", "tmux"]
ignore = [".git", ".github", "README.md"]
```

### Tools Management

Install and manage dev tools from multiple sources:

```bash
bossa tools apply                # Install all configured tools
bossa tools apply rg fd          # Install specific tools
bossa tools outdated             # Check for updates
bossa tools list --all           # Show all (installed + configured)
```

Config (`config.toml`):

```toml
[tools]
install_dir = "~/.local/bin"

[tools.rg]
source = "github-release"
repo = "BurntSushi/ripgrep"
version = "14.1.0"
asset = "ripgrep-{version}-{arch}-{os}.tar.gz"

[tools.fd]
source = "cargo"
crate = "fd-find"

[tools.pnpm]
source = "npm"
version = "9.15.0"

[tools.bun]
source = "npm"
depends = ["pnpm"] # Dependency chain — installed first via topological sort
```

Supported sources: `github-release` | `cargo` | `npm` | `http` | `container`

### Brew Management

```bash
bossa brew apply       # Install from Brewfile
bossa brew capture     # Update Brewfile with current packages
bossa brew audit       # Detect drift between Brewfile and system
```

## Configuration

Bossa reads from a platform-specific config directory:

| Platform    | Default Location   |
| ----------- | ------------------ |
| Linux/macOS | `~/.config/bossa/` |
| Windows     | `%APPDATA%\bossa\` |

Files:

- `config.toml` — main configuration (collections, workspaces, tools, symlinks)
- `tools.toml` — installed tools tracking
- `caches.toml` — cache symlinks configuration

```bash
bossa config show                        # Show current config files
bossa config convert all --format toml   # Convert to TOML
bossa config validate                    # Validate configs
```

### Environment Variables

| Variable               | Description               | Default                |
| ---------------------- | ------------------------- | ---------------------- |
| `BOSSA_CONFIG_DIR`     | Override config directory | `~/.config/bossa`      |
| `BOSSA_STATE_DIR`      | Override state directory  | `~/.local/state/bossa` |
| `BOSSA_WORKSPACES_DIR` | Override workspaces root  | `~/dev/ws`             |

Resolution order: env var > existing `~/.config/bossa/` > `XDG_CONFIG_HOME/bossa` > platform default.

### Dotfiles Integration

```bash
# In your shell profile
export BOSSA_CONFIG_DIR="$HOME/dotfiles/bossa"

# Or symlink
ln -s ~/dotfiles/bossa ~/.config/bossa
```

## Nova Stages

`bossa nova` bootstraps a new machine through 16 idempotent stages:

```bash
bossa nova                       # Run all stages
bossa nova --list-stages         # Show available stages
bossa nova --only=stow,refs      # Run specific stages
bossa nova --skip=dock           # Skip stages
bossa nova --dry-run             # Preview
bossa nova -j 4                  # Parallel execution
```

| Stage         | Description                               |
| ------------- | ----------------------------------------- |
| `defaults`    | macOS system defaults (Finder, Dock, etc) |
| `terminal`    | Terminal font setup                       |
| `git-signing` | Git commit signing key                    |
| `homebrew`    | Homebrew installation                     |
| `bash`        | Bash 4+ bootstrap                         |
| `essential`   | Core packages (stow, jq, gh, rg, fd)      |
| `brew`        | Full Brewfile packages                    |
| `pnpm`        | Node packages via pnpm                    |
| `dock`        | Dock configuration                        |
| `ecosystem`   | Ecosystem extensions (VS Code, etc)       |
| `handlers`    | File type handlers (duti)                 |
| `stow`        | Dotfile symlinks                          |
| `caches`      | Cache symlinks to external drive          |
| `mcp`         | MCP server configuration                  |
| `refs`        | Reference repositories                    |
| `workspaces`  | Developer workspaces                      |

All stages are idempotent — re-running is always safe.

## Global Flags

```
-v, --verbose    Increase verbosity (-vv, -vvv)
-q, --quiet      Suppress non-essential output
```

## Shell Completions

```bash
bossa completions bash >> ~/.bashrc
bossa completions zsh >> ~/.zshrc
bossa completions fish > ~/.config/fish/completions/bossa.fish
```

## License

MIT
