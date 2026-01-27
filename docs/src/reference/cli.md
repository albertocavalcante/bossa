# CLI Commands

Complete reference for bossa commands.

## Global Options

```
-v, --verbose    Increase verbosity (can repeat: -v, -vv, -vvv)
-q, --quiet      Suppress non-essential output
-h, --help       Print help
-V, --version    Print version
```

## Commands Overview

| Command       | Description                   |
| ------------- | ----------------------------- |
| `nova`        | Bootstrap a new machine       |
| `status`      | Show current vs desired state |
| `apply`       | Apply desired state           |
| `diff`        | Preview what would change     |
| `add`         | Add resources to config       |
| `rm`          | Remove resources from config  |
| `list`        | List resources                |
| `show`        | Show detailed resource info   |
| `doctor`      | System health check           |
| `migrate`     | Migrate legacy configs        |
| `caches`      | Manage cache locations        |
| `collections` | Manage repository collections |
| `manifest`    | Content manifest operations   |
| `icloud`      | iCloud Drive management       |
| `storage`     | Unified storage overview      |
| `brew`        | Homebrew package management   |
| `refs`        | Deprecated refs commands      |
| `completions` | Generate shell completions    |

---

## nova

```bash
bossa nova [OPTIONS]
```

Options:

```
--skip <STAGES>      Skip specific stages (comma-separated)
--only <STAGES>      Only run specific stages (comma-separated)
--list-stages        List all available stages
--dry-run            Show what would be done
-j, --jobs <N>       Number of parallel jobs (max 128)
```

Examples:

```bash
bossa nova
bossa nova --list-stages
bossa nova --only=homebrew,brew
bossa nova --skip=dock
bossa nova --dry-run
```

---

## status

```bash
bossa status [TARGET]
```

Examples:

```bash
bossa status
bossa status collections.refs
bossa status storage.t9
```

---

## apply

```bash
bossa apply [TARGET] [OPTIONS]
```

Options:

```
--dry-run           Show what would be done
-j, --jobs <N>       Number of parallel jobs (max 128)
```

Examples:

```bash
bossa apply
bossa apply collections.refs
bossa apply --dry-run
```

---

## diff

```bash
bossa diff [TARGET]
```

Examples:

```bash
bossa diff
bossa diff workspaces
```

---

## add

```bash
bossa add <SUBCOMMAND>
```

Subcommands:

| Subcommand   | Description                |
| ------------ | -------------------------- |
| `collection` | Add a collection           |
| `repo`       | Add a repo to a collection |
| `workspace`  | Add a workspace repo       |
| `storage`    | Add a storage volume       |

Examples:

```bash
bossa add collection refs ~/dev/refs --description "Reference repos"
bossa add repo refs https://github.com/rust-lang/rust.git
bossa add workspace https://github.com/user/app.git --category work
bossa add storage t9 /Volumes/T9 --storage-type external
```

---

## rm

```bash
bossa rm <SUBCOMMAND>
```

Subcommands:

| Subcommand   | Description                     |
| ------------ | ------------------------------- |
| `collection` | Remove a collection             |
| `repo`       | Remove a repo from a collection |
| `workspace`  | Remove a workspace repo         |
| `storage`    | Remove a storage volume         |

Examples:

```bash
bossa rm collection refs
bossa rm repo refs rust
bossa rm workspace myproject
bossa rm storage t9
```

---

## list

```bash
bossa list <collections|repos|workspaces|storage>
```

Examples:

```bash
bossa list collections
bossa list workspaces
```

---

## show

```bash
bossa show <TARGET>
```

Examples:

```bash
bossa show collections.refs
bossa show workspaces.myproject
bossa show storage.t9
```

---

## doctor

```bash
bossa doctor
```

---

## migrate

```bash
bossa migrate [OPTIONS]
```

Options:

```
-n, --dry-run    Preview changes without writing
```

---

## caches

```bash
bossa caches <COMMAND>
```

Subcommands:

| Command  | Description                          |
| -------- | ------------------------------------ |
| `status` | Show cache status                    |
| `apply`  | Apply cache config (create symlinks) |
| `audit`  | Detect drift                         |
| `doctor` | Cache health check                   |
| `init`   | Create starter config                |

Examples:

```bash
bossa caches init
bossa caches status
bossa caches apply
bossa caches apply --dry-run
bossa caches audit
```

---

## collections

```bash
bossa collections <COMMAND>
```

Subcommands:

| Command    | Description                 |
| ---------- | --------------------------- |
| `list`     | List all collections        |
| `status`   | Show collection status      |
| `sync`     | Clone missing repos         |
| `audit`    | Find drift                  |
| `snapshot` | Regenerate config from disk |
| `add`      | Add repo to collection      |
| `rm`       | Remove repo from collection |
| `clean`    | Delete clones               |

Examples:

```bash
bossa collections list
bossa collections status refs
bossa collections sync refs -j 8
bossa collections audit refs --fix
bossa collections add refs https://github.com/neovim/neovim.git --clone
bossa collections rm refs neovim --delete
bossa collections clean refs --dry-run
```

---

## manifest

```bash
bossa manifest <COMMAND>
```

Subcommands:

| Command      | Description              |
| ------------ | ------------------------ |
| `scan`       | Hash files in directory  |
| `stats`      | Show manifest statistics |
| `duplicates` | Find duplicates          |

Examples:

```bash
bossa manifest scan ~/dev --force
bossa manifest stats ~/dev
bossa manifest duplicates ~/dev --min-size 1048576
```

---

## icloud

```bash
bossa icloud <COMMAND>
```

Subcommands:

| Command          | Description                |
| ---------------- | -------------------------- |
| `status`         | Show iCloud status         |
| `list`           | List files with status     |
| `find-evictable` | Find large local files     |
| `evict`          | Evict files to free space  |
| `download`       | Download files from iCloud |

Examples:

```bash
bossa icloud status
bossa icloud list --cloud
bossa icloud find-evictable --min-size 100MB
bossa icloud evict ~/Library/Mobile\ Documents --recursive --dry-run
bossa icloud download ~/Library/Mobile\ Documents --recursive
```

---

## storage

```bash
bossa storage <COMMAND>
```

Subcommands:

| Command      | Description           |
| ------------ | --------------------- |
| `status`     | Show storage overview |
| `duplicates` | Find duplicates       |

Examples:

```bash
bossa storage status
bossa storage duplicates --list
bossa storage duplicates icloud t9 --min-size 1048576 --limit 5
```

---

## brew

```bash
bossa brew <COMMAND>
```

Subcommands:

| Command   | Description                             |
| --------- | --------------------------------------- |
| `apply`   | Install packages from Brewfile          |
| `capture` | Update Brewfile with installed packages |
| `audit`   | Detect drift                            |
| `list`    | List installed packages                 |

Examples:

```bash
bossa brew apply --dry-run
bossa brew apply --file ~/dotfiles/Brewfile
bossa brew capture --output ~/dotfiles/Brewfile
bossa brew audit --file ~/dotfiles/Brewfile
bossa brew list --type cask
```

---

## refs (deprecated)

```bash
bossa refs <COMMAND>
```

Use `bossa collections` instead. Subcommands mirror collections behavior.

---

## completions

```bash
bossa completions <SHELL>
```

Supported shells:

- `bash`
- `zsh`
- `fish`
- `powershell`
