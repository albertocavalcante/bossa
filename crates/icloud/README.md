# icloud

A Rust library for managing iCloud Drive files on macOS.

## Safety Guarantees

**This crate NEVER deletes files from iCloud.** All operations are non-destructive:

| Operation    | What it does                | What it does NOT do         |
| ------------ | --------------------------- | --------------------------- |
| **Evict**    | Removes LOCAL copy only     | Does NOT delete from iCloud |
| **Download** | Fetches cloud copy to local | Does NOT modify cloud copy  |
| **Status**   | Reads file state            | Read-only operation         |

There are intentionally **NO delete, remove, or destructive operations** in this crate.

## What is "eviction"?

When you evict a file:

1. The local copy is removed from your Mac's disk
2. The file **remains stored in iCloud** (cloud copy is untouched)
3. A placeholder appears in Finder with a download icon ☁️
4. The file will be re-downloaded automatically when you open it
5. You free up local disk space while keeping the file accessible

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
icloud = { path = "../icloud" } # or from crates.io when published
```

## Usage

```rust
use icloud::Client;

fn main() -> icloud::Result<()> {
    let client = Client::new()?;

    // Check file status
    let status = client.status("~/Library/Mobile Documents/com~apple~CloudDocs/myfile.txt")?;
    println!("State: {:?}", status.state);

    // Evict to free local space (file stays in iCloud!)
    if status.state.is_local() {
        client.evict(&status.path)?;
        println!("Evicted! File is now cloud-only.");
    }

    // Download when needed
    client.download(&status.path)?;

    Ok(())
}
```

## Finding evictable files

```rust
use icloud::Client;

let client = Client::new()?;
let root = client.icloud_root()?;

// Find files larger than 100MB that could be evicted
let large_files = client.find_evictable(&root, 100 * 1024 * 1024)?;

for file in large_files {
    println!("{}: {} bytes", file.path.display(), file.size.unwrap_or(0));
}

// Calculate total evictable size
let total = client.evictable_size(&root, 100 * 1024 * 1024)?;
println!("Could free {} bytes", total);
```

## Requirements

- macOS 10.15 or later
- iCloud Drive enabled and signed in
- "Optimize Mac Storage" should be enabled for best results
- Files must be fully synced before they can be evicted

## Backends

The crate supports multiple backends:

- `brctl` (default): Uses Apple's `brctl` CLI tool - safe and well-tested
- `native` (future): Direct FFI to `NSFileManager` for better performance

## Error Handling

```rust
use icloud::{Client, Error};

let client = Client::new()?;

match client.evict("~/iCloud/file.txt") {
    Ok(()) => println!("Evicted successfully"),
    Err(Error::AlreadyEvicted(_)) => println!("Already cloud-only"),
    Err(Error::NotSynced(_)) => println!("Still uploading, try again later"),
    Err(Error::NotInICloud(_)) => println!("Not in iCloud Drive"),
    Err(e) => println!("Error: {}", e),
}
```

## License

MIT
