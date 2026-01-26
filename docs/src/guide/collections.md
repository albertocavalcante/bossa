# Collections

Collections are groups of git repositories that you want to manage together. Use them for reference repositories, tools, or any set of repos you want to keep in sync.

## Commands

```bash
bossa collections list                # List all collections
bossa collections status <name>       # Show clone status
bossa collections sync <name>         # Clone missing repos
bossa collections audit <name>        # Find drift
bossa collections snapshot <name>     # Regenerate config from disk
bossa collections add <name> <url>    # Add repo to collection
bossa collections rm <name> <repo>    # Remove repo from collection
bossa collections clean <name>        # Delete clones (preserve config)
```

## Defining Collections

Collections are defined in `~/.config/bossa/config.toml`:

```toml
[collections.refs]
path = "~/dev/refs"

[[collections.refs.repos]]
url = "https://github.com/rust-lang/rust.git"
name = "rust"

[[collections.refs.repos]]
url = "https://github.com/torvalds/linux.git"
name = "linux"

[[collections.refs.repos]]
url = "https://github.com/neovim/neovim.git"
name = "neovim"

[collections.tools]
path = "~/dev/tools"

[[collections.tools.repos]]
url = "https://github.com/BurntSushi/ripgrep.git"
name = "ripgrep"

[[collections.tools.repos]]
url = "https://github.com/sharkdp/fd.git"
name = "fd"
```

## Workflow

### 1. List Collections

See all defined collections:

```bash
bossa collections list
```

Output:

```
refs (~/dev/refs)
  - rust-lang/rust
  - torvalds/linux
  - neovim/neovim

tools (~/dev/tools)
  - BurntSushi/ripgrep
  - sharkdp/fd
```

### 2. Check Status

See which repos are cloned:

```bash
bossa collections status refs
```

Output:

```
refs (3 repos)
  ✓ rust           cloned
  ✓ linux          cloned
  ✗ neovim         missing
```

### 3. Sync Collections

Clone missing repositories:

```bash
bossa collections sync refs
```

Features:

- Parallel cloning (configurable with `-j`)
- Automatic retry on failure
- Progress reporting

### 4. Add Repositories

Add a new repo to a collection:

```bash
bossa collections add refs https://github.com/golang/go.git
```

This:

1. Adds the repo to `config.toml`
2. Optionally clones immediately with `--clone`

### 5. Remove Repositories

Remove a repo from a collection:

```bash
bossa collections rm refs neovim
```

This removes from config only. Use `--delete` to also delete the clone.

### 6. Audit for Drift

Find repos on disk that aren't in config:

```bash
bossa collections audit refs
```

Output:

```
refs: 1 untracked repo
  ~/dev/refs/some-repo  (not in config)
```

### 7. Snapshot from Disk

Regenerate config from what's actually on disk:

```bash
bossa collections snapshot refs
```

Useful when you've manually cloned repos and want to track them.

## Options

### Sync Options

```bash
# Parallel cloning
bossa collections sync refs -j 8

# Retry failed clones
bossa collections sync refs --retries 5

# Dry run
bossa collections sync refs --dry-run

# Verbose output
bossa collections sync refs -v
```

### Add Options

```bash
# Custom name
bossa collections add refs https://github.com/user/repo.git --name myrepo

# Clone immediately
bossa collections add refs https://github.com/user/repo.git --clone
```

## Configuration Format

### Basic Repository

```toml
[[collections.refs.repos]]
url = "https://github.com/user/repo.git"
name = "repo"
```

When you add repos with the CLI, the name is derived from the URL if omitted.

### With Custom Name

```toml
[[collections.refs.repos]]
url = "https://github.com/user/repo.git"
name = "custom-name"
```

### SSH URLs

```toml
[[collections.refs.repos]]
url = "git@github.com:user/repo.git"
name = "repo"
```

## Use Cases

### Reference Repositories

Keep copies of important projects for reference:

```toml
[collections.refs]
path = "~/dev/refs"

[[collections.refs.repos]]
url = "https://github.com/rust-lang/rust.git"
name = "rust"

[[collections.refs.repos]]
url = "https://github.com/golang/go.git"
name = "go"
```

### Tool Sources

Track tools you might want to build from source:

```toml
[collections.tools]
path = "~/dev/tools"

[[collections.tools.repos]]
url = "https://github.com/neovim/neovim.git"
name = "neovim"
```

### Learning Projects

Curate repos for learning:

```toml
[collections.learning]
path = "~/dev/learning"

[[collections.learning.repos]]
url = "https://github.com/codecrafters-io/build-your-own-x.git"
name = "build-your-own-x"
```

## Tips

1. **Use HTTPS for public repos** - No SSH key needed
2. **Use SSH for private repos** - `git@github.com:...`
3. **Parallel sync** - Use `-j` for faster cloning
4. **Audit regularly** - Catch manual clones with `audit`
5. **Snapshot before changes** - Backup current state
