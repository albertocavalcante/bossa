# Configuration Files

Reference for bossa configuration and state files.

## Configuration Directory

Default location: `~/.config/bossa/`

Bossa currently uses fixed paths (no environment variable overrides).

---

## config.toml

Unified configuration for collections, workspaces, storage, packages, nova, and more.

### Location

`~/.config/bossa/config.toml`

### Example

```toml
[collections.refs]
path = "~/dev/refs"

[[collections.refs.repos]]
url = "https://github.com/rust-lang/rust.git"
name = "rust"

[workspaces]
root = "~/dev/ws"
structure = "bare-worktree"

[[workspaces.repos]]
name = "myproject"
url = "https://github.com/user/myproject.git"
category = "work"

[storage.t9]
mount = "/Volumes/T9"
type = "external"

[[storage.t9.symlinks]]
from = "~/Library/Caches/Homebrew"
to = "{mount}/caches/homebrew"
```

### Key Fields

| Field                             | Type   | Description                                 |
| --------------------------------- | ------ | ------------------------------------------- |
| `collections.<name>.path`         | string | Directory for a collection                  |
| `collections.<name>.repos[].name` | string | Repository name                             |
| `collections.<name>.repos[].url`  | string | Git repository URL                          |
| `workspaces.root`                 | string | Workspace root directory                    |
| `workspaces.repos[].name`         | string | Workspace repository name                   |
| `workspaces.repos[].url`          | string | Workspace repository URL                    |
| `storage.<name>.mount`            | string | Storage mount point                         |
| `storage.<name>.type`             | string | `external`, `internal`, or `network`        |
| `storage.<name>.symlinks[]`       | table  | Symlinks to create under that storage mount |

---

## caches.toml

Cache mappings used by `bossa caches` commands.

### Location

`~/.config/bossa/caches.toml` (create with `bossa caches init`)

### Example

```toml
external_drive = { name = "T9", mount_point = "/Volumes/T9", base_path = "caches" }

[[symlinks]]
name = "homebrew"
source = "~/Library/Caches/Homebrew"
target = "homebrew"

[[symlinks]]
name = "cargo-registry"
source = "~/.cargo/registry"
target = "cargo/registry"

[bazelrc]
output_base = "/Volumes/T9/caches/bazel/output_base"
```

### Key Fields

| Field                        | Type   | Description                           |
| ---------------------------- | ------ | ------------------------------------- |
| `external_drive.name`        | string | Drive label used in status output     |
| `external_drive.mount_point` | string | Expected mount path                   |
| `external_drive.base_path`   | string | Base folder under the mount point     |
| `symlinks[].name`            | string | Human-friendly identifier             |
| `symlinks[].source`          | string | Source path to replace with a symlink |
| `symlinks[].target`          | string | Target path under the cache root      |
| `bazelrc.output_base`        | string | Optional Bazel output base            |

---

## Brewfile

Homebrew bundle configuration.

### Location

Default path: `~/dotfiles/Brewfile` (override with `bossa brew apply --file <path>`).

### Format

Standard Homebrew bundle format (Ruby DSL).

---

## Manifests

SQLite databases created by `bossa manifest scan`.

### Location

`~/.config/bossa/manifests/*.db`

Do not edit manually.

---

## State

Internal state tracking for bossa operations.

### Location

`~/.local/state/bossa/state.toml`

Do not edit manually.

---

## Legacy Files

If you have older configs in `~/.config/workspace-setup/`, migrate them with:

```bash
bossa migrate --dry-run
bossa migrate
```
