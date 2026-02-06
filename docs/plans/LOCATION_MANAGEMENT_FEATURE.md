# Location Management & Self-Healing Feature Plan

> **Status**: Planning
> **Created**: 2026-02-05
> **Updated**: 2026-02-05
> **Author**: Claude Code Analysis

---

## Table of Contents

1. [Problem Statement](#problem-statement)
2. [Current State Analysis](#current-state-analysis)
3. [Pre-Requisite Refactoring](#pre-requisite-refactoring) ⭐ NEW
4. [Proposed Solution](#proposed-solution)
5. [UX Design](#ux-design) ⭐ NEW
6. [Idiomatic Implementation](#idiomatic-implementation) ⭐ NEW
7. [Implementation Plan](#implementation-plan)
8. [Testing Strategy](#testing-strategy)
9. [Success Criteria](#success-criteria)

---

## Problem Statement

When moving development directories (e.g., `~/dev` → `/Volumes/T9/dev`), all dotfiles, configs, and symlinks that reference the old location break. Currently, users must manually:

1. Find all broken symlinks
2. Update each config file that references the old path
3. Recreate symlinks pointing to new locations
4. Update shell rc files, tool configs, IDE settings, etc.

This is error-prone, time-consuming, and discourages organizing storage optimally.

---

## Current State Analysis

### What Bossa Already Has

| Capability                | File                      | Description                                           |
| ------------------------- | ------------------------- | ----------------------------------------------------- |
| **Symlink Resource**      | `src/resource/symlink.rs` | Creates/repairs symlinks, detects `WrongTarget` state |
| **Caches Command**        | `src/commands/caches.rs`  | Manages symlinks to external drives                   |
| **Path Resolution**       | `src/paths.rs`            | Environment overrides, tilde expansion                |
| **Stow Command**          | `src/commands/stow.rs`    | Dotfile symlink management                            |
| **Declarative Framework** | `crates/declarative/`     | State detection, convergence, dry-run                 |
| **State Tracking**        | `src/state.rs`            | Tracks applied changes including `symlinks_created`   |
| **Doctor Command**        | Multiple                  | Health checks across subsystems                       |

### Key Discovery: State Already Tracks Symlinks!

In `src/state.rs`, `StorageState` already has:

```rust
pub struct StorageState {
    pub symlinks_created: Vec<String>,  // ✅ Inventory exists!
    // ...
}
```

This means the **inventory concept partially exists** - it just needs to be:

1. Used consistently across all symlink-creating commands
2. Extended with more metadata (location, subsystem, timestamps)
3. Made queryable for bulk operations

### Current Limitations & Gaps

| Gap                                        | Impact                                           | Priority            |
| ------------------------------------------ | ------------------------------------------------ | ------------------- |
| **Path expansion duplicated**              | `expand_path()` in stow.rs, caches.rs, schema.rs | P1 - Refactor first |
| **Stow doesn't track symlinks**            | Can't bulk-update stow symlinks                  | P1 - Critical       |
| **Stow doesn't use Resource trait**        | Inconsistent with rest of codebase               | P2 - Nice to have   |
| **CachesConfig separate from BossaConfig** | Confusing config split                           | P2 - Nice to have   |
| **No location abstraction**                | Paths are absolute strings                       | P1 - Core feature   |
| **No config file scanning**                | Can't find path references                       | P1 - Core feature   |

---

## Pre-Requisite Refactoring

Before implementing locations, these refactorings will create a solid foundation:

### Phase 0.1: Centralize Path Expansion

**Problem**: `expand_path()` is duplicated across files with slight variations.

**Current locations**:

- `src/commands/stow.rs:634` - `fn expand_path(path: &str) -> PathBuf`
- `src/commands/caches.rs` - uses `shellexpand::tilde` inline
- `src/schema.rs:177` - `fn expanded_path(&self)` on Collection
- `src/resource/symlink.rs:27` - `fn expand_paths(&self)`

**Refactor to**:

```rust
// src/paths.rs - ADD to existing module

/// Expand tilde and environment variables in a path string
pub fn expand(path: &str) -> PathBuf {
    let expanded = shellexpand::full(path)
        .unwrap_or(std::borrow::Cow::Borrowed(path));
    PathBuf::from(expanded.as_ref())
}

/// Expand with location resolution (Phase 1)
pub fn resolve(path: &str, locations: &LocationRegistry) -> PathBuf {
    // First expand ${locations.xxx} references
    // Then expand ~ and env vars
    // Finally resolve to absolute path
}
```

**Files to update**:

- `src/commands/stow.rs` - use `paths::expand()`
- `src/commands/caches.rs` - use `paths::expand()`
- `src/schema.rs` - use `paths::expand()`
- `src/resource/symlink.rs` - use `paths::expand()`

### Phase 0.2: Stow Symlink Tracking

**Problem**: Stow creates symlinks but doesn't record them in state. This means:

- Can't query "what symlinks did bossa create?"
- Can't bulk-update when paths change
- Inconsistent with caches which does track

**Current flow** (stow.rs):

```rust
fn sync(...) {
    // Creates symlinks but doesn't save to state!
    create_symlink(&op.source, &op.target)?;
}
```

**Refactor to**:

```rust
fn sync(...) {
    let mut state = BossaState::load()?;

    create_symlink(&op.source, &op.target)?;

    // Track in state
    state.add_stow_symlink(&op.target.to_string_lossy());
    state.save()?;
}
```

**Add to state.rs**:

```rust
pub struct BossaState {
    // ... existing fields ...

    /// Symlinks created by stow command
    #[serde(default)]
    pub stow_symlinks: Vec<TrackedSymlink>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TrackedSymlink {
    pub source: String,      // What symlink points to
    pub target: String,      // Where symlink lives
    pub subsystem: String,   // "stow", "caches", "collections"
    pub created_at: DateTime<Utc>,
}
```

### Phase 0.3: Unify Symlink Inventory

**Problem**: Different state structures for different subsystems.

**Current**:

- `StorageState.symlinks_created: Vec<String>` - just paths
- Stow: nothing tracked
- Collections: repos tracked, not symlinks

**Unify to**:

```rust
pub struct BossaState {
    // ... existing fields ...

    /// Unified symlink inventory
    #[serde(default)]
    pub symlinks: SymlinkInventory,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SymlinkInventory {
    pub entries: Vec<TrackedSymlink>,
}

impl SymlinkInventory {
    pub fn add(&mut self, symlink: TrackedSymlink);
    pub fn remove(&mut self, target: &str);
    pub fn find_by_source_prefix(&self, prefix: &Path) -> Vec<&TrackedSymlink>;
    pub fn find_by_target_prefix(&self, prefix: &Path) -> Vec<&TrackedSymlink>;
    pub fn find_by_subsystem(&self, subsystem: &str) -> Vec<&TrackedSymlink>;
}
```

### Phase 0.4: (Optional) Stow as Resource

**Problem**: Stow command doesn't use the declarative Resource trait, making it inconsistent.

**Current**: Hand-rolled state detection and application in `stow.rs`.

**Ideal**: Implement `Resource` trait for symlinks, use `execute()` from declarative crate.

**Benefits**:

- Consistent dry-run handling
- Parallel execution
- Privilege batching (if ever needed)
- Unified progress reporting

**Trade-off**: Significant refactor, can defer to later.

---

## Proposed Solution

### Core Concept: Logical Locations

Introduce "logical locations" - named path mappings that can be updated in one place:

```toml
# In ~/.config/bossa/config.toml

[locations]
dev = "/Volumes/T9/dev"
workspaces = "${locations.dev}/ws" # Variable reference
refs = "${locations.dev}/refs"
forks = "${locations.dev}/forks"
dotfiles = "~/dotfiles"

[locations.aliases]
# Historical paths that redirect to locations
"~/dev" = "dev"
"/Users/adsc/dev" = "dev"
```

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     BossaConfig                              │
│  ┌─────────────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │   [locations]   │  │  [symlinks]  │  │ [collections] │  │
│  │ dev = /Vol/T9.. │  │ source = ... │  │ path = ${loc..│  │
│  └────────┬────────┘  └──────────────┘  └───────────────┘  │
│           │                                                  │
└───────────┼──────────────────────────────────────────────────┘
            │
            ▼
┌─────────────────────────────────────────────────────────────┐
│                   LocationRegistry                           │
│  ┌─────────────────────────────────────────────────────┐    │
│  │ resolve(path) → expands ${locations.x} + ~ + $ENV   │    │
│  │ identify(path) → finds matching location name       │    │
│  │ relocate(name, new_path) → updates + tracks history │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
            │
            ▼
┌─────────────────────────────────────────────────────────────┐
│                    BossaState                                │
│  ┌─────────────────┐  ┌──────────────────────────────────┐  │
│  │SymlinkInventory │  │      LocationHistory             │  │
│  │ - entries[]     │  │ - changes: Vec<LocationChange>   │  │
│  │ - find_by_*()   │  │ - last_known: HashMap<name,path> │  │
│  └─────────────────┘  └──────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

---

## UX Design

### Principle 1: Progressive Disclosure

Simple mode for common cases, advanced options available:

```bash
# Simple: One command does everything
bossa relocate ~/dev /Volumes/T9/dev

# Advanced: Step by step with control
bossa locations scan ~/dev              # Just scan
bossa locations relocate dev /Vol/T9/dev --dry-run  # Preview
bossa locations relocate dev /Vol/T9/dev            # Execute
```

### Principle 2: Visual Diff Before Changes

Always show what will change before doing it:

```
$ bossa relocate ~/dev /Volumes/T9/dev

Scanning for references to ~/dev...

Found 47 references:

Symlinks (23):
  ~/.config/nvim → ~/dev/dotfiles/nvim
                 → /Volumes/T9/dev/dotfiles/nvim  ✓ will update

  ~/.zshrc → ~/dev/dotfiles/zsh/.zshrc
           → /Volumes/T9/dev/dotfiles/zsh/.zshrc  ✓ will update

Config Files (12):
  ~/.config/bossa/config.toml
    - collections.refs.path = "~/dev/refs"
    + collections.refs.path = "/Volumes/T9/dev/refs"

  ~/.gitconfig
    - [core] excludesfile = ~/dev/dotfiles/git/.gitignore_global
    + [core] excludesfile = /Volumes/T9/dev/dotfiles/git/.gitignore_global

Shell RC (3):
  ~/.zshrc
    - export DEV_HOME=~/dev
    + export DEV_HOME=/Volumes/T9/dev

Not Updated (9):
  ⚠ ~/.config/Code/User/settings.json (binary format - manual update needed)
  ⚠ ~/Library/Application Support/JetBrains/IntelliJIdea2025.3/options/path.macros.xml

───────────────────────────────────────────────────────
Summary: 38 updates, 9 manual, 0 errors

Proceed? [y/N/d(diff)/q(quit)]
```

### Principle 3: Safe Defaults with Escape Hatches

```bash
# Default: Creates backups, asks for confirmation
bossa relocate ~/dev /Volumes/T9/dev

# Skip confirmation (for scripts)
bossa relocate ~/dev /Volumes/T9/dev --yes

# No backups (dangerous)
bossa relocate ~/dev /Volumes/T9/dev --no-backup

# Force update even with errors
bossa relocate ~/dev /Volumes/T9/dev --force
```

### Principle 4: Undo/Rollback

```bash
# After relocate, backups are kept with timestamp
~/.local/state/bossa/backups/
  2026-02-05T14-30-00/
    .config/bossa/config.toml
    .gitconfig
    .zshrc

# Rollback command
bossa locations rollback

# Or restore specific backup
bossa locations rollback 2026-02-05T14-30-00
```

### Principle 5: Integration with Existing Commands

```bash
# Doctor shows location health
bossa doctor

System Health:
  ✓ Homebrew
  ✓ Shell

Locations:
  ✓ dev: /Volumes/T9/dev (accessible)
  ✗ dotfiles: ~/dotfiles (missing)
    └─ Hint: Found at /Volumes/T9/dev/dotfiles
    └─ Run: bossa locations relocate dotfiles /Volumes/T9/dev/dotfiles

Symlinks:
  ✓ 45/45 valid
```

### Principle 6: Shell Integration

```bash
# Generate shell integration
bossa locations export --shell zsh

# Output (add to .zshrc):
# ─────────────────────────────────────
# Bossa Location Shortcuts
export BOSSA_DEV="/Volumes/T9/dev"
export BOSSA_WS="$BOSSA_DEV/ws"
export BOSSA_REFS="$BOSSA_DEV/refs"

# Navigation functions
dev()  { cd "$BOSSA_DEV/${1:-.}"; }
ws()   { cd "$BOSSA_WS/${1:-.}"; }
refs() { cd "$BOSSA_REFS/${1:-.}"; }

# Completions
_bossa_ws() { _files -W "$BOSSA_WS" -/; }
compdef _bossa_ws ws
# ─────────────────────────────────────
```

---

## Idiomatic Implementation

### Follow Existing Patterns

#### 1. Config in `schema.rs`

```rust
// Add to BossaConfig struct
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BossaConfig {
    // ... existing fields ...

    /// Logical locations for path management
    #[serde(default)]
    pub locations: LocationsConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LocationsConfig {
    /// Named location mappings
    #[serde(default)]
    pub paths: HashMap<String, String>,

    /// Aliases for historical paths
    #[serde(default)]
    pub aliases: HashMap<String, String>,

    /// Detection settings
    #[serde(default)]
    pub detection: LocationDetection,
}

impl LocationsConfig {
    pub fn validate(&self) -> Result<()> {
        for (name, path) in &self.paths {
            // Validate name is alphanumeric + underscore
            if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                bail!("Invalid location name '{}': must be alphanumeric", name);
            }
            // Validate path doesn't contain ${} that reference unknown locations
            self.validate_path_references(path)?;
        }
        Ok(())
    }
}
```

#### 2. State in `state.rs`

```rust
// Add to BossaState struct
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BossaState {
    // ... existing fields ...

    /// Unified symlink inventory
    #[serde(default)]
    pub symlinks: SymlinkInventory,

    /// Location change history for rollback
    #[serde(default)]
    pub location_history: Vec<LocationChange>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LocationChange {
    pub name: String,
    pub from: String,
    pub to: String,
    pub timestamp: DateTime<Utc>,
    pub backup_path: Option<String>,
}
```

#### 3. Command Handler Pattern

```rust
// src/commands/locations.rs

use crate::Context;
use crate::cli::LocationsCommand;
use crate::schema::BossaConfig;
use crate::state::BossaState;
use crate::ui;

pub fn run(ctx: &Context, cmd: LocationsCommand) -> Result<()> {
    match cmd {
        LocationsCommand::List => list(ctx),
        LocationsCommand::Add { name, path } => add(ctx, &name, &path),
        LocationsCommand::Remove { name } => remove(ctx, &name),
        LocationsCommand::Relocate { name, path, dry_run, force } => {
            relocate(ctx, &name, &path, dry_run, force)
        }
        LocationsCommand::Scan { path } => scan(ctx, &path),
        LocationsCommand::Doctor => doctor(ctx),
        LocationsCommand::Export { shell } => export(ctx, &shell),
        LocationsCommand::Rollback { timestamp } => rollback(ctx, timestamp.as_deref()),
    }
}

fn list(ctx: &Context) -> Result<()> {
    let config = BossaConfig::load()?;

    ui::header("Locations");

    for (name, path) in &config.locations.paths {
        let expanded = paths::resolve(path, &config.locations);
        let exists = expanded.exists();

        let icon = if exists { "✓".green() } else { "✗".red() };
        ui::kv(&format!("{} {}", icon, name), &expanded.display().to_string());
    }

    Ok(())
}
```

#### 4. Use Resource Trait (If Converting Stow)

```rust
// src/resource/stow_symlink.rs

use declarative::{Resource, ResourceState, ApplyResult, ApplyContext, SudoRequirement};

pub struct StowSymlink {
    pub source: PathBuf,
    pub target: PathBuf,
    pub package: String,
}

impl Resource for StowSymlink {
    fn id(&self) -> String {
        self.target.to_string_lossy().to_string()
    }

    fn description(&self) -> String {
        format!("Symlink {} -> {}", self.target.display(), self.source.display())
    }

    fn resource_type(&self) -> &'static str {
        "stow-symlink"
    }

    fn sudo_requirement(&self) -> SudoRequirement {
        SudoRequirement::None
    }

    fn current_state(&self) -> Result<ResourceState> {
        // Check filesystem state
    }

    fn desired_state(&self) -> ResourceState {
        ResourceState::Present {
            details: Some(format!("-> {}", self.source.display())),
        }
    }

    fn apply(&self, ctx: &mut ApplyContext) -> Result<ApplyResult> {
        if ctx.dry_run {
            return Ok(ApplyResult::Skipped { reason: "Dry run".into() });
        }
        // Create symlink and track in inventory
    }
}
```

#### 5. Error Handling with Context

```rust
// Good: Rich error context
let config = BossaConfig::load()
    .context("Failed to load bossa config")?;

let location = config.locations.paths.get(name)
    .with_context(|| format!("Location '{}' not found", name))?;

// Also good: Bail with clear message
if !new_path.exists() {
    bail!(
        "Target path does not exist: {}\n\
         Create it first or use --force to proceed anyway",
        new_path.display()
    );
}
```

#### 6. UI Patterns (via pintui)

```rust
use crate::ui;

// Headers
ui::header("Relocating Location");

// Key-value pairs
ui::kv("From", &old_path.display().to_string());
ui::kv("To", &new_path.display().to_string());

// Status messages
ui::success("Location updated successfully");
ui::warn("Some references could not be updated");
ui::error("Failed to update symlinks");

// Dimmed details
ui::dim(&format!("Backup created at {}", backup_path.display()));

// Sections
ui::section("Symlinks") {
    for symlink in &updated_symlinks {
        println!("  {} {}", "✓".green(), symlink);
    }
}
```

---

## Implementation Plan

### Phase 0: Pre-Requisites (Foundation)

| Task                         | Priority | Effort | Files                                   |
| ---------------------------- | -------- | ------ | --------------------------------------- |
| Centralize `expand_path()`   | P1       | Small  | paths.rs, stow.rs, caches.rs, schema.rs |
| Add symlink tracking to stow | P1       | Medium | stow.rs, state.rs                       |
| Unify SymlinkInventory       | P1       | Medium | state.rs, caches.rs                     |
| (Optional) Stow as Resource  | P3       | Large  | resource/stow_symlink.rs, stow.rs       |

### Phase 1: Location Registry

| Task                         | Priority | Effort | Files                         |
| ---------------------------- | -------- | ------ | ----------------------------- |
| LocationsConfig in schema    | P1       | Small  | schema.rs                     |
| Location resolution in paths | P1       | Medium | paths.rs (new: locations.rs)  |
| Basic CLI commands           | P1       | Small  | cli.rs, commands/locations.rs |
| Validation                   | P1       | Small  | schema.rs                     |

### Phase 2: Symlink Inventory

| Task                    | Priority | Effort | Files              |
| ----------------------- | -------- | ------ | ------------------ |
| SymlinkInventory struct | P1       | Small  | state.rs           |
| Integrate with stow     | P1       | Small  | commands/stow.rs   |
| Integrate with caches   | P1       | Small  | commands/caches.rs |
| Query methods           | P1       | Small  | state.rs           |

### Phase 3: Config Scanner

| Task                      | Priority | Effort | Files                   |
| ------------------------- | -------- | ------ | ----------------------- |
| TOML/JSON path extraction | P1       | Medium | config_scanner.rs (new) |
| Shell rc file parsing     | P1       | Medium | config_scanner.rs       |
| Pattern matching          | P1       | Small  | config_scanner.rs       |

### Phase 4: Relocate Command

| Task             | Priority | Effort | Files                      |
| ---------------- | -------- | ------ | -------------------------- |
| Scan workflow    | P1       | Medium | commands/relocate.rs (new) |
| Backup creation  | P1       | Small  | commands/relocate.rs       |
| Config rewriting | P1       | Large  | commands/relocate.rs       |
| Rollback command | P2       | Medium | commands/relocate.rs       |
| Interactive mode | P2       | Medium | commands/relocate.rs       |

### Phase 5: Integration & Polish

| Task               | Priority | Effort | Files                 |
| ------------------ | -------- | ------ | --------------------- |
| Doctor integration | P1       | Small  | commands/doctor.rs    |
| Shell export       | P2       | Small  | commands/locations.rs |
| Completions        | P3       | Small  | cli.rs                |
| Documentation      | P2       | Medium | docs/                 |

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_location_resolution_simple() {
        let config = LocationsConfig {
            paths: [("dev".into(), "/Volumes/T9/dev".into())].into(),
            ..Default::default()
        };

        assert_eq!(
            resolve("${locations.dev}/ws", &config),
            PathBuf::from("/Volumes/T9/dev/ws")
        );
    }

    #[test]
    fn test_location_resolution_nested() {
        let config = LocationsConfig {
            paths: [
                ("dev".into(), "/Volumes/T9/dev".into()),
                ("ws".into(), "${locations.dev}/ws".into()),
            ].into(),
            ..Default::default()
        };

        assert_eq!(
            resolve("${locations.ws}/myproject", &config),
            PathBuf::from("/Volumes/T9/dev/ws/myproject")
        );
    }

    #[test]
    fn test_symlink_inventory_find_by_source() {
        let mut inv = SymlinkInventory::default();
        inv.add(TrackedSymlink {
            source: "~/dev/dotfiles/nvim".into(),
            target: "~/.config/nvim".into(),
            subsystem: "stow".into(),
            created_at: Utc::now(),
        });

        let found = inv.find_by_source_prefix(Path::new("~/dev"));
        assert_eq!(found.len(), 1);
    }
}
```

### Integration Tests

```rust
#[test]
fn test_relocate_updates_symlinks() {
    let temp = TempDir::new().unwrap();
    let old_dev = temp.path().join("old_dev");
    let new_dev = temp.path().join("new_dev");

    // Setup
    fs::create_dir_all(&old_dev.join("dotfiles")).unwrap();
    fs::create_dir_all(&new_dev).unwrap();

    // Create symlink to old location
    let target = temp.path().join("link");
    std::os::unix::fs::symlink(&old_dev.join("dotfiles"), &target).unwrap();

    // Move files
    fs::rename(&old_dev, &new_dev.join("dev")).unwrap();

    // Run relocate
    // ... test that symlink is updated
}
```

---

## Success Criteria

| Criterion               | Measurement                              |
| ----------------------- | ---------------------------------------- |
| **Zero Manual Updates** | `bossa relocate` updates all references  |
| **Complete Coverage**   | All symlinks + config files updated      |
| **Safe Operation**      | Dry-run accurate, backups always created |
| **Rollback Works**      | Can undo any relocate                    |
| **Self-Healing**        | `bossa doctor` detects + suggests fixes  |
| **Shell Integration**   | Navigation works after relocate          |
| **Idiomatic Code**      | Follows existing bossa patterns          |
| **Well Tested**         | >80% coverage on new code                |

---

## Appendix A: Config Patterns to Scan

### Shell RC Files

- `~/.bashrc`, `~/.zshrc`, `~/.profile`
- `~/.bash_profile`, `~/.zprofile`
- `~/.config/fish/config.fish`

### Tool Configs

- `~/.gitconfig` - `[core] excludesfile`, `[include] path`
- `~/.npmrc` - `prefix`, `cache`
- `~/.cargo/config.toml` - `[source]`, `[build]`
- `~/go/env` - `GOPATH`, `GOBIN`
- `~/.gradle/gradle.properties` - various paths

### IDE Settings

- `~/.config/Code/User/settings.json`
- `~/Library/Application Support/JetBrains/*/options/*.xml`
- `~/.config/JetBrains/*/options/*.xml`

### Bossa's Own Configs

- `~/.config/bossa/config.toml`
- `~/.config/bossa/caches.toml`

---

## Appendix B: Related Tools

| Tool             | Approach             | Limitation                  |
| ---------------- | -------------------- | --------------------------- |
| **GNU Stow**     | Symlink farm manager | No path tracking            |
| **chezmoi**      | Dotfile templating   | Complex, no runtime updates |
| **home-manager** | Nix-based            | Requires Nix                |
| **mackup**       | App settings backup  | Simpler, no symlinks        |
| **rcm**          | Dotfile management   | No path abstraction         |

Bossa's advantage: **Runtime location management** with full symlink tracking and config rewriting.
