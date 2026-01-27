# Declarative State

Bossa follows a declarative approach: you define the desired state, and bossa figures out how to achieve it.

## Core Concepts

### Desired State

Configuration files define what your system should look like:

```toml
# config.toml - desired repositories
[collections.refs]
path = "~/dev/refs"

[[collections.refs.repos]]
url = "https://github.com/rust-lang/rust.git"
name = "rust"
```

```ruby
# Brewfile - desired packages
brew "ripgrep"
brew "fd"
brew "jq"
```

### Current State

Bossa inspects your system to determine current state:

- Installed packages
- Cloned repositories
- File states (local vs cloud)

### Reconciliation

Commands compare desired vs current state and take action:

```bash
bossa status   # Show differences
bossa diff     # Preview changes
bossa apply    # Apply changes
```

## The Apply Pattern

### Check → Plan → Apply

1. **Check** - Compare desired vs current
2. **Plan** - Determine actions needed
3. **Apply** - Execute actions

```bash
# Check: What's different?
bossa brew audit

# Plan: What would change?
bossa diff

# Apply: Make it so
bossa apply
```

### Idempotency

All operations are idempotent - running them multiple times produces the same result:

```bash
# First run: installs 5 packages
bossa brew apply
# Output: Installed 5 packages

# Second run: nothing to do
bossa brew apply
# Output: Already in sync
```

## State Operations

### Capture

Capture current state to configuration:

```bash
# Capture installed packages
bossa brew capture

# Capture cloned repos
bossa collections snapshot refs
```

### Audit

Detect drift between desired and current:

```bash
# Package drift
bossa brew audit

# Repository drift
bossa collections audit refs
```

### Sync

Bring current state in line with desired:

```bash
# Sync packages
bossa brew apply

# Sync repositories
bossa collections sync refs
```

## Drift Detection

### What is Drift?

Drift occurs when current state diverges from desired:

- **Package drift**: Manually installed/removed packages
- **Repository drift**: Manually cloned/deleted repos
- **Config drift**: Modified configuration files

### Detecting Drift

```bash
# Overall status
bossa status

# Package drift
bossa brew audit
# Output:
#   Missing: ripgrep (in Brewfile, not installed)
#   Extra: wget (installed, not in Brewfile)

# Repository drift
bossa collections audit refs
# Output:
#   Untracked: ~/dev/refs/some-repo (not in config)
```

### Resolving Drift

Two approaches:

1. **Apply desired state** - Remove extras, add missing
   ```bash
   bossa apply
   ```

2. **Update desired state** - Capture current state
   ```bash
   bossa brew capture
   bossa collections snapshot refs
   ```

## Two-Way Sync

Bossa supports both directions:

### Desired → Current (Apply)

Push configuration to system:

```bash
bossa brew apply        # Install packages
bossa collections sync refs  # Clone repos
```

### Current → Desired (Capture)

Pull system state to configuration:

```bash
bossa brew capture           # Update Brewfile
bossa collections snapshot refs   # Update config.toml
```

## Workflow Examples

### New Machine Setup

1. Clone dotfiles with configuration
2. Apply desired state
   ```bash
   bossa nova
   # or
   bossa apply
   ```

### After Manual Changes

After manually installing packages:

```bash
# Option 1: Capture the change
bossa brew capture
git add Brewfile
git commit -m "Add new package"

# Option 2: Revert to desired state
bossa brew apply
```

### Regular Maintenance

Periodic drift check:

```bash
# Check for drift
bossa status

# If drift found, decide:
# - Capture: Accept current state as new desired
# - Apply: Revert to desired state
```

## Best Practices

1. **Version control configs** - Keep in dotfiles repo
2. **Capture after changes** - Don't let drift accumulate
3. **Audit before apply** - Know what will change
4. **Use dry-run** - Preview before executing
5. **Single source of truth** - Config files are authoritative
