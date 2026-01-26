# iCloud Integration

Bossa provides tools for managing iCloud Drive storage, including downloading, evicting, and monitoring files.

## Commands

```bash
bossa icloud status     # Show iCloud space usage
bossa icloud download   # Download files from cloud
bossa icloud evict      # Evict files to free local space
bossa icloud list       # List files with their status
bossa icloud find-evictable  # Find large local files
```

## Understanding iCloud States

Files in iCloud Drive can be in different states:

| State           | Icon            | Description                              |
| --------------- | --------------- | ---------------------------------------- |
| **Local**       | (solid cloud)   | File is downloaded and available locally |
| **Cloud**       | (cloud outline) | File is in cloud only, not downloaded    |
| **Downloading** | (progress)      | File is being downloaded                 |
| **Uploading**   | (arrow up)      | Local changes being synced               |

## Status Overview

Check iCloud storage usage:

```bash
bossa icloud status
```

Output:

```
iCloud Drive Status
===================

Storage: 200 GB plan
  Used:      120 GB (60%)
  Available:  80 GB

Local Usage:
  Downloaded:  45 GB
  Cloud-only:  75 GB

By Folder:
  Documents/         15 GB (local)
  Photos/            50 GB (cloud)
  Developer/         30 GB (mixed)
  Archives/          25 GB (cloud)
```

## Downloading Files

Download files from iCloud to local storage:

```bash
# Download a specific folder
bossa icloud download ~/Library/Mobile\ Documents/com~apple~CloudDocs/Documents/

# Download a folder recursively
bossa icloud download ~/Library/Mobile\ Documents/ --recursive
```

## Evicting Files

Remove local copies to free space (files remain in iCloud):

```bash
# Evict a folder
bossa icloud evict ~/Library/Mobile\ Documents/com~apple~CloudDocs/Archives/

# Evict files larger than 100MB (recursive)
bossa icloud evict ~/Library/Mobile\ Documents/ --min-size 100MB --recursive

# Dry run
bossa icloud evict ~/Library/Mobile\ Documents/ --min-size 100MB --dry-run
```

## Finding Evictable Files

Find large local files that are safe to evict:

```bash
bossa icloud find-evictable ~/Library/Mobile\ Documents/ --min-size 100MB
```

## Listing Files

See file states:

```bash
bossa icloud list ~/Library/Mobile\ Documents/
```

Output:

```
Status  Size     Name
------  ----     ----
local   15 MB    Documents/report.pdf
cloud   500 MB   Archives/backup.zip
local   2 MB     Documents/notes.md
cloud   1.2 GB   Videos/recording.mov
```

### Filter Options

```bash
# Show only cloud files
bossa icloud list --cloud

# Show only local files
bossa icloud list --local
```

## iCloud Drive Paths

iCloud Drive files are stored in:

```
~/Library/Mobile Documents/
├── com~apple~CloudDocs/     # iCloud Drive root
├── com~apple~Numbers/       # Numbers documents
├── com~apple~Pages/         # Pages documents
└── [app-bundle-id]/         # Third-party app documents
```

### Common Paths

| Location          | Path                                                        |
| ----------------- | ----------------------------------------------------------- |
| iCloud Drive root | `~/Library/Mobile Documents/com~apple~CloudDocs/`           |
| Desktop           | `~/Library/Mobile Documents/com~apple~CloudDocs/Desktop/`   |
| Documents         | `~/Library/Mobile Documents/com~apple~CloudDocs/Documents/` |

## Workflows

### Free Local Space

1. Check what's using space:
   ```bash
   bossa icloud status
   ```

2. Find large local files:
   ```bash
   bossa icloud find-evictable --min-size 100MB
   ```

3. Evict old files:
   ```bash
   bossa icloud evict ~/Library/Mobile\ Documents/ --min-size 50MB --recursive
   ```

### Prepare for Offline

Download files you need offline:

```bash
# Download work folder
bossa icloud download ~/Library/Mobile\ Documents/com~apple~CloudDocs/Work/

# Verify
bossa icloud list ~/Library/Mobile\ Documents/com~apple~CloudDocs/Work/
```

### Clean Up After Project

Evict project files you no longer need locally:

```bash
bossa icloud evict ~/Library/Mobile\ Documents/com~apple~CloudDocs/Projects/old-project/
```

## Options

### Download Options

```bash
--recursive, -r    Download directories recursively
```

### Evict Options

```bash
--recursive, -r    Evict directories recursively
--min-size <SIZE>  Only evict files larger than SIZE
--dry-run          Preview what would be evicted
```

### List Options

```bash
--local            Show only local files
--cloud            Show only cloud-only files
```

## Tips

1. **Evict archives** - Old backups and archives are good eviction candidates
2. **Keep active work local** - Download folders you're actively using
3. **Check before evicting** - Use `--dry-run` first
4. **Monitor regularly** - Run `bossa icloud status` periodically
