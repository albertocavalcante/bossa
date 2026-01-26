# Caching

Bossa provides tools for managing caches across storage locations.

## Cache Management

### Problem

Development tools create large caches:

- Homebrew: `~/Library/Caches/Homebrew` (GBs of downloads)
- Cargo: `~/.cargo/registry` (Rust crate cache)
- npm: `~/.npm` (Node package cache)
- pip: `~/.cache/pip` (Python package cache)

These consume valuable SSD space.

### Solution

Move caches to external storage while maintaining functionality:

```toml
# ~/.config/bossa/caches.toml
external_drive = { name = "T9", mount_point = "/Volumes/T9", base_path = "caches" }

[[symlinks]]
name = "homebrew"
source = "~/Library/Caches/Homebrew"
target = "homebrew"
```

## Commands

```bash
bossa caches status    # Show cache locations
bossa caches apply     # Apply config (create/migrate symlinks)
bossa caches audit     # Detect drift
bossa caches doctor    # Health checks
bossa caches init      # Create a starter config
```

## Configuration

### caches.toml

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

[[symlinks]]
name = "npm"
source = "~/.npm"
target = "npm"

[bazelrc]
output_base = "/Volumes/T9/caches/bazel/output_base"
```

## Workflow

### Initial Setup

1. Create a starter config:
   ```bash
   bossa caches init
   ```

2. Edit `~/.config/bossa/caches.toml` to your paths

3. Apply the config:
   ```bash
   bossa caches apply
   ```

### How It Works

The `apply` command:

1. Moves cache contents to target location (if needed)
2. Removes the original directory
3. Creates a symlink from source to target

```
Before:
~/Library/Caches/Homebrew/  (actual directory)

After:
~/Library/Caches/Homebrew -> /Volumes/T9/caches/homebrew
```

### Verification

```bash
bossa caches status
```

Output:

```
Cache Status
============

homebrew
  Source: ~/Library/Caches/Homebrew
  Target: /Volumes/T9/caches/homebrew
  Status: Symlinked ✓
  Size:   2.3 GB

cargo-registry
  Source: ~/.cargo/registry
  Target: /Volumes/T9/caches/cargo/registry
  Status: Symlinked ✓
  Size:   1.8 GB

npm
  Source: ~/.npm
  Target: /Volumes/T9/caches/npm
  Status: Not created ○
  Size:   500 MB
```

## When External Drive is Disconnected

### Detection

```bash
bossa caches status
```

If target is unavailable:

```
homebrew
  Status: Drive not mounted ⚠
  Target: /Volumes/T9/caches/homebrew
```

### Graceful Handling

Most tools handle missing cache gracefully:

- Homebrew: Re-downloads packages
- Cargo: Re-downloads crates
- npm: Re-downloads packages

No data loss, just longer first build.

### Temporary Override

To use local cache temporarily:

```bash
# Homebrew
export HOMEBREW_CACHE=~/.local-cache/Homebrew

# Cargo
export CARGO_HOME=~/.local-cargo
```

## CI/CD Integration

### GitHub Actions

```yaml
- name: Restore cache
  uses: actions/cache@v4
  with:
    path: |
      ~/.cargo/registry
      ~/.cargo/git
      target/
    key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
```

### Cache Modes

For CI, consider read-only mode to prevent cache pollution:

```bash
# Read from cache, don't write
export CARGO_NET_OFFLINE=true
```

## Best Practices

1. **Use fast external storage** - USB 3.0+ or Thunderbolt
2. **Format for your OS** - APFS for macOS, ext4 for Linux
3. **Regular cleanup** - Caches grow over time
4. **Backup important caches** - Cargo registry, npm, etc.
5. **Monitor disk space** - External drives fill up too

## Cleanup

Periodically clean caches:

```bash
# Homebrew
brew cleanup

# Cargo
cargo cache --autoclean

# npm
npm cache clean --force

# pip
pip cache purge
```

## Common Cache Locations

| Tool     | Default Location                    | Typical Size  |
| -------- | ----------------------------------- | ------------- |
| Homebrew | `~/Library/Caches/Homebrew`         | 2-10 GB       |
| Cargo    | `~/.cargo/registry`, `~/.cargo/git` | 1-5 GB        |
| npm      | `~/.npm`                            | 500 MB - 2 GB |
| pip      | `~/.cache/pip`                      | 200 MB - 1 GB |
| Go       | `~/go/pkg/mod`                      | 500 MB - 2 GB |
| Maven    | `~/.m2/repository`                  | 1-5 GB        |
| Gradle   | `~/.gradle/caches`                  | 1-5 GB        |
