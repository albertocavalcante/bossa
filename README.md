# bossa

Unified CLI for managing your development environment.

## Features

- **stow** - Native dotfile symlink management (GNU stow replacement)
- **tools** - Install and manage dev tools from multiple sources (HTTP, container, GitHub releases, cargo, npm)
- **theme** - GNOME/GTK theme presets (Linux only)
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
# Install nightly version (recommended for latest features)
BOSSA_VERSION=nightly curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash

# Install specific version
BOSSA_VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash

# Install to custom directory
BOSSA_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.sh | bash
```

### Quick Install (Windows PowerShell)

```powershell
irm https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.ps1 | iex
```

Or with options:

```powershell
# Install specific version
$env:BOSSA_VERSION = "v0.1.0"; irm https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.ps1 | iex

# Install to custom directory
$env:BOSSA_DIR = "C:\Tools\bossa"; irm https://raw.githubusercontent.com/albertocavalcante/bossa/main/tools/scripts/install.ps1 | iex
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

### Stow (Dotfile Management)

Native replacement for GNU stow, designed for dotfile management:

```bash
# Show status of all packages
bossa stow status

# Preview what would be synced
bossa stow diff

# Create/update symlinks
bossa stow sync                  # Sync all packages
bossa stow sync zsh git          # Sync specific packages
bossa stow sync --dry-run        # Preview only

# Manage packages
bossa stow list                  # List configured packages
bossa stow add nvim              # Add package to config
bossa stow rm nvim               # Remove package from config
bossa stow rm nvim --unlink      # Remove and delete symlinks

# Remove symlinks
bossa stow unlink                # Unlink all packages
bossa stow unlink zsh            # Unlink specific package

# Initialize config from dotfiles directory
bossa stow init                  # Auto-detect from ~/dotfiles
bossa stow init --source ~/dots  # Specify source directory
```

Configure in `config.toml`:

```toml
[symlinks]
source = "~/dotfiles"
target = "~"
packages = ["zsh", "git", "nvim", "tmux"]
ignore = [".git", ".github", "README.md"]
```

### Tools Management

Install and manage development tools from multiple sources:

```bash
# Apply tools from config
bossa tools apply                # Install all configured tools
bossa tools apply rg fd          # Install specific tools
bossa tools apply --dry-run      # Preview only

# Check for updates
bossa tools outdated             # Check all installed tools
bossa tools outdated rg fd       # Check specific tools
bossa tools outdated --json      # Output as JSON

# List and inspect
bossa tools list                 # Show installed tools
bossa tools list --all           # Include uninstalled from config
bossa tools status rg            # Show details for a tool

# Imperative installation
bossa tools install mytool --url https://example.com/tool.tar.gz
bossa tools uninstall mytool
```

Configure in `config.toml`:

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

[tools.delta]
source = "github-release"
repo = "dandavison/delta"
version = "0.18.2"

# npm/pnpm global packages (prefers pnpm, falls back to npm)
[tools.pnpm]
source = "npm"
version = "9.15.0"

[tools.bun]
source = "npm"
version = "1.3.5"
depends = ["pnpm"] # Install pnpm first
needs_scripts = true # Required for postinstall scripts
```

### Tool Dependencies

Tools can declare dependencies on other tools using the `depends` field. Bossa will automatically install dependencies first using topological sort:

```toml
# npm → pnpm → bun (dependency chain)
[tools.pnpm]
source = "npm"

[tools.bun]
source = "npm"
depends = ["pnpm"] # pnpm will be installed before bun
```

Supported sources:

- `github-release` - Download from GitHub releases
- `cargo` - Install via cargo (crates.io or git)
- `npm` - Install via pnpm (preferred) or npm globally
- `http` - Download from any HTTP URL
- `container` - Extract from container images

### Theme Management (Linux)

Apply GNOME/GTK theme presets on Linux:

```bash
# Show current theme status
bossa theme status

# List available presets
bossa theme list

# Apply a theme preset
bossa theme apply whitesur        # Apply WhiteSur dark theme
bossa theme apply --dry-run       # Preview changes

# Show preset details
bossa theme show whitesur
```

Configure theme presets in `config.toml`:

```toml
[themes.whitesur]
description = "macOS Big Sur style (dark)"
gtk = "WhiteSur-Dark"
shell = "WhiteSur-Dark"
wm = "WhiteSur-Dark"
wm_buttons = "close,minimize,maximize:" # macOS-style left buttons
icons = "WhiteSur-dark"
cursor = "WhiteSur-cursors"
terminal = "whitesur"
requires = ["whitesur-gtk", "whitesur-icons"] # Tools to install first

[themes.whitesur-light]
description = "macOS Big Sur style (light)"
gtk = "WhiteSur-Light"
shell = "WhiteSur-Light"
wm = "WhiteSur-Light"
wm_buttons = "close,minimize,maximize:"
icons = "WhiteSur"
cursor = "WhiteSur-cursors"
```

Theme fields:

- `gtk` - GTK theme (apps like Nautilus)
- `shell` - GNOME Shell theme (panel, overview)
- `wm` - Window manager theme (title bars)
- `wm_buttons` - Button layout (`close,minimize,maximize:` = left/macOS style)
- `icons` - Icon theme
- `cursor` - Cursor theme
- `terminal` - Terminal color scheme (informational)
- `requires` - Tools that must be installed first

## Configuration

Bossa reads configuration from a platform-specific config directory:

| Platform    | Default Location   |
| ----------- | ------------------ |
| Linux/macOS | `~/.config/bossa/` |
| Windows     | `%APPDATA%\bossa\` |

Config files:

- `config.toml` - Main configuration
- `tools.toml` - Installed tools tracking
- `caches.toml` - Cache symlinks configuration

TOML format is preferred when both formats exist. Use `bossa config convert` to switch formats:

```bash
# Show current config files
bossa config show

# Convert to TOML
bossa config convert all --format toml

# Validate configs
bossa config validate
```

## Environment Variables

Bossa supports environment variable overrides for path configuration, making it easy to symlink configs from a dotfiles repository.

| Variable               | Description                        | Default                             |
| ---------------------- | ---------------------------------- | ----------------------------------- |
| `BOSSA_CONFIG_DIR`     | Override config directory          | `~/.config/bossa` (see table above) |
| `BOSSA_STATE_DIR`      | Override state directory           | `~/.local/state/bossa`              |
| `BOSSA_WORKSPACES_DIR` | Override workspaces root directory | `~/dev/ws`                          |

### Path Resolution Priority

For the config directory, bossa checks in order:

1. `BOSSA_CONFIG_DIR` environment variable
2. Existing `~/.config/bossa/` (backwards compatibility)
3. `XDG_CONFIG_HOME/bossa` (if XDG_CONFIG_HOME is set)
4. Platform default (see table above)

### Dotfiles Integration Example

```bash
# In your shell profile (~/.bashrc, ~/.zshrc, etc.)
export BOSSA_CONFIG_DIR="$HOME/dotfiles/bossa"

# Or symlink the config directory
ln -s ~/dotfiles/bossa ~/.config/bossa
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
| stow        | Dotfile symlinks via bossa stow         |
| mcp         | MCP server configuration                |
| refs        | Reference repositories                  |
| workspaces  | Developer workspaces                    |

## License

MIT
