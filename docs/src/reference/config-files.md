# Configuration Files

Reference for bossa configuration and state files.

## Configuration Directory

Default location: `~/.config/bossa/`

Set `BOSSA_CONFIG_DIR` to select another directory. Otherwise an existing `~/.config/bossa/`
takes precedence over `XDG_CONFIG_HOME/bossa`, followed by the platform default. State and
workspace roots have separate `BOSSA_STATE_DIR` and `BOSSA_WORKSPACES_DIR` overrides.

---

## config.toml or config.json

Unified configuration for collections, workspaces, storage, packages, nova, and more.

### Location

`~/.config/bossa/config.toml`

### Loading and editing

Nova, CRUD, status, diff, and apply use the same canonical configuration model. TOML is preferred
when both files exist. A read or parse failure in TOML blocks the command rather than selecting
JSON. Unknown fields in typed sections are rejected; explicitly named maps, such as macOS defaults
or custom Git sections, retain their documented arbitrary keys.

Configuration reads do not create directories or write files. Missing configuration is an
unconfigured status; apply and Nova require a selected file even for a preview. Files must be
regular UTF-8 files or symlinks to such files, at most 8 MiB. Unreadable files, dangling symlinks,
unsupported shapes, and over-limit files are errors.

Duplicate JSON object keys are rejected, including nested keys, so an edit cannot silently drop an
earlier value. TOML uses its native value model, preserving floats and datetimes.

CRUD and migration edits preserve the selected filename and format. Editing JSON does not create a
higher-priority TOML file. Complete contents are staged before replacement; a staging failure leaves
the original file intact and removes the temporary file. Existing file symlinks and basic file
permissions are preserved. Serialization preserves modeled sections; preserving TOML comments,
coordinating concurrent read-modify-write operations, and crash recovery are separate contracts.
Retain a copy before manually converting formats.

The former vector-shaped collections and storage sections have explicit read compatibility.
Reading them does not convert the file. CRUD refuses to rewrite legacy input automatically.
Duplicate resource names and legacy workspace layouts with custom paths are rejected because
conversion could change their meaning. Review and explicitly convert such layouts before applying;
there is no generic automatic conversion command.

The historical unused workspace default (`root = ""`, `structure = ""`, no repositories) remains
readable and editable. Configured workspaces require the `bare-worktree` layout and relative
categories and worktree branch paths that stay within the configured root. Repository clone depth,
single-branch selection, additional options, and default branch retain their configured meanings;
additional options cannot override the corresponding typed fields or the argument boundary.

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

State reads use the same regular UTF-8 file and 8 MiB limits. Malformed or unavailable state is an
error; relocation does not replace it with an empty successful observation. Do not edit manually.

---

## Legacy Files

If you have older configs in `~/.config/workspace-setup/`, migrate them with:

```bash
bossa migrate --dry-run
bossa migrate
```
