//! Example: Evict a file from local storage
//!
//! Run with: cargo run -p icloud --example evict -- <path>

use icloud::Client;
use std::env;

fn main() -> icloud::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <path>", args[0]);
        eprintln!("       {} --large  (evict all files >100MB)", args[0]);
        std::process::exit(1);
    }

    let client = Client::new()?;

    if args[1] == "--large" {
        // Evict all large files
        let root = client.icloud_root()?;
        let evictable = client.find_evictable(&root, 100 * 1024 * 1024)?;

        println!("Found {} large local files:", evictable.len());
        for file in &evictable {
            println!(
                "  {} ({} bytes)",
                file.path.display(),
                file.size.unwrap_or(0)
            );
        }

        print!("\nEvict all? [y/N]: ");
        use std::io::Write;
        std::io::stdout().flush().ok();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();

        if input.trim().to_lowercase() == "y" {
            for file in evictable {
                print!("Evicting {}... ", file.path.display());
                match client.evict(&file.path) {
                    Ok(()) => println!("done"),
                    Err(e) => println!("error: {}", e),
                }
            }
        } else {
            println!("Aborted.");
        }
    } else {
        // Evict specific file
        let path = &args[1];

        // Check status first
        let status = client.status(path)?;
        println!("File: {}", status.path.display());
        println!("State: {:?}", status.state);
        println!("Size: {:?}", status.size);

        if status.state.is_cloud_only() {
            println!("\nFile is already evicted (cloud-only).");
            return Ok(());
        }

        println!("\nEvicting...");
        client.evict(path)?;
        println!("Done! File is now cloud-only.");
    }

    Ok(())
}
