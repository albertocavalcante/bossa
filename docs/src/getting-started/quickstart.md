# Quick Start

This guide gets you up and running with bossa in minutes.

## Prerequisites

- macOS or Linux
- Git installed
- Homebrew (optional, for `bossa brew` commands)

## First Steps

### 1. Check System Status

See what bossa can manage on your system:

```bash
bossa status
```

This shows an overview of:

- Homebrew packages (installed vs desired)
- Repository collections
- Storage locations

### 2. Run Health Checks

Verify your system is properly configured:

```bash
bossa doctor
```

The doctor command checks:

- Git configuration
- SSH keys
- Homebrew installation
- Required tools

## Common Workflows

### Managing Homebrew Packages

```bash
# See what would change
bossa brew audit

# Apply your Brewfile
bossa brew apply

# Capture currently installed packages
bossa brew capture
```

### Managing Repository Collections

Collections are groups of git repositories that you want to keep in sync.

```bash
# List your collections
bossa collections list

# Check collection status
bossa collections status refs

# Clone missing repositories
bossa collections sync refs

# Add a new repo to a collection
bossa collections add refs https://github.com/user/repo.git
```

### Bootstrapping a New Machine

The `nova` command runs through 16 stages to set up a new machine:

```bash
# See available stages
bossa nova --list-stages

# Run all stages
bossa nova

# Run specific stages only
bossa nova --only=homebrew,brew,stow

# Skip certain stages
bossa nova --skip=dock,handlers

# Dry run to see what would happen
bossa nova --dry-run
```

### Storage Management

Get a unified view of your storage:

```bash
# Overview of all storage locations
bossa storage status

# Find duplicates across locations
bossa storage duplicates
```

### iCloud Management

Control what's downloaded from iCloud Drive:

```bash
# See iCloud status
bossa icloud status

# Download files
bossa icloud download ~/Library/Mobile\ Documents/

# Evict files to free local space
bossa icloud evict ~/Library/Mobile\ Documents/some-folder/
```

## Configuration

Bossa reads configuration from `~/.config/bossa/`:

```
~/.config/bossa/
├── config.toml    # Unified config (collections, workspaces, storage, packages)
└── caches.toml    # Cache mappings (created by `bossa caches init`)
```

Homebrew uses a Brewfile at `~/dotfiles/Brewfile` by default (override with `bossa brew apply --file <path>`).

See [Configuration](../guide/configuration.md) for details.

## Next Steps

- [Configuration](../guide/configuration.md) - Learn about config files
- [Nova Bootstrap](../guide/nova.md) - Deep dive into machine setup
- [CLI Reference](../reference/cli.md) - All commands and options
