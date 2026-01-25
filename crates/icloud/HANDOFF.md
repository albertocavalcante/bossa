# iCloud Crate - Handoff Document

This document describes the current state of the `icloud` crate and what should be done next.

## Current State

### What's Implemented

The `icloud` crate provides non-destructive iCloud Drive file management:

| Feature              | Status  | Notes                                |
| -------------------- | ------- | ------------------------------------ |
| Evict files          | ✅ Done | Removes local copy, keeps cloud copy |
| Download files       | ✅ Done | Fetches cloud copy to local          |
| Status query         | ✅ Done | Check if file is local/cloud/syncing |
| Find evictable files | ✅ Done | Find large local files               |
| brctl backend        | ✅ Done | Shells out to Apple's CLI            |
| Cargo build          | ✅ Done | Workspace crate                      |
| Bazel build          | ✅ Done | Full build support                   |
| Unit tests           | ✅ Done | 7 tests passing                      |
| Documentation        | ✅ Done | Safety docs, README                  |
| **CLI Integration**  | ✅ Done | `bossa icloud` commands              |
| **Recursive ops**    | ✅ Done | `-r` flag for directories            |
| **Progress bar**     | ✅ Done | Visual feedback for bulk operations  |
| **Block detection**  | ✅ Done | Reliable cloud-only detection        |

### Architecture

```
crates/icloud/
├── src/
│   ├── lib.rs              # Client API (high-level)
│   ├── error.rs            # Error types
│   ├── types.rs            # FileStatus, DownloadState, etc.
│   └── backend/
│       ├── mod.rs          # Backend trait
│       └── brctl.rs        # brctl CLI implementation
├── examples/
│   ├── status.rs           # List iCloud files
│   └── evict.rs            # Evict files
├── BUILD.bazel             # Bazel build
├── Cargo.toml              # Cargo manifest
└── README.md               # User documentation
```

### CLI Commands (in bossa)

```bash
bossa icloud status [path]              # Show file/directory summary
bossa icloud list [path]                # List files (--local, --cloud)
bossa icloud find-evictable [path]      # Find large files (-m/--min-size)
bossa icloud evict <path>               # Evict (-r, --min-size, --dry-run)
bossa icloud download <path>            # Download (-r)
```

### Safety Guarantees

**CRITICAL**: This crate NEVER deletes files from iCloud. This is by design:

- `evict()` = remove LOCAL copy only, file stays in iCloud
- `download()` = fetch cloud copy to local (read-only)
- `status()` = read-only query

There are NO delete operations. The `brctl` tool itself has no delete functionality.

---

## TODO: Next Steps

### 1. ~~Better Status Detection~~ ✅ DONE

Implemented block-based detection which is more reliable than mdls:

**What was done:**

- Cloud-only files have `size > 0` but `blocks == 0` (no local data allocated)
- Downloaded files have `blocks > 0` (actual data on disk)
- Falls back to xattr for edge cases (zero-size files, download in progress)
- Works regardless of Spotlight indexing status
- Added unit test for block detection logic

**Why block-based instead of mdls:**

- `mdls` requires Spotlight indexing which may be disabled/broken
- Block detection is a direct filesystem query - always works
- Faster (no shelling out to mdls)

---

### 2. ~~Storage Overview Command~~ ✅ DONE

Implemented `bossa storage status` command - unified view of all storage:

```
Storage Overview
────────────────

Local SSD
  Used: 220.8 GB / 228.3 GB (96%)
  Available: 7.4 GB

iCloud Drive
  Local: 0 B (0 files downloaded)
  Cloud-only: 3.8 GB (309 files evicted)

T9 External
  Status: Mounted ✓
  Used: 18.1 GB / 1.64 TB (1%)

Scanned Manifests
        1.8 GB     2768 files │ tmp │ 241 dups (104.6 MB)

Hints
  → Clean 104.6 MB in duplicates: bossa manifest duplicates <path>
```

