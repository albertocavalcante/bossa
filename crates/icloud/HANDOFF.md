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
| Unit tests           | ✅ Done | 6 tests passing                      |
| Documentation        | ✅ Done | Safety docs, README                  |

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

### Safety Guarantees

**CRITICAL**: This crate NEVER deletes files from iCloud. This is by design:

- `evict()` = remove LOCAL copy only, file stays in iCloud
- `download()` = fetch cloud copy to local (read-only)
- `status()` = read-only query

There are NO delete operations. The `brctl` tool itself has no delete functionality.

---

## TODO: Next Steps

### 1. Native FFI Backend (High Priority)

Add a native backend using Rust's `objc` crate to call `NSFileManager` directly instead of shelling out to `brctl`.

**Why:**

- Better performance (no process spawn overhead)
- Richer metadata access
- Better error information
- More reliable status detection

**Implementation:**

```rust
// crates/icloud/src/backend/native.rs
use objc::{class, msg_send, sel, sel_impl};
use objc::runtime::Object;

pub struct NativeBackend {
    file_manager: *mut Object,
}

impl NativeBackend {
    pub fn new() -> Result<Self> {
        unsafe {
            let fm: *mut Object = msg_send![class!(NSFileManager), defaultManager];
            Ok(Self { file_manager: fm })
        }
    }
}

impl Backend for NativeBackend {
    fn evict(&self, path: &Path) -> Result<()> {
        unsafe {
            let url = path_to_nsurl(path)?;
            let mut error: *mut Object = std::ptr::null_mut();
            let success: bool = msg_send![
                self.file_manager,
                evictUbiquitousItemAtURL: url
                error: &mut error
            ];
            if success {
                Ok(())
            } else {
                Err(nseerror_to_error(error))
            }
        }
    }

    // ... similar for download, status
}
```

**Dependencies to add:**

```toml
[dependencies]
objc = { version = "0.2", optional = true }
objc-foundation = { version = "0.1", optional = true }
block = { version = "0.1", optional = true }

[features]
default = ["brctl"]
brctl = []
native = ["objc", "objc-foundation", "block"]
```

**References:**

- Apple docs: https://developer.apple.com/documentation/foundation/filemanager/1409696-evictubiquitousitem
- objc crate: https://docs.rs/objc/latest/objc/

---

### 2. Better Status Detection (High Priority)

Current status detection is basic (checks file size and xattrs). Improve it to accurately detect:

- Cloud-only (evicted) files
- Files currently downloading
- Files currently uploading
- Upload/download progress percentage

**Implementation approach:**

Use `NSMetadataQuery` to get accurate iCloud status:

```rust
// Key attributes to query:
// - NSMetadataUbiquitousItemDownloadingStatusKey
// - NSMetadataUbiquitousItemIsDownloadingKey
// - NSMetadataUbiquitousItemPercentDownloadedKey
// - NSMetadataUbiquitousItemIsUploadingKey
// - NSMetadataUbiquitousItemPercentUploadedKey
```

Or use `mdls` command more comprehensively:

```bash
mdls -name kMDItemIsUbiquitous \
     -name kMDItemFSContentChangeDate \
     -name com_apple_metadata_kMDItemIsUploading \
     /path/to/file
```

---

### 3. Bossa CLI Integration (Medium Priority)

Add `bossa icloud` commands to expose the crate functionality.

**Commands to add:**

```bash
# Status and discovery
bossa icloud status [path]           # Show file status
bossa icloud list [path]             # List files with status
bossa icloud find-evictable [path]   # Find large local files

# Operations
bossa icloud evict <path>            # Evict file/folder
bossa icloud download <path>         # Download file/folder

# Storage overview
bossa storage status                 # Show local + T9 + iCloud usage
bossa storage audit                  # Find optimization opportunities
```

**Implementation location:** `src/commands/icloud.rs`

**CLI structure:**

```rust
#[derive(Subcommand)]
pub enum ICloudCommand {
    /// Show status of iCloud files
    Status {
        #[arg(default_value = "~")]
        path: String,
    },
    /// Evict files to free local space
    Evict {
        path: String,
        #[arg(long)]
        recursive: bool,
        #[arg(long)]
        min_size: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Download files from iCloud
    Download {
        path: String,
        #[arg(long)]
        recursive: bool,
    },
    /// Find large files that could be evicted
    FindEvictable {
        #[arg(default_value = "~/Library/Mobile Documents/com~apple~CloudDocs")]
        path: String,
        #[arg(long, default_value = "100MB")]
        min_size: String,
    },
}
```

---

### 4. Recursive Operations (Medium Priority)

Add proper recursive support for directories:

```rust
impl Client {
    /// Recursively evict all files in a directory
    pub fn evict_recursive(&self, path: &Path, options: &EvictOptions) -> Result<BulkResult> {
        let mut result = BulkResult::default();

        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                match self.evict(entry.path()) {
                    Ok(()) => result.add_success(/* size */),
                    Err(e) if e.is_already_done() => result.add_skip(),
                    Err(e) => result.add_failure(entry.path().to_path_buf(), e.to_string()),
                }
            }
        }

        Ok(result)
    }
}
```

