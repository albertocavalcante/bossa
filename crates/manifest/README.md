# manifest

Content hashing and duplicate detection for storage volumes.

## Features

- **BLAKE3 hashing** - Fast, secure content hashing
- **SQLite storage** - Persistent manifest database
- **Duplicate detection** - Find files with identical content
- **Progress callbacks** - Track long-running scans
- **Prune missing** - Automatically remove stale entries

## Usage

```rust
use manifest::{Manifest, NoProgress};
use std::path::Path;

// Open or create a manifest database
let manifest = Manifest::open_for_root(Path::new("/path/to/manifest.db"), Path::new("/Volumes/MyDrive"))?;

// Scan a directory
let base_path = Path::new("/Volumes/MyDrive");
let result = manifest.scan(base_path, false, &mut NoProgress)?;
println!("Hashed {} files", result.hashed);

// Get statistics
let stats = manifest.stats()?;
println!("Files: {}", stats.file_count);
println!("Total size: {}", manifest::format_size(stats.total_size));
println!("Wasted space: {}", manifest::format_size(stats.duplicates.wasted_space));

// Find duplicates larger than 1MB
let duplicates = manifest.find_duplicates(1024 * 1024)?;
for group in duplicates {
    println!("{} copies of {} bytes", group.count, group.size_each);
    for path in &group.paths {
        println!("  {}", path);
    }
}
```

## Progress Tracking

Implement `ProgressCallback` for custom progress reporting:

```rust
use manifest::{ProgressCallback, ScanResult};
use std::path::Path;

struct MyProgress;

impl ProgressCallback for MyProgress {
    fn on_start(&mut self, total_files: u64, total_size: u64) {
        println!("Scanning {} files ({} bytes)", total_files, total_size);
    }

    fn on_file(&mut self, path: &Path, size: u64) {
        println!("  {}", path.display());
    }

    fn on_file_complete(&mut self, success: bool) {
        if !success {
            println!("  (failed)");
        }
    }

    fn on_complete(&mut self, result: &ScanResult) {
        println!("Done: {} hashed, {} errors", result.hashed, result.errors);
    }
}
```

## API

### `Manifest`

- `open(db_path)` - Read an existing manifest without creating it
- `open_for_root(db_path, root)` - Create or open a root-bound versioned store
- `open_existing(db_path, root)` - Read a root-bound store without requiring the root online
- `scan(base_path, force, progress)` - Scan directory and update manifest
- `stats()` - Get manifest statistics
- `find_duplicates(min_size)` - Find duplicate file groups
- `delete_duplicates(root, groups)` - Verify and delete duplicates, leaving inventory stale until a full rescan

### Types

- `ScanResult` - Results from a scan operation
- `ManifestStats` - Statistics about the manifest
- `DuplicateGroup` - A group of files with identical content
- `DuplicateStats` - Statistics about duplicates

### Utilities

- `path_to_name(path)` - Convert path to manifest name
- `format_size(bytes)` - Format bytes as human-readable size

## License

MIT

## Storage and compatibility

New scans use a versioned store bound to the declared root and observed canonical root. A failed scan keeps the prior completed generation. Non-forced scans may reuse hashes when size and timestamps match; this is not a filesystem snapshot. Use `force` to rehash.

Legacy unversioned databases are preserved. Read-only statistics can be viewed as historical and unqualified; duplicate paths and deletion require a fresh scan in a new versioned database. Read-only opens do not create or migrate files; they reject WAL-mode SQLite files because opening them can create sidecars. Concurrent external changes to journal mode between the header check and opening are outside this guarantee. A successful duplicate deletion marks the inventory stale until another complete scan. Deletion verifies current root identity, keeper and candidate content and containment; it cannot prevent adversarial replacement between checks and filesystem operations.

The public duplicate path API uses `NativePath`, serialized as platform-tagged Unix bytes or Windows UTF-16 units; this changes the former string-based Rust and JSON format. Display formatting escapes control characters and is not an address for deletion. Native duplicate deletion is currently supported on Unix only.
