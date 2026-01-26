# Configuration

Bossa uses configuration files to define your desired system state.

## Configuration Directory

All configuration lives in `~/.config/bossa/`:

```
~/.config/bossa/
├── config.toml    # Unified config (collections, workspaces, storage, packages)
├── caches.toml    # Cache mappings (created by `bossa caches init`)
└── manifests/     # Manifest databases (auto-generated)
```

## File Format

Bossa uses TOML for `config.toml`. The caches file is also TOML by default (created with `bossa caches init`).

Legacy configs from `~/.config/workspace-setup/` can be migrated with:

```bash
bossa migrate --dry-run
bossa migrate
```

## Repository Collections (config.toml)

Define groups of repositories to manage:

```toml
# Collection name becomes a directory
[collections.refs]
path = "~/dev/refs"

[[collections.refs.repos]]
url = "https://github.com/rust-lang/rust.git"
name = "rust"
name = "rust" # Optional, derived from URL if omitted

[[collections.refs.repos]]
url = "https://github.com/torvalds/linux.git"
name = "linux"

[collections.tools]
path = "~/dev/tools"

[[collections.tools.repos]]
url = "https://github.com/neovim/neovim.git"
name = "neovim"
```

## Workspaces (config.toml)

Define development workspaces:

```toml
[workspaces]
root = "~/dev/ws"
structure = "bare-worktree"

[[workspaces.repos]]
name = "myproject"
url = "https://github.com/user/myproject.git"
category = "work"

[[workspaces.repos]]
name = "another"
url = "git@github.com:user/another.git"
category = "personal"
```

## Brewfile

Standard Homebrew bundle format. By default bossa reads `~/dotfiles/Brewfile` (override with `--file` or `--output`):

```ruby
# Taps
tap "homebrew/bundle"
tap "albertocavalcante/tap"

# Formulae
brew "git"
brew "ripgrep"
brew "fd"
brew "jq"
brew "gh"

# Casks
cask "visual-studio-code"
cask "iterm2"
cask "docker"

# Mac App Store
mas "Xcode", id: 497799835
```

## Cache Mappings (caches.toml)

Map cache directories to external storage:

```toml
external_drive = { name = "T9", mount_point = "/Volumes/T9", base_path = "caches" }

[[symlinks]]
name = "homebrew"
source = "~/Library/Caches/Homebrew"
target = "homebrew"
description = "Homebrew downloads"

[[symlinks]]
name = "cargo-registry"
source = "~/.cargo/registry"
target = "cargo/registry"
```

## Validation

Validate your configuration files:

```bash
# Check all configs
bossa doctor

# Show health checks
bossa doctor
```

## Best Practices

1. **Use TOML** - More readable than JSON, with comments support
2. **Version control** - Keep configs in a dotfiles repo
3. **Keep Brewfile in dotfiles** - Default is `~/dotfiles/Brewfile`
4. **Capture regularly** - Run `bossa brew capture` after installing packages
5. **Audit for drift** - Run `bossa brew audit` periodically