**Features:**

- Local SSD space via `statvfs`
- iCloud summary (local/cloud-only/evictable)
- T9 external drive status and space
- Scanned manifest summaries with duplicate stats
- Actionable optimization hints with commands

---

### 3. Integration Tests (Medium Priority)

Add tests that work with real iCloud Drive:

```rust
// tests/integration.rs

#[test]
#[ignore] // Run with: cargo test -- --ignored
fn test_real_icloud_evict_and_download() {
    let client = Client::new().unwrap();
    let root = client.icloud_root().unwrap();

    let test_file = root.join("__icloud_crate_test__.txt");
    std::fs::write(&test_file, "test content").unwrap();

    // Wait for sync...
    std::thread::sleep(Duration::from_secs(5));

    client.evict(&test_file).unwrap();
    let status = client.status(&test_file).unwrap();
    assert!(status.state.is_cloud_only());

    client.download(&test_file).unwrap();
    std::fs::remove_file(&test_file).unwrap();
}
```

---

### 4. Native FFI Backend (Defer)

Add native backend using `objc` crate for `NSFileManager`.

**Defer because:**

- brctl backend works fine
- FFI adds complexity and unsafe code
- Only worth it if brctl becomes a bottleneck

**If needed later:**

```rust
// crates/icloud/src/backend/native.rs
impl Backend for NativeBackend {
    fn evict(&self, path: &Path) -> Result<()> {
        unsafe {
            let url = path_to_nsurl(path)?;
            let success: bool = msg_send![
                self.file_manager,
                evictUbiquitousItemAtURL: url
                error: &mut error
            ];
            // ...
        }
    }
}
```

---

### 5. Cross-Storage Duplicate Detection (Future)

Find duplicates across iCloud + T9 + local:

```bash
bossa storage duplicates

Cross-storage duplicates:
  file.pdf (100 MB)
    ★ /Volumes/T9/archive/file.pdf
    ✗ ~/Library/Mobile Documents/.../file.pdf (evictable)
```

---

### 6. Configuration (Low Priority, Deferred)

```toml
# ~/.config/bossa/config.toml

[icloud]
min_evictable_size = "100MB"
protected_paths = [
  "~/Library/Mobile Documents/com~apple~CloudDocs/Important",
]
```

---

### 7. Publish to crates.io (Future)

When API stabilizes:

1. Rename to `icloud-drive` or `macos-icloud`
2. Add LICENSE file
3. Add CHANGELOG.md
4. `cargo publish -p icloud`

---

## Testing Commands

```bash
# Run unit tests
cargo test -p icloud

# Run CLI
cargo run -- icloud status
cargo run -- icloud find-evictable -m 50MB
cargo run -- icloud evict ~/path/to/file --dry-run

# Build docs
cargo doc -p icloud --open
```

---

## Key Files

| File                                 | Purpose                              |
| ------------------------------------ | ------------------------------------ |
| `crates/icloud/src/lib.rs`           | Client API, safety docs              |
| `crates/icloud/src/backend/brctl.rs` | brctl CLI implementation             |
| `src/commands/icloud.rs`             | CLI command implementations          |
| `src/ui.rs`                          | Shared utilities (format_size, etc.) |

---

## Recent Changes

- **[pending]**: Added `bossa storage status` unified storage overview command
- **200999d**: Improved status detection using block allocation (replaces unreliable xattr/mdls approach)
- **b934adf**: Refactored CLI with shared utilities, progress bar, better error handling
- **21d2823**: Initial CLI integration with all commands
- **897e023**: Extracted manifest and declarative crates

---

## Recommendation

**Next step: Integration tests (#3) or Cross-storage duplicates (#5)**

The core iCloud functionality and storage overview are complete. Next priorities:

1. **Integration tests** - Real iCloud tests to catch edge cases
2. **Cross-storage duplicates** - Find files that exist on both iCloud and T9, allowing safe eviction
