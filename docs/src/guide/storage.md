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

Output:

```
Storage Overview
================

Local SSD (500 GB)
  Used:      350 GB (70%)
  Available: 150 GB

  Top directories:
    ~/Library           45 GB
    ~/dev               80 GB
    ~/Downloads         12 GB

iCloud Drive (200 GB)
  Used:      120 GB (60%)
  Available:  80 GB
  Local:      45 GB (downloaded)
  Cloud:      75 GB (evicted)

External: T9 (/Volumes/T9)
  Used:      1.2 TB
  Available: 800 GB

  Top directories:
    /Volumes/T9/Backups    500 GB
    /Volumes/T9/Media      400 GB
    /Volumes/T9/Caches     300 GB
```

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

### Free Up Local Space

1. Check storage status:
   ```bash
   bossa storage status
   ```

2. Evict iCloud files:
   ```bash
   bossa icloud evict ~/Library/Mobile\ Documents/large-folder/
   ```

3. Move caches to external:
   ```bash
   bossa caches apply
   ```

4. Find and remove duplicates:
   ```bash
   bossa storage duplicates
   ```

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
