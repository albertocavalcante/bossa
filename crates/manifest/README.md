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
let manifest = Manifest::open(Path::new("/path/to/manifest.db"))?;

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

- `open(db_path)` - Open or create a manifest database
- `scan(base_path, force, progress)` - Scan directory and update manifest
- `stats()` - Get manifest statistics
- `find_duplicates(min_size)` - Find duplicate file groups
- `delete_entry(path)` - Remove an entry from the manifest

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