**Dependencies:** Add `walkdir` to the crate.

---

### 5. Progress Tracking (Medium Priority)

Add progress callbacks for long-running operations:

```rust
pub trait ProgressCallback: Send + Sync {
    fn on_start(&self, total_files: usize, total_bytes: u64);
    fn on_file_start(&self, path: &Path, size: u64);
    fn on_file_complete(&self, path: &Path, success: bool);
    fn on_complete(&self, result: &BulkResult);
}

impl Client {
    pub fn evict_with_progress<P: ProgressCallback>(
        &self,
        paths: &[&Path],
        options: &EvictOptions,
        progress: &P,
    ) -> Result<BulkResult> {
        // ...
    }
}
```

---

### 6. Integration Tests (Medium Priority)

Add integration tests that work with real iCloud Drive:

```rust
// tests/integration.rs

#[test]
#[ignore] // Run with: cargo test -- --ignored
fn test_real_icloud_evict_and_download() {
    let client = Client::new().unwrap();
    let root = client.icloud_root().unwrap();

    // Create a test file
    let test_file = root.join("__icloud_crate_test__.txt");
    std::fs::write(&test_file, "test content").unwrap();

    // Wait for sync...
    std::thread::sleep(Duration::from_secs(5));

    // Test evict
    client.evict(&test_file).unwrap();

    // Verify status
    let status = client.status(&test_file).unwrap();
    assert!(status.state.is_cloud_only());

    // Test download
    client.download(&test_file).unwrap();

    // Cleanup
    std::fs::remove_file(&test_file).unwrap();
}
```

---

### 7. Katharsis Integration (Low Priority)

Consider adding iCloud support to Katharsis (Swift menu bar app) as discussed:

**Rationale:**

- Katharsis is already Swift (native API access)
- Has menu bar UI for status display
- Has FSEvents monitoring infrastructure
- Thematically aligned ("cleansing" = freeing space)

**If proceeding:**

- Add `icloud` module to Katharsis
- Reuse the same CLI command structure
- Add menu bar status for iCloud sync
- Add drag & drop eviction

---

### 8. Error Recovery (Low Priority)

Add retry logic for transient failures:

```rust
impl Client {
    pub fn evict_with_retry(
        &self,
        path: &Path,
        max_retries: usize,
        delay: Duration,
    ) -> Result<()> {
        let mut last_error = None;

        for attempt in 0..=max_retries {
            match self.evict(path) {
                Ok(()) => return Ok(()),
                Err(e) if e.is_transient() => {
                    last_error = Some(e);
                    if attempt < max_retries {
                        std::thread::sleep(delay);
                    }
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_error.unwrap())
    }
}
```

---

### 9. Configuration (Low Priority)

Add configuration support for default behaviors:

```toml
# ~/.config/bossa/config.toml

[icloud]
# Default minimum size for find-evictable
min_evictable_size = "100MB"

# Auto-evict files older than N days
auto_evict_after_days = 30

# Paths to never evict
protected_paths = [
  "~/Library/Mobile Documents/com~apple~CloudDocs/Important",
]
```

---

### 10. Publish to crates.io (Future)

When the API stabilizes:

1. Update `Cargo.toml` with full metadata
2. Add `LICENSE` file to crate directory
3. Add `CHANGELOG.md`
4. Publish: `cargo publish -p icloud`

Consider renaming to avoid conflicts:

- `icloud-drive`
- `macos-icloud`
- `icloud-rs`

---

## Testing Commands

```bash
# Run unit tests
cargo test -p icloud

# Run with Bazel
bazel test //crates/icloud:icloud_test

# Run examples
cargo run -p icloud --example status
cargo run -p icloud --example evict -- --large

# Build docs
cargo doc -p icloud --open
```

---

## Key Files to Understand

| File                   | Purpose                                                |
| ---------------------- | ------------------------------------------------------ |
| `src/lib.rs`           | Main `Client` API, safety documentation                |
| `src/backend/mod.rs`   | `Backend` trait definition                             |
| `src/backend/brctl.rs` | Current implementation using brctl CLI                 |
| `src/error.rs`         | Error types with `is_transient()`, `is_already_done()` |
| `src/types.rs`         | `FileStatus`, `DownloadState`, options structs         |

---

## Important Safety Notes

1. **NEVER add delete functionality** - This is intentional. The crate only manages local copies.

2. **Always verify iCloud path** - Check `is_in_icloud()` before operations.

3. **Handle "not synced" errors gracefully** - Files that are still uploading cannot be evicted.

4. **Test with real iCloud** - Mock tests are useful but real integration tests catch edge cases.

---

## Contact / Context

This crate was created as part of bossa's storage management features. The original discussion included:

- Checking T9 external drive usage
- Managing iCloud Drive space
- Finding duplicates across storage locations
- Potential integration with Katharsis (Swift app)

See conversation context for full background on design decisions.
