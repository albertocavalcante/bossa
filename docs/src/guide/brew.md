# Homebrew Management

Bossa provides declarative Homebrew package management with drift detection.

## Commands

```bash
bossa brew apply     # Install packages from Brewfile
bossa brew capture   # Update Brewfile with installed packages
bossa brew audit     # Detect drift between installed and desired
bossa brew list      # List installed packages
```

## Workflow

### 1. Capture Current State

Start by capturing your currently installed packages:

```bash
bossa brew capture
```

This updates `~/dotfiles/Brewfile` with all installed:

- Formulae
- Casks
- Taps
- Mac App Store apps

### 2. Apply Desired State

Install packages defined in your Brewfile:

```bash
bossa brew apply
```

This runs `brew bundle` and installs missing packages.

### 3. Detect Drift

Check if installed packages match your Brewfile:

```bash
bossa brew audit
```

Output shows:

- **Missing**: In Brewfile but not installed
- **Untracked**: Installed but not in Brewfile
- **Version mismatches**: Installed version differs from Brewfile

## Brewfile Format

Standard Homebrew bundle format:

```ruby
# Taps - third-party repositories
tap "homebrew/bundle"
tap "homebrew/services"
tap "albertocavalcante/tap"

# Formulae - command-line tools
brew "git"
brew "ripgrep"
brew "fd"
brew "jq"
brew "gh"
brew "neovim"

# Casks - GUI applications
cask "visual-studio-code"
cask "iterm2"
cask "docker"
cask "1password"

# Mac App Store apps (requires `mas` CLI)
mas "Xcode", id: 497799835
mas "Slack", id: 803453959
```

## Options

### Apply Options

```bash
# Only install essentials (no casks/mas/vscode)
bossa brew apply --essential

# Dry run
bossa brew apply --dry-run

# Use a specific Brewfile
bossa brew apply --file ~/path/to/Brewfile
```

### Capture Options

```bash
# Write to a specific path
bossa brew capture --output ~/dotfiles/Brewfile
```

### List Options

```bash
# Filter by type: tap, brew, cask, mas, vscode
bossa brew list --type cask
```

## Best Practices

### 1. Organize Your Brewfile

Group packages by category with comments:

```ruby
# === Development ===
brew "git"
brew "gh"
brew "neovim"

# === Languages ===
brew "rust"
brew "go"
brew "python@3.12"

# === Utilities ===
brew "ripgrep"
brew "fd"
brew "jq"
brew "yq"

# === Applications ===
cask "visual-studio-code"
cask "iterm2"
```

### 2. Version Control

Keep your Brewfile in version control (default is `~/dotfiles/Brewfile`):

```bash
cd ~/dotfiles
git add Brewfile
git commit -m "Update Brewfile"
```

### 3. Regular Audits

Run audits periodically to catch drift:

```bash
# Add to crontab or run weekly
bossa brew audit
```

### 4. Capture After Installing

After installing new packages manually, capture the change:

```bash
brew install something-new
bossa brew capture
```

## Troubleshooting

### Package Not Found

If `brew apply` fails to find a package:

1. Check the tap is included in Brewfile
2. Run `brew update`
3. Verify package name: `brew search <name>`

### Cask Already Installed

Casks installed outside Homebrew aren't tracked. Either:

- Uninstall and reinstall via `brew install --cask`
- Add to Brewfile anyway (Homebrew will adopt it)

### Mac App Store Apps

Requires `mas` CLI:

```bash
brew install mas
mas signin # Sign in to App Store
bossa brew apply
```
