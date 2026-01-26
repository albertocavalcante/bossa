# Machine Bootstrap (Nova)

The `nova` command bootstraps a new machine through 16 stages. The name comes from "bossa nova" - the Brazilian music genre.

## Usage

```bash
# Run all stages
bossa nova

# List available stages
bossa nova --list-stages

# Run specific stages only
bossa nova --only=homebrew,brew,stow

# Skip certain stages
bossa nova --skip=dock,handlers

# Dry run
bossa nova --dry-run

# Parallel execution
bossa nova -j 4
```

## Stages

| Stage         | Description                                             |
| ------------- | ------------------------------------------------------- |
| `defaults`    | macOS system defaults (Finder, Dock, keyboard settings) |
| `terminal`    | Terminal font setup                                     |
| `git-signing` | Git commit signing key configuration                    |
| `homebrew`    | Homebrew installation                                   |
| `bash`        | Bash 4+ bootstrap (macOS ships with Bash 3)             |
| `essential`   | Essential packages (stow, jq, gh, ripgrep, fd)          |
| `brew`        | Full Brewfile installation                              |
| `pnpm`        | Node.js packages via pnpm                               |
| `dock`        | Dock configuration (apps, size, position)               |
| `ecosystem`   | Ecosystem extensions (VS Code, etc.)                    |
| `handlers`    | File type handlers via duti                             |
| `stow`        | Symlinks via GNU Stow                                   |
| `caches`      | Cache symlinks to external drive                        |
| `mcp`         | MCP server configuration                                |
| `refs`        | Reference repository collections                        |
| `workspaces`  | Development workspaces                                  |

## Stage Details

### defaults

Configures macOS system preferences:

- Finder: Show hidden files, path bar, status bar
- Dock: Size, magnification, position
- Keyboard: Key repeat rate, disable auto-correct
- Screenshots: Location, format

### homebrew

Installs Homebrew if not present:

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

### essential

Installs core utilities needed by other stages:

- `stow` - Symlink manager for dotfiles
- `jq` - JSON processor
- `gh` - GitHub CLI
- `ripgrep` - Fast search
- `fd` - Fast find

### brew

Runs `brew bundle` with your Brewfile:

```bash
brew bundle --file=~/dotfiles/Brewfile
```

### stow

Symlinks dotfiles from your dotfiles repository:

```bash
cd ~/dotfiles
stow -t ~ bash zsh git vim
```

### caches

Applies cache symlink configuration:

```bash
bossa caches apply
```

### refs

Clones repository collections defined in `config.toml`:

```bash
bossa collections sync refs
```

## Customizing Stages

### Running Specific Stages

```bash
# Only install Homebrew and essential tools
bossa nova --only=homebrew,essential

# Skip dock configuration
bossa nova --skip=dock
```

### Stage Dependencies

Some stages depend on others:

- `brew` requires `homebrew`
- `stow` requires `essential` (for stow binary)
- `refs` requires `essential` (for git)

When using `--only`, dependencies are **not** automatically included. Make sure to include required stages.

### Dry Run

Preview what would happen without making changes:

```bash
bossa nova --dry-run
```

## Idempotency

All stages are idempotent - running them multiple times is safe:

- Already installed packages are skipped
- Existing symlinks are preserved
- Configurations are applied only if different

## Parallelism

Use `-j` to run independent stages in parallel:

```bash
bossa nova -j 4
```

Stages with dependencies still run in order.

## Troubleshooting

### Stage Failed

If a stage fails, fix the issue and re-run:

```bash
# Re-run failed stage
bossa nova --only=brew
```

### Reset and Retry

```bash
# Skip problematic stages
bossa nova --skip=dock,handlers
```

### Verbose Output

```bash
bossa nova -vv
```
