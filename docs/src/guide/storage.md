# Storage Management

Bossa provides a unified view of storage across your local SSD, iCloud Drive, and external drives.

## Commands

```bash
bossa storage status      # Unified storage overview
bossa storage duplicates  # Find duplicates across locations
```

## Storage Overview

Get a bird's-eye view of all storage:

```bash
bossa storage status
```

The overview includes local filesystem capacity, iCloud metadata and available manifest summaries.
The iCloud scan is metadata-only, excludes hidden entries, stops at depth 3 and processes at most 1,000 entries (including the root). It reports filesystem coverage as `complete`, `partial`, or `unavailable`, plus skipped entries, errors and unknown sizes. iCloud residency is always unknown: allocation and extended-attribute heuristics cannot prove local or cloud state. Logical-byte subtotals include only observed known-size files; they are lower bounds,
not allocated bytes, reclaimable space, or evidence that eviction is safe. Counts are observed counts, not complete totals when coverage is partial. Concurrent filesystem changes mean a scan is not an atomic snapshot. An unavailable
root is unknown, not an empty directory. This does not inspect Photos or request cloud-file downloads. Native no-hydration behavior remains to be qualified on each supported macOS release. The entry limit is not a wall-clock deadline: the existing iCloud status backend may block on a system call or subprocess. Callers must not assume a bounded completion time.

Use `bossa storage status --json` for schema `bossa.storage-status/v1`. Its `icloud`
object includes the resolved root when available, scope, coverage and observation counts.
The known logical-byte subtotal is null when the root cannot be observed. iCloud state coverage is unavailable even for a complete filesystem walk. `local`, `external`
and `manifests` may be null when their observation is unavailable. Never infer a complete
inventory or verified backup from this report.

## Finding Duplicates

Find duplicate files across storage locations:

```bash
bossa storage duplicates
```

This requires manifests to be scanned first:

```bash
# Scan locations to build manifests
bossa manifest scan ~/dev
bossa manifest scan ~/Library/Mobile\ Documents
bossa manifest scan /Volumes/External

# Then find duplicates
bossa storage duplicates
```

Output:

```
Duplicates Found: 15 files (2.3 GB total)

project-backup.zip (500 MB)
  ~/Downloads/project-backup.zip
  /Volumes/External/Backups/project-backup.zip

photo-2024.jpg (15 MB)
  ~/Pictures/photo-2024.jpg
  ~/Library/Mobile Documents/com~apple~CloudDocs/Photos/photo-2024.jpg
```

## Related Commands

### Cache Management

Move caches to external storage:

```bash
bossa caches status     # Show cache locations
bossa caches apply      # Apply cache config (create symlinks)
bossa caches audit      # Detect drift
bossa caches doctor     # Health checks
bossa caches init       # Create a starter config
```

### iCloud Management

Control iCloud Drive downloads:

```bash
bossa icloud status     # Show iCloud space usage
bossa icloud download   # Download files from cloud
bossa icloud evict      # Evict files to free space
```

### Manifest Operations

Build file manifests for duplicate detection:

```bash
bossa manifest scan <path>   # Hash files in directory
bossa manifest stats <path>  # Show manifest stats
bossa manifest duplicates <path>  # Find duplicates in manifest
```

## Configuration

### Cache Mappings

Define cache locations in `~/.config/bossa/caches.toml`:

```toml
external_drive = { name = "T9", mount_point = "/Volumes/T9", base_path = "caches" }

[[symlinks]]
name = "homebrew"
source = "~/Library/Caches/Homebrew"
target = "homebrew"

[[symlinks]]
name = "cargo"
source = "~/.cargo/registry"
target = "cargo/registry"
```

## Workflows

### External Drive Setup

1. Create cache directories:
   ```bash
   mkdir -p /Volumes/External/Caches/{Homebrew,cargo,npm}
   ```

2. Configure mappings in `caches.toml`

3. Move caches:
   ```bash
   bossa caches apply
   ```

4. Verify:
   ```bash
   bossa caches status
   ```

## Tips

1. **Scan regularly** - Keep manifests up to date for accurate duplicate detection
2. **Use external drives for caches** - Keep SSD space for active work
3. **Evict large iCloud folders** - Download on demand
4. **Check before deleting** - Review duplicates carefully before removal
